// SPDX-License-Identifier: GPL-3.0-or-later

//! What the Main screen changes in the config (F4, F7, F8): pure functions
//! on [`Config`]. The caller reads the config, applies one, and sends it
//! back with the revision it read (`GetConfig` → `Apply`), as the CLI
//! does, so the file keeps its comments and concurrent edits are caught.
//! Junction-level changes (on/off, master volume, delete) have their own
//! D-Bus methods and do not come through here.

use junc_core::config::slugify;
use junc_core::model::{Config, DefaultRole, Input, Listen, MAX_BUS_CHANNELS, Match};

/// An Input or a Listen output of a Junction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Input(String),
    Listen(String),
}

/// One change.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// Turn it on or off.
    Enabled(Part, bool),
    /// Its volume in percent.
    Volume(Part, u16),
    /// Remove these channel pairs, given the pairs in use now (the default
    /// mapping when the config lists none): the rest become the config's
    /// own list.
    Unlink {
        part: Part,
        remove: Vec<[u8; 2]>,
        in_use: Vec<[u8; 2]>,
    },
    /// Add a channel pair (boards 13, 14), given the pairs in use now; it
    /// is no longer one another tool removed.
    Link {
        part: Part,
        pair: [u8; 2],
        in_use: Vec<[u8; 2]>,
    },
    /// Add an Input taking `matcher`; its id comes from `label`.
    AddInput { matcher: Box<Match>, label: String },
    /// Add a Listen output to `target` (`default` or an output node.name);
    /// its id comes from `label`.
    AddListen { target: String, label: String },
    /// Add this many channels to the Bus (board 16: a pair or one).
    AddBus(u8),
    /// Input Options (board 15): mute the app elsewhere while captured.
    Exclusive { input: String, on: bool },
    /// Input Options: Input channel `ch` feeds Bus channel `bus` alone
    /// (`None`: nothing), given the pairs in use now.
    Feed {
        input: String,
        ch: u8,
        bus: Option<u8>,
        in_use: Vec<[u8; 2]>,
    },
    /// Input Options: remove the Input.
    RemoveInput(String),
    /// A Cable (board 12) takes audio from another app: its one Input
    /// matches `0`.
    CableFrom(Box<Match>),
    /// A Cable delivers to another app.
    CableTo(Box<Match>),
    /// A Listen output goes to another target.
    ListenTarget { listen: String, target: String },
    /// A copy of the Junction named `0` after it (board 21), its id from
    /// the name; it is no system default.
    Duplicate(String),
    /// A Junction a template made (board 49), after the others; its id is
    /// made unique.
    AddJunction(Box<junc_core::model::Junction>),
    /// A new Cable (board 27) named `name` from app `from` (its Input's id
    /// from `label`) to app `to`, after the others.
    NewCable {
        name: String,
        from: Box<Match>,
        label: String,
        to: Box<Match>,
    },
}

