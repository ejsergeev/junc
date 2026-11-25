// SPDX-License-Identifier: GPL-3.0-or-later

//! Config edits behind the CLI commands. Pure functions on [`Config`]: the
//! caller reads the config, applies one of these, and sends the result back
//! with the revision it read, so concurrent changes are detected.

use junc_core::config::slugify;
use junc_core::model::{Cable, Config, DefaultRole, Input, Junction, Kind, Listen, Match};
use junc_ipc::ErrorBody;
use junc_ipc::view::JunctionView;
use junc_l10n::trf;

/// Result of a step that can fail with a user-facing error.
pub type Result<T> = std::result::Result<T, ErrorBody>;

pub fn usage(message: String, hint: &str) -> ErrorBody {
    ErrorBody::new("E_USAGE", message, Some(hint))
}

fn not_found(message: String, hint: String) -> ErrorBody {
    ErrorBody::new("E_NOT_FOUND", message, Some(&hint))
}

/// Edit distance, for "did you mean".
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != *cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

/// The closest candidate to `wanted`, if it is close enough to suggest.
fn suggest<'a>(wanted: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let w = wanted.to_lowercase();
    // A candidate the user typed the start of counts as close (`zoom` for
    // `zoom-mic`).
    candidates
        .into_iter()
        .map(|c| {
            let lc = c.to_lowercase();
            let d = if lc.starts_with(&w) {
                1
            } else {
                distance(&w, &lc)
            };
            (d, c)
        })
        .filter(|(d, _)| *d <= (w.chars().count() / 3).max(2))
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

pub fn did_you_mean<'a>(
    wanted: &str,
    candidates: impl IntoIterator<Item = &'a str>,
    list: &str,
) -> String {
    match suggest(wanted, candidates) {
        Some(s) => trf("Did you mean '{s}'?", &[("s", &s)]),
        None => list.to_owned(),
    }
}

/// Index of the Junction called `name`: its id, or its name ignoring case.
pub fn find(config: &Config, name: &str) -> Result<usize> {
    let js = &config.junction;
    if let Some(i) = js.iter().position(|j| j.id == name) {
        return Ok(i);
    }
    let by_name: Vec<usize> = js
        .iter()
        .enumerate()
        .filter(|(_, j)| j.name.to_lowercase() == name.to_lowercase())
        .map(|(i, _)| i)
        .collect();
    match by_name.as_slice() {
        [i] => Ok(*i),
        [] => Err(not_found(
            trf("No Junction called '{name}'.", &[("name", &name)]),
            did_you_mean(
                name,
                js.iter().flat_map(|j| [j.id.as_str(), j.name.as_str()]),
                "See them with `junc list`.",
            ),
        )),
        _ => Err(usage(
            trf("Several Junctions are called '{name}'.", &[("name", &name)]),
            "Use the id shown by `junc list`.",
        )),
    }
}

fn input_index(j: &Junction, id: &str) -> Result<usize> {
    j.input.iter().position(|i| i.id == id).ok_or_else(|| {
        not_found(
            trf("'{j}' has no Input '{id}'.", &[("j", &j.name), ("id", &id)]),
            did_you_mean(
                id,
                j.input.iter().map(|i| i.id.as_str()),
                "See the Inputs with `junc status`.",
            ),
        )
    })
}

fn listen_index(j: &Junction, id: &str) -> Result<usize> {
    j.listen.iter().position(|l| l.id == id).ok_or_else(|| {
        not_found(
            trf(
                "'{j}' has no Listen output '{id}'.",
                &[("j", &j.name), ("id", &id)],
            ),
            did_you_mean(
                id,
                j.listen.iter().map(|l| l.id.as_str()),
                "See the Listen outputs with `junc status`.",
            ),
        )
    })
}

/// An id from `text` that is not in `taken`: `slugify`, at most 63
/// characters, then `-2`, `-3`, … as needed.
pub fn unique_id<'a>(text: &str, taken: impl IntoIterator<Item = &'a str> + Clone) -> String {
    let mut base = slugify(text);
    base.truncate(56);
    let base = base.trim_end_matches('-').to_owned();
    (1..)
        .map(|n| {
            if n == 1 {
                base.clone()
            } else {
                format!("{base}-{n}")
            }
        })
        .find(|id| taken.clone().into_iter().all(|t| t != id))
        .unwrap_or(base)
}