/// Applies `edit` to Junction `junction`. Returns false when it or the
/// part is gone (someone else changed the config meanwhile).
pub fn apply(config: &mut Config, junction: &str, edit: &Edit) -> bool {
    if let Edit::AddJunction(j) = edit {
        let mut j = (**j).clone();
        j.id = unique_id(&j.id, config.junction.iter().map(|k| k.id.as_str()));
        config.junction.push(j);
        return true;
    }
    if let Edit::NewCable {
        name,
        from,
        label,
        to,
    } = edit
    {
        let mut t = toml::Table::new();
        let id = unique_id(name, config.junction.iter().map(|j| j.id.as_str()));
        t.insert("id".into(), id.into());
        t.insert("name".into(), name.clone().into());
        t.insert("kind".into(), "cable".into());
        let mut cable = toml::Table::new();
        let Ok(to) = toml::Value::try_from(&**to) else {
            return false;
        };
        cable.insert("to".into(), to);
        t.insert("cable".into(), cable.into());
        let Ok(mut j) = toml::Value::Table(t).try_into::<junc_core::model::Junction>() else {
            return false;
        };
        j.input.push(new_input(
            unique_id(label, std::iter::empty()),
            (**from).clone(),
        ));
        config.junction.push(j);
        return true;
    }
    if let Edit::Duplicate(name) = edit {
        let Some(i) = config.junction.iter().position(|j| j.id == junction) else {
            return false;
        };
        let mut copy = config.junction[i].clone();
        copy.id = unique_id(name, config.junction.iter().map(|j| j.id.as_str()));
        copy.name.clone_from(name);
        copy.default = DefaultRole::None;
        config.junction.insert(i + 1, copy);
        return true;
    }
    let Some(j) = config.junction.iter_mut().find(|j| j.id == junction) else {
        return false;
    };
    let part = match edit {
        Edit::Enabled(p, _)
        | Edit::Volume(p, _)
        | Edit::Unlink { part: p, .. }
        | Edit::Link { part: p, .. } => p,
        Edit::AddInput { matcher, label } => {
            let id = unique_id(label, j.input.iter().map(|i| i.id.as_str()));
            j.input.push(new_input(id, (**matcher).clone()));
            return true;
        }
        Edit::AddBus(n) => {
            let wanted = j.bus.channels.saturating_add(*n);
            if wanted > MAX_BUS_CHANNELS {
                return false;
            }
            j.bus.channels = wanted;
            return true;
        }
        Edit::AddListen { target, label } => {
            let id = unique_id(label, j.listen.iter().map(|l| l.id.as_str()));
            j.listen.push(new_listen(id, target));
            return true;
        }
        Edit::CableFrom(m) => {
            if j.cable.is_none() || j.input.len() != 1 {
                return false;
            }
            j.input[0].matcher = (**m).clone();
            return true;
        }
        Edit::CableTo(m) => {
            let Some(c) = j.cable.as_mut() else {
                return false;
            };
            c.to = (**m).clone();
            return true;
        }
        Edit::ListenTarget { listen, target } => {
            let Some(l) = j.listen.iter_mut().find(|l| &l.id == listen) else {
                return false;
            };
            l.target.clone_from(target);
            l.links = None;
            return true;
        }
        // Done above: they add a Junction rather than change one.
        Edit::Duplicate(_) | Edit::NewCable { .. } | Edit::AddJunction(_) => return true,
        Edit::RemoveInput(id) => {
            let before = j.input.len();
            j.input.retain(|i| &i.id != id);
            return j.input.len() < before;
        }
        Edit::Exclusive { input, on } => {
            let Some(i) = j.input.iter_mut().find(|i| &i.id == input) else {
                return false;
            };
            i.exclusive = *on;
            return true;
        }
        Edit::Feed {
            input,
            ch,
            bus,
            in_use,
        } => {
            if bus.is_some_and(|b| b == 0 || b > j.bus.channels) {
                return false;
            }
            let Some(i) = j.input.iter_mut().find(|i| &i.id == input) else {
                return false;
            };
            let mut links: Vec<[u8; 2]> = i
                .links
                .as_deref()
                .unwrap_or(in_use)
                .iter()
                .filter(|p| p[0] != *ch)
                .copied()
                .collect();
            links.extend(bus.map(|b| [*ch, b]));
            links.sort_unstable();
            i.links = Some(links);
            return true;
        }
    };
    match part {
        Part::Input(id) => {
            let Some(i) = j.input.iter_mut().find(|i| &i.id == id) else {
                return false;
            };
            match edit {
                Edit::Enabled(_, on) => i.enabled = *on,
                Edit::Volume(_, v) => i.volume = *v,
                Edit::Unlink { remove, in_use, .. } => {
                    i.links = Some(remaining(i.links.as_deref(), in_use, remove));
                }
                Edit::Link { pair, in_use, .. } => {
                    i.links = Some(with(i.links.as_deref(), in_use, *pair));
                    i.suppressed.retain(|p| p != pair);
                }
                _ => {}
            }
        }
        Part::Listen(id) => {
            let Some(l) = j.listen.iter_mut().find(|l| &l.id == id) else {
                return false;
            };
            match edit {
                Edit::Enabled(_, on) => l.enabled = *on,
                Edit::Volume(_, v) => l.volume = *v,
                Edit::Unlink { remove, in_use, .. } => {
                    l.links = Some(remaining(l.links.as_deref(), in_use, remove));
                }
                Edit::Link { pair, in_use, .. } => {
                    l.links = Some(with(l.links.as_deref(), in_use, *pair));
                    l.suppressed.retain(|p| p != pair);
                }
                _ => {}
            }
        }
    }
    true
}