fn junction(id: String, name: String) -> Junction {
    let mut t = toml::Table::new();
    t.insert("id".into(), id.into());
    t.insert("name".into(), name.into());
    // Deserializing fills in the model's defaults.
    toml::Value::Table(t)
        .try_into()
        .unwrap_or_else(|e| unreachable!("a minimal Junction always parses: {e}"))
}

fn app(binary: &str) -> Match {
    Match {
        binary: Some(binary.to_owned()),
        ..Match::default()
    }
}

/// Adds a Cable from one app to another; returns its id.
pub fn create_cable(config: &mut Config, from: &str, to: &str, name: Option<&str>) -> String {
    let name = name.map_or_else(
        || trf("{from} to {to}", &[("from", &from), ("to", &to)]),
        str::to_owned,
    );
    let id = unique_id(&name, config.junction.iter().map(|j| j.id.as_str()));
    let mut j = junction(id.clone(), name);
    j.kind = Kind::Cable;
    j.input.push(input(unique_id(from, []), app(from), false));
    j.cable = Some(Cable { to: app(to) });
    config.junction.push(j);
    id
}

/// Copies a Junction under a new name; the copy is not a default device.
pub fn duplicate(config: &mut Config, index: usize, name: Option<&str>) -> String {
    let mut copy = config.junction[index].clone();
    copy.name = name.map_or_else(
        || trf("{name} copy", &[("name", &copy.name)]),
        str::to_owned,
    );
    copy.id = unique_id(&copy.name, config.junction.iter().map(|j| j.id.as_str()));
    copy.default = DefaultRole::None;
    let id = copy.id.clone();
    config.junction.insert(index + 1, copy);
    id
}

fn input(id: String, matcher: Match, exclusive: bool) -> Input {
    let mut t = toml::Table::new();
    t.insert("id".into(), id.into());
    t.insert(
        "match".into(),
        toml::Value::try_from(&matcher).unwrap_or_else(|e| unreachable!("{e}")),
    );
    let mut i: Input = toml::Value::Table(t)
        .try_into()
        .unwrap_or_else(|e| unreachable!("a minimal Input always parses: {e}"));
    i.exclusive = exclusive;
    i
}

/// What an Input takes audio from.
#[derive(Debug, Clone)]
pub enum Source {
    App(String),
    Device(String),
    Monitor(String),
    Junction(String),
}

impl Source {
    fn matcher(&self) -> Match {
        let mut m = Match::default();
        match self {
            Self::App(b) => m.binary = Some(b.clone()),
            Self::Device(d) => m.device = Some(d.clone()),
            Self::Monitor(s) => m.monitor = Some(s.clone()),
            Self::Junction(j) => m.junction = Some(j.clone()),
        }
        m
    }

    fn label(&self) -> &str {
        match self {
            Self::App(s) | Self::Device(s) | Self::Monitor(s) | Self::Junction(s) => s,
        }
    }
}

/// Adds an Input; returns its id. A nested Junction may be named by id or
/// name; the config always stores its id.
pub fn input_add(
    config: &mut Config,
    index: usize,
    source: &Source,
    exclusive: bool,
    pro_audio: bool,
    id: Option<&str>,
) -> Result<String> {
    let source = match source {
        Source::Junction(name) => Source::Junction(config.junction[find(config, name)?].id.clone()),
        other => other.clone(),
    };
    let j = &mut config.junction[index];
    let id = match id {
        Some(id) if j.input.iter().any(|i| i.id == id) => {
            return Err(usage(
                trf(
                    "'{j}' already has an Input '{id}'.",
                    &[("j", &j.name), ("id", &id)],
                ),
                "Pick another --id.",
            ));
        }
        Some(id) => id.to_owned(),
        None => unique_id(source.label(), j.input.iter().map(|i| i.id.as_str())),
    };
    let mut matcher = source.matcher();
    if pro_audio {
        matcher.profile = Some(junc_core::proaudio::PRO_AUDIO.to_owned());
    }
    j.input.push(input(id.clone(), matcher, exclusive));
    Ok(id)
}

/// Removes an Input.
pub fn input_remove(config: &mut Config, index: usize, id: &str) -> Result<()> {
    let j = &mut config.junction[index];
    let i = input_index(j, id)?;
    j.input.remove(i);
    Ok(())
}

/// Turns an Input on or off.
pub fn input_enable(config: &mut Config, index: usize, id: &str, on: bool) -> Result<()> {
    let j = &mut config.junction[index];
    let i = input_index(j, id)?;
    j.input[i].enabled = on;
    Ok(())
}

/// Sets an Input's volume.
pub fn input_volume(config: &mut Config, index: usize, id: &str, percent: u16) -> Result<()> {
    let j = &mut config.junction[index];
    let i = input_index(j, id)?;
    j.input[i].volume = percent;
    Ok(())
}

/// Sets a Listen output's volume.
pub fn listen_volume(config: &mut Config, index: usize, id: &str, percent: u16) -> Result<()> {
    let j = &mut config.junction[index];
    let i = listen_index(j, id)?;
    j.listen[i].volume = percent;
    Ok(())
}

/// Adds a Listen output to `target` (`default` or an output node name);
/// returns its id.
pub fn listen_add(
    config: &mut Config,
    index: usize,
    target: &str,
    id: Option<&str>,
) -> Result<String> {
    let j = &mut config.junction[index];
    let id = match id {
        Some(id) if j.listen.iter().any(|l| l.id == id) => {
            return Err(usage(
                trf(
                    "'{j}' already has a Listen output '{id}'.",
                    &[("j", &j.name), ("id", &id)],
                ),
                "Pick another --id.",
            ));
        }
        Some(id) => id.to_owned(),
        None => unique_id(target, j.listen.iter().map(|l| l.id.as_str())),
    };
    let mut t = toml::Table::new();
    t.insert("id".into(), id.clone().into());
    t.insert("target".into(), target.into());
    let listen: Listen = toml::Value::Table(t)
        .try_into()
        .unwrap_or_else(|e| unreachable!("a minimal Listen always parses: {e}"));
    j.listen.push(listen);
    Ok(id)
}

/// Removes a Listen output.
pub fn listen_remove(config: &mut Config, index: usize, id: &str) -> Result<()> {
    let j = &mut config.junction[index];
    let i = listen_index(j, id)?;
    j.listen.remove(i);
    Ok(())
}

/// Adds `count` channels to the Bus.
pub fn bus_add(config: &mut Config, index: usize, count: u8) {
    let bus = &mut config.junction[index].bus;
    bus.channels = bus.channels.saturating_add(count);
}

/// Removes Bus channel `ch` (1-based). Links to later channels move down
/// one so they keep pointing at the same audio.
pub fn bus_remove(config: &mut Config, index: usize, ch: u8) -> Result<()> {
    let j = &mut config.junction[index];
    if ch == 0 || ch > j.bus.channels {
        return Err(usage(
            trf(
                "'{j}' has channels 1 to {n}; there is no channel {ch}.",
                &[("j", &j.name), ("n", &j.bus.channels), ("ch", &ch)],
            ),
            "See the Bus with `junc status`.",
        ));
    }
    if j.bus.channels == 1 {
        return Err(usage(
            trf("'{j}' needs at least one Bus channel.", &[("j", &j.name)]),
            "Delete the Junction instead: `junc delete`.",
        ));
    }
    j.bus.channels -= 1;
    // Input pairs are [input ch, bus ch]; Listen pairs are [bus ch, out ch].
    let shift = |pairs: &mut Vec<[u8; 2]>, side: usize| {
        pairs.retain(|p| p[side] != ch);
        for p in pairs.iter_mut() {
            if p[side] > ch {
                p[side] -= 1;
            }
        }
    };
    for i in &mut j.input {
        if let Some(l) = &mut i.links {
            shift(l, 1);
        }
        shift(&mut i.suppressed, 1);
    }
    for l in &mut j.listen {
        if let Some(links) = &mut l.links {
            shift(links, 0);
        }
        shift(&mut l.suppressed, 0);
    }
    Ok(())
}

/// One end of a link: a Bus channel, or a channel of an Input or Listen
/// output (`mic:1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    Bus(u8),
    Entry(String, u8),
}

fn channel(text: &str, hint: &str) -> Result<u8> {
    text.parse::<u8>()
        .ok()
        .filter(|c| *c >= 1)
        .ok_or_else(|| usage(trf("'{t}' is not a channel number.", &[("t", &text)]), hint))
}

impl std::str::FromStr for End {
    type Err = ErrorBody;
    fn from_str(s: &str) -> Result<Self> {
        match s.rsplit_once(':') {
            Some((id, ch)) => Ok(Self::Entry(
                id.to_owned(),
                channel(ch, "Channels count from 1.")?,
            )),
            None => Ok(Self::Bus(channel(s, LINK_HINT)?)),
        }
    }
}