/// An id from `label` that no other Input (or Listen output) of the
/// Junction has: the slug, then `-2`, `-3`, … (as the CLI makes them).
fn unique_id<'a>(label: &str, taken: impl Iterator<Item = &'a str> + Clone) -> String {
    let mut base = slugify(label);
    base.truncate(56);
    let base = match base.trim_end_matches('-') {
        "" => "input".to_owned(),
        b => b.to_owned(),
    };
    (1..)
        .map(|n| {
            if n == 1 {
                base.clone()
            } else {
                format!("{base}-{n}")
            }
        })
        .find(|id| taken.clone().all(|t| t != id))
        .unwrap_or(base)
}

/// A new Input with the model's defaults (on, 100 %, default links).
fn new_input(id: String, matcher: Match) -> Input {
    let mut t = toml::Table::new();
    t.insert("id".into(), id.into());
    t.insert(
        "match".into(),
        toml::Value::try_from(&matcher).unwrap_or_else(|e| unreachable!("{e}")),
    );
    toml::Value::Table(t)
        .try_into()
        .unwrap_or_else(|e| unreachable!("a minimal Input always parses: {e}"))
}

/// A Listen output to `target` with everything else at its default.
fn new_listen(id: String, target: &str) -> Listen {
    let mut t = toml::Table::new();
    t.insert("id".into(), id.into());
    t.insert("target".into(), target.into());
    toml::Value::Table(t)
        .try_into()
        .unwrap_or_else(|e| unreachable!("a minimal Listen always parses: {e}"))
}

fn with(own: Option<&[[u8; 2]]>, in_use: &[[u8; 2]], pair: [u8; 2]) -> Vec<[u8; 2]> {
    let mut out = own.unwrap_or(in_use).to_vec();
    if !out.contains(&pair) {
        out.push(pair);
        out.sort_unstable();
    }
    out
}