const LINK_HINT: &str = "Link an Input channel to a Bus channel (`junc link NAME mic:1 2`) \
or a Bus channel to a Listen output channel (`junc link NAME 1 speakers:2`).";

/// Which list a link belongs to and the pair in it.
enum Target {
    Input(usize, [u8; 2]),
    Listen(usize, [u8; 2]),
}

fn target(j: &Junction, from: &End, to: &End) -> Result<Target> {
    match (from, to) {
        (End::Entry(id, c), End::Bus(b)) => Ok(Target::Input(input_index(j, id)?, [*c, *b])),
        (End::Bus(b), End::Entry(id, c)) => Ok(Target::Listen(listen_index(j, id)?, [*b, *c])),
        _ => Err(usage(
            "A link goes from an Input to the Bus, or from the Bus to a Listen output.".into(),
            LINK_HINT,
        )),
    }
}

/// The pairs to edit: the config's own list, or the mapping in use (from the
/// daemon's view) turned into an explicit list.
fn pairs<'a>(links: &'a mut Option<Vec<[u8; 2]>>, in_use: &[[u8; 2]]) -> &'a mut Vec<[u8; 2]> {
    links.get_or_insert_with(|| in_use.to_vec())
}

/// Adds a link. Returns false if it was already there.
pub fn link(
    config: &mut Config,
    index: usize,
    view: Option<&JunctionView>,
    from: &End,
    to: &End,
) -> Result<bool> {
    let j = &mut config.junction[index];
    for end in [from, to] {
        if let End::Bus(b) = end
            && *b > j.bus.channels
        {
            return Err(bus_range(j, *b));
        }
    }
    let added = match target(j, from, to)? {
        Target::Input(i, pair) => {
            let in_use = view
                .and_then(|v| v.inputs.get(i))
                .map(|v| v.links.clone())
                .unwrap_or_default();
            let input = &mut j.input[i];
            input.suppressed.retain(|p| *p != pair);
            add_pair(pairs(&mut input.links, &in_use), pair)
        }
        Target::Listen(i, pair) => {
            let in_use = view
                .and_then(|v| v.listens.get(i))
                .map(|v| v.links.clone())
                .unwrap_or_default();
            let listen = &mut j.listen[i];
            listen.suppressed.retain(|p| *p != pair);
            add_pair(pairs(&mut listen.links, &in_use), pair)
        }
    };
    Ok(added)
}

fn add_pair(list: &mut Vec<[u8; 2]>, pair: [u8; 2]) -> bool {
    if list.contains(&pair) {
        return false;
    }
    list.push(pair);
    list.sort_unstable();
    true
}

fn bus_range(j: &Junction, b: u8) -> ErrorBody {
    usage(
        trf(
            "'{j}' has Bus channels 1 to {n}; there is no channel {b}.",
            &[("j", &j.name), ("n", &j.bus.channels), ("b", &b)],
        ),
        "Add channels with `junc bus add`.",
    )
}

/// Removes a link.
pub fn unlink(
    config: &mut Config,
    index: usize,
    view: Option<&JunctionView>,
    from: &End,
    to: &End,
) -> Result<()> {
    let j = &mut config.junction[index];
    let (list, pair) = match target(j, from, to)? {
        Target::Input(i, pair) => {
            let in_use = view
                .and_then(|v| v.inputs.get(i))
                .map(|v| v.links.clone())
                .unwrap_or_default();
            (pairs(&mut j.input[i].links, &in_use), pair)
        }
        Target::Listen(i, pair) => {
            let in_use = view
                .and_then(|v| v.listens.get(i))
                .map(|v| v.links.clone())
                .unwrap_or_default();
            (pairs(&mut j.listen[i].links, &in_use), pair)
        }
    };
    let before = list.len();
    list.retain(|p| *p != pair);
    if list.len() == before {
        return Err(not_found(
            trf(
                "There is no link {a} → {b}.",
                &[("a", &pair[0]), ("b", &pair[1])],
            ),
            "See the links with `junc status --json`.".to_owned(),
        ));
    }
    Ok(())
}

/// Adds a Junction made from a preset or template (F22, F24), named `name`
/// or as the preset says, with an id made unique from the name. Returns
/// its id and notes for the user: a default role another Junction already
/// has is dropped.
pub fn add_junction(
    config: &mut Config,
    mut j: Junction,
    name: Option<&str>,
) -> (String, Vec<String>) {
    if let Some(n) = name {
        n.clone_into(&mut j.name);
    }
    j.id = unique_id(&j.name, config.junction.iter().map(|j| j.id.as_str()));
    let mut notes = Vec::new();
    if j.default != DefaultRole::None
        && let Some(other) = config.junction.iter().find(|o| o.default == j.default)
    {
        let role = match j.default {
            DefaultRole::Output => "output",
            _ => "input",
        };
        notes.push(trf(
            "{other} stays the default {role}; make this one the default with: junc switch {id}",
            &[("other", &other.name), ("role", &role), ("id", &j.id)],
        ));
        j.default = DefaultRole::None;
    }
    let id = j.id.clone();
    config.junction.push(j);
    (id, notes)
}

/// Adds a Junction or Cable made from an existing setup (F13); returns its
/// id. The setup's apps, devices and monitors become Inputs and the
/// outputs it plays to become Listen outputs; a Cable needs one app into
/// another.
pub fn import(
    config: &mut Config,
    item: &junc_core::import::Importable,
    as_cable: bool,
) -> Result<String> {
    let id = unique_id(&item.name, config.junction.iter().map(|j| j.id.as_str()));
    let mut j = junction(id.clone(), item.name.clone());
    if as_cable {
        let Some(c) = &item.cable else {
            return Err(usage(
                trf(
                    "'{id}' is not one app into another, so it cannot be a Cable.",
                    &[("id", &item.id)],
                ),
                "Import it as a Junction: --as junction",
            ));
        };
        j.kind = Kind::Cable;
        let label = c
            .from
            .binary
            .as_deref()
            .or(c.from.app.as_deref())
            .unwrap_or("app");
        j.input
            .push(input(unique_id(label, []), c.from.clone(), false));
        j.cable = Some(Cable { to: c.to.clone() });
    } else {
        j.bus.channels = item.channels.clamp(1, junc_core::model::MAX_BUS_CHANNELS);
        for m in &item.inputs {
            let label = m
                .binary
                .as_deref()
                .or(m.app.as_deref())
                .or(m.device.as_deref())
                .or(m.monitor.as_deref())
                .unwrap_or("input");
            let iid = unique_id(label, j.input.iter().map(|i| i.id.as_str()));
            j.input.push(input(iid, m.clone(), false));
        }
        config.junction.push(j);
        let index = config.junction.len() - 1;
        for target in &item.listens {
            listen_add(config, index, target, None)?;
        }
        return Ok(id);
    }
    config.junction.push(j);
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use junc_core::config;

    const CONFIG: &str = r#"schema = 1
[[junction]]
id = "zoom"
name = "Zoom Mic"
bus = { channels = 4 }
[[junction.input]]
id = "mic"
match = { device = "alsa_input.usb-mic" }
links = [[1, 1], [1, 2], [1, 3]]
[[junction.listen]]
id = "hp"
target = "default"
links = [[1, 1], [4, 2]]
[[junction]]
id = "obs"
name = "OBS Mix"
"#;

    fn cfg() -> Config {
        config::load(CONFIG, "junctions.toml")
            .expect("config")
            .config
    }

    fn valid(c: &Config) {
        let text = config::to_toml(c);
        if let Err(d) = config::load(&text, "out.toml") {
            panic!("{d:?}\n{text}");
        }
    }

    #[test]
    fn names_resolve_by_id_or_name_with_suggestions() {
        let c = cfg();
        assert_eq!(find(&c, "obs").ok(), Some(1));
        assert_eq!(find(&c, "zoom mic").ok(), Some(0));
        let e = find(&c, "zom").expect_err("unknown");
        assert_eq!(e.code, "E_NOT_FOUND");
        assert_eq!(e.hint.as_deref(), Some("Did you mean 'zoom'?"));
        let e = find(&c, "zoom m").expect_err("prefix");
        assert_eq!(e.hint.as_deref(), Some("Did you mean 'Zoom Mic'?"));
        let e = find(&c, "something else").expect_err("unknown");
        assert_eq!(e.hint.as_deref(), Some("See them with `junc list`."));
    }

    #[test]
    fn ids_are_unique_and_valid() {
        assert_eq!(unique_id("Zoom Mic", ["zoom-mic"]), "zoom-mic-2");
        assert_eq!(
            unique_id("alsa_input.usb-Blue_Yeti", []),
            "alsa-input-usb-blue-yeti"
        );
        let long = "x".repeat(100);
        assert!(unique_id(&long, []).len() <= 63);
    }

    #[test]
    fn cables_and_duplicates_validate() {
        let mut c = cfg();
        let id = create_cable(&mut c, "spotify", "obs", None);
        assert_eq!(id, "spotify-to-obs");
        let copy = duplicate(&mut c, 0, None);
        assert_eq!(copy, "zoom-mic-copy");
        assert_eq!(c.junction[1].name, "Zoom Mic copy");
        valid(&c);
    }

    #[test]
    fn inputs_and_listens_are_added_and_removed() {
        let mut c = cfg();
        assert_eq!(
            input_add(&mut c, 1, &Source::App("firefox".into()), true, false, None)
                .ok()
                .as_deref(),
            Some("firefox")
        );
        assert_eq!(
            input_add(
                &mut c,
                1,
                &Source::Junction("Zoom Mic".into()),
                false,
                false,
                None
            )
            .ok()
            .as_deref(),
            Some("zoom")
        );
        assert_eq!(
            c.junction[1].input[1].matcher.junction.as_deref(),
            Some("zoom")
        );
        assert!(
            input_add(
                &mut c,
                1,
                &Source::App("x".into()),
                false,
                false,
                Some("firefox")
            )
            .is_err()
        );
        assert_eq!(
            listen_add(&mut c, 1, "default", None).ok().as_deref(),
            Some("default")
        );
        valid(&c);
        input_enable(&mut c, 1, "firefox", false).expect("disable");
        assert!(!c.junction[1].input[0].enabled);
        input_remove(&mut c, 1, "firefox").expect("remove");
        listen_remove(&mut c, 1, "default").expect("remove");
        assert_eq!(
            input_remove(&mut c, 1, "nope").expect_err("missing").code,
            "E_NOT_FOUND"
        );
        valid(&c);
    }

    #[test]
    fn removing_a_bus_channel_shifts_later_links() {
        let mut c = cfg();
        bus_remove(&mut c, 0, 2).expect("remove");
        let j = &c.junction[0];
        assert_eq!(j.bus.channels, 3);
        assert_eq!(j.input[0].links.as_deref(), Some(&[[1, 1], [1, 2]][..]));
        assert_eq!(j.listen[0].links.as_deref(), Some(&[[1, 1], [3, 2]][..]));
        assert!(bus_remove(&mut c, 0, 9).is_err());
        bus_add(&mut c, 0, 2);
        assert_eq!(c.junction[0].bus.channels, 5);
        valid(&c);
    }

    #[test]
    fn links_are_added_and_removed_from_the_mapping_in_use() {
        let mut c = cfg();
        let from: End = "mic:1".parse().expect("end");
        let to: End = "4".parse().expect("end");
        assert!(link(&mut c, 0, None, &from, &to).expect("link"));
        assert!(
            !link(&mut c, 0, None, &from, &to).expect("again"),
            "already there"
        );
        unlink(&mut c, 0, None, &from, &"1".parse().expect("end")).expect("unlink");
        assert_eq!(
            c.junction[0].input[0].links.as_deref(),
            Some(&[[1, 2], [1, 3], [1, 4]][..])
        );
        assert!(
            link(&mut c, 0, None, &from, &"9".parse().expect("end")).is_err(),
            "no bus ch 9"
        );
        assert!(
            link(
                &mut c,
                0,
                None,
                &"1".parse().expect("end"),
                &"2".parse().expect("end")
            )
            .is_err()
        );
        assert_eq!("mic:0".parse::<End>().expect_err("ch 0").code, "E_USAGE");

        // An Input on the default mapping: the mapping in use becomes explicit.
        let mut c = cfg();
        c.junction[0].input[0].links = None;
        let mut view = junc_ipc::view::junction(&c.junction[0], &[], None);
        view.inputs[0].links = vec![[1, 1], [1, 2]];
        link(&mut c, 0, Some(&view), &from, &"3".parse().expect("end")).expect("link");
        assert_eq!(
            c.junction[0].input[0].links.as_deref(),
            Some(&[[1, 1], [1, 2], [1, 3]][..])
        );
        valid(&c);
    }
}