fn remaining(own: Option<&[[u8; 2]]>, in_use: &[[u8; 2]], remove: &[[u8; 2]]) -> Vec<[u8; 2]> {
    own.unwrap_or(in_use)
        .iter()
        .filter(|p| !remove.contains(p))
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"schema = 1
[[junction]]
id = "zoom"
name = "Zoom"
bus = { channels = 4 }
[[junction.input]]
id = "mic"
match = { device = "usb-mic" }
[[junction.input]]
id = "music"
match = { binary = "spotify" }
links = [[1, 3], [2, 4]]
[[junction.listen]]
id = "hp"
target = "default"
"#;

    fn config() -> Config {
        junc_core::config::load(CONFIG, "junctions.toml")
            .expect("config")
            .config
    }

    #[test]
    fn toggles_and_volumes_reach_the_right_part() {
        let mut c = config();
        assert!(apply(
            &mut c,
            "zoom",
            &Edit::Enabled(Part::Input("mic".into()), false)
        ));
        assert!(apply(
            &mut c,
            "zoom",
            &Edit::Volume(Part::Listen("hp".into()), 80)
        ));
        assert!(!c.junction[0].input[0].enabled);
        assert!(c.junction[0].input[1].enabled);
        assert_eq!(c.junction[0].listen[0].volume, 80);
        assert!(!apply(
            &mut c,
            "zoom",
            &Edit::Enabled(Part::Input("gone".into()), true)
        ));
        assert!(!apply(
            &mut c,
            "other",
            &Edit::Volume(Part::Listen("hp".into()), 1)
        ));
    }

    #[test]
    fn unlinking_keeps_the_other_pairs() {
        let mut c = config();
        // Default mapping in use: it becomes the Input's own list, less one.
        let mic = Part::Input("mic".into());
        assert!(apply(
            &mut c,
            "zoom",
            &Edit::Unlink {
                part: mic,
                remove: vec![[1, 1]],
                in_use: vec![[1, 1], [2, 2]],
            }
        ));
        assert_eq!(c.junction[0].input[0].links, Some(vec![[2, 2]]));
        // An own list is edited as it is.
        let music = Part::Input("music".into());
        assert!(apply(
            &mut c,
            "zoom",
            &Edit::Unlink {
                part: music,
                remove: vec![[2, 4]],
                in_use: vec![],
            }
        ));
        assert_eq!(c.junction[0].input[1].links, Some(vec![[1, 3]]));
    }

    #[test]
    fn a_template_filled_in_adds_a_valid_junction() {
        let mut c = config();
        let t = junc_core::preset::builtin("mic-plus-app").expect("template");
        let fills: std::collections::BTreeMap<String, junc_core::preset::Fill> = [
            (
                "mic".to_owned(),
                junc_core::preset::Fill::value("alsa_input.usb-mic"),
            ),
            ("app".to_owned(), junc_core::preset::Fill::value("spotify")),
        ]
        .into();
        let j = t.fill(&fills).expect("filled");
        assert!(apply(&mut c, "", &Edit::AddJunction(Box::new(j.clone()))));
        assert!(apply(&mut c, "", &Edit::AddJunction(Box::new(j))));
        let ids: Vec<&str> = c.junction.iter().skip(1).map(|j| j.id.as_str()).collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1], "ids made unique");
        let text = toml::to_string(&c).expect("toml");
        junc_core::config::load(&text, "junctions.toml").expect("still valid");
    }

    #[test]
    fn a_duplicate_follows_its_original_and_is_no_default() {
        let mut c = config();
        c.junction[0].default = DefaultRole::Input;
        assert!(apply(&mut c, "zoom", &Edit::Duplicate("Zoom copy".into())));
        assert!(apply(&mut c, "zoom", &Edit::Duplicate("Zoom copy".into())));
        let ids: Vec<&str> = c.junction.iter().map(|j| j.id.as_str()).collect();
        assert_eq!(&ids[..3], ["zoom", "zoom-copy-2", "zoom-copy"]);
        assert_eq!(c.junction[1].name, "Zoom copy");
        assert_eq!(c.junction[1].input, c.junction[0].input);
        assert_eq!(c.junction[1].default, DefaultRole::None);
        let text = toml::to_string(&c).expect("toml");
        junc_core::config::load(&text, "junctions.toml").expect("still valid");
        assert!(!apply(&mut c, "gone", &Edit::Duplicate("x".into())));
    }

    #[test]
    fn a_new_cable_has_one_app_input_and_validates() {
        let mut c = config();
        let app = |b: &str| {
            Box::new(Match {
                binary: Some(b.into()),
                ..Match::default()
            })
        };
        let cable = Edit::NewCable {
            name: "New Cable".into(),
            from: app("firefox"),
            label: "Firefox".into(),
            to: app("obs"),
        };
        assert!(apply(&mut c, "", &cable));
        let j = c.junction.last().expect("cable");
        assert_eq!((j.id.as_str(), j.name.as_str()), ("new-cable", "New Cable"));
        assert_eq!(j.input.len(), 1);
        assert_eq!(j.input[0].id, "firefox");
        assert_eq!(
            j.cable.as_ref().map(|c| c.to.binary.clone()),
            Some(Some("obs".into()))
        );
        let text = toml::to_string(&c).expect("toml");
        junc_core::config::load(&text, "junctions.toml").expect("still valid");
    }

    #[test]
    fn added_inputs_get_their_own_id_and_validate() {
        let mut c = config();
        let firefox = Match {
            binary: Some("firefox".into()),
            ..Match::default()
        };
        let mic = Match {
            device: Some("alsa_input.other".into()),
            ..Match::default()
        };
        assert!(apply(
            &mut c,
            "zoom",
            &Edit::AddInput {
                matcher: Box::new(firefox),
                label: "Firefox".into()
            }
        ));
        // "Music" is taken by an Input already… as "music".
        assert!(apply(
            &mut c,
            "zoom",
            &Edit::AddInput {
                matcher: Box::new(mic),
                label: "Music".into()
            }
        ));
        let ids: Vec<&str> = c.junction[0].input.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["mic", "music", "firefox", "music-2"]);
        assert!(c.junction[0].input[2].enabled);
        assert_eq!(c.junction[0].input[2].volume, 100);
        let text = toml::to_string(&c).expect("toml");
        junc_core::config::load(&text, "junctions.toml").expect("still valid");
        assert!(!apply(
            &mut c,
            "gone",
            &Edit::AddInput {
                matcher: Box::default(),
                label: "x".into()
            }
        ));
    }

    #[test]
    fn added_listen_outputs_get_their_own_id_and_validate() {
        let mut c = config();
        let add = |c: &mut Config, target: &str, label: &str| {
            apply(
                c,
                "zoom",
                &Edit::AddListen {
                    target: target.into(),
                    label: label.into(),
                },
            )
        };
        assert!(add(&mut c, "alsa_output.hdmi", "HDMI Output"));
        assert!(add(&mut c, "default", "default"));
        assert!(add(&mut c, "alsa_output.hdmi-2", "HDMI Output"));
        let l = &c.junction[0].listen;
        let got: Vec<(&str, &str)> = l
            .iter()
            .map(|l| (l.id.as_str(), l.target.as_str()))
            .collect();
        assert_eq!(
            &got[got.len() - 3..],
            [
                ("hdmi-output", "alsa_output.hdmi"),
                ("default", "default"),
                ("hdmi-output-2", "alsa_output.hdmi-2"),
            ]
        );
        assert!(l.iter().all(|l| l.enabled && l.links.is_none()));
        let text = toml::to_string(&c).expect("toml");
        junc_core::config::load(&text, "junctions.toml").expect("still valid");
        assert!(!apply(
            &mut c,
            "gone",
            &Edit::AddListen {
                target: "default".into(),
                label: "default".into()
            }
        ));
    }

    #[test]
    fn the_bus_grows_by_a_pair_or_one_up_to_its_limit() {
        let mut c = config();
        assert!(apply(&mut c, "zoom", &Edit::AddBus(2)));
        assert!(apply(&mut c, "zoom", &Edit::AddBus(1)));
        assert_eq!(c.junction[0].bus.channels, 7);
        c.junction[0].bus.channels = MAX_BUS_CHANNELS - 1;
        assert!(!apply(&mut c, "zoom", &Edit::AddBus(2)), "never past 64");
        assert_eq!(c.junction[0].bus.channels, MAX_BUS_CHANNELS - 1);
        assert!(apply(&mut c, "zoom", &Edit::AddBus(1)));
        let text = toml::to_string(&c).expect("toml");
        junc_core::config::load(&text, "junctions.toml").expect("still valid");
        assert!(!apply(&mut c, "gone", &Edit::AddBus(1)));
    }

    #[test]
    fn input_options_change_exclusive_feeds_and_remove_it() {
        let mut c = config();
        let ex = |on| Edit::Exclusive {
            input: "music".into(),
            on,
        };
        assert!(apply(&mut c, "zoom", &ex(true)));
        assert!(c.junction[0].input[1].exclusive);
        // music: 1 → 3, 2 → 4. Channel 1 to Bus 1 instead; 2 to nothing.
        let feed = |ch, bus| Edit::Feed {
            input: "music".into(),
            ch,
            bus,
            in_use: vec![[1, 3], [2, 4]],
        };
        assert!(apply(&mut c, "zoom", &feed(1, Some(1))));
        assert_eq!(c.junction[0].input[1].links, Some(vec![[1, 1], [2, 4]]));
        assert!(apply(&mut c, "zoom", &feed(2, None)));
        assert_eq!(c.junction[0].input[1].links, Some(vec![[1, 1]]));
        assert!(
            !apply(&mut c, "zoom", &feed(1, Some(5))),
            "no Bus channel 5"
        );
        // mic has no list of its own: the pairs in use become it.
        assert!(apply(
            &mut c,
            "zoom",
            &Edit::Feed {
                input: "mic".into(),
                ch: 2,
                bus: Some(1),
                in_use: vec![[1, 1], [2, 2]],
            }
        ));
        assert_eq!(c.junction[0].input[0].links, Some(vec![[1, 1], [2, 1]]));
        let text = toml::to_string(&c).expect("toml");
        junc_core::config::load(&text, "junctions.toml").expect("still valid");
        assert!(apply(&mut c, "zoom", &Edit::RemoveInput("mic".into())));
        assert!(!apply(&mut c, "zoom", &Edit::RemoveInput("mic".into())));
        let ids: Vec<&str> = c.junction[0].input.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["music"]);
        assert!(!apply(
            &mut c,
            "zoom",
            &Edit::Exclusive {
                input: "mic".into(),
                on: true
            }
        ));
    }

    #[test]
    fn a_cable_changes_its_apps_and_a_listen_its_target() {
        let text = format!(
            "{CONFIG}\n[[junction]]\nid = \"ff\"\nname = \"Firefox → OBS\"\nkind = \"cable\"\n\
             [[junction.input]]\nid = \"firefox\"\nmatch = {{ binary = \"firefox\" }}\n\
             [junction.cable]\nto = {{ binary = \"obs\" }}\n"
        );
        let mut c = junc_core::config::load(&text, "junctions.toml")
            .expect("config")
            .config;
        let app = |b: &str| {
            Box::new(Match {
                binary: Some(b.into()),
                ..Match::default()
            })
        };
        assert!(apply(&mut c, "ff", &Edit::CableFrom(app("chromium"))));
        assert!(apply(&mut c, "ff", &Edit::CableTo(app("discord"))));
        assert_eq!(
            c.junction[1].input[0].matcher.binary.as_deref(),
            Some("chromium")
        );
        assert_eq!(
            c.junction[1].cable.as_ref().map(|c| c.to.binary.as_deref()),
            Some(Some("discord"))
        );
        assert!(
            !apply(&mut c, "zoom", &Edit::CableTo(app("x"))),
            "not a Cable"
        );
        assert!(
            !apply(&mut c, "zoom", &Edit::CableFrom(app("x"))),
            "not a Cable"
        );
        assert!(apply(
            &mut c,
            "zoom",
            &Edit::ListenTarget {
                listen: "hp".into(),
                target: "alsa_output.hdmi".into()
            }
        ));
        assert_eq!(c.junction[0].listen[0].target, "alsa_output.hdmi");
        let text = toml::to_string(&c).expect("toml");
        junc_core::config::load(&text, "junctions.toml").expect("still valid");
    }

    #[test]
    fn linking_adds_the_pair_to_the_ones_in_use() {
        let mut c = config();
        let link = |part: Part, pair, in_use: Vec<[u8; 2]>| Edit::Link { part, pair, in_use };
        // music has its own list: 1 → 3, 2 → 4.
        assert!(apply(
            &mut c,
            "zoom",
            &link(Part::Input("music".into()), [1, 1], vec![])
        ));
        assert_eq!(
            c.junction[0].input[1].links,
            Some(vec![[1, 1], [1, 3], [2, 4]])
        );
        // mic follows the default mapping: it becomes its own list.
        c.junction[0].input[0].suppressed = vec![[2, 2]];
        assert!(apply(
            &mut c,
            "zoom",
            &link(Part::Input("mic".into()), [2, 2], vec![[1, 1]])
        ));
        assert_eq!(c.junction[0].input[0].links, Some(vec![[1, 1], [2, 2]]));
        assert!(
            c.junction[0].input[0].suppressed.is_empty(),
            "not removed any more"
        );
        assert!(apply(
            &mut c,
            "zoom",
            &link(Part::Listen("hp".into()), [3, 1], vec![[1, 1], [2, 2]])
        ));
        assert_eq!(
            c.junction[0].listen[0].links,
            Some(vec![[1, 1], [2, 2], [3, 1]])
        );
        let text = toml::to_string(&c).expect("toml");
        junc_core::config::load(&text, "junctions.toml").expect("still valid");
    }
}
