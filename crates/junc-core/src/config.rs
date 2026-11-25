// SPDX-License-Identifier: GPL-3.0-or-later

//! Loading, validating, migrating and saving `junctions.toml` (F22).
//!
//! Loading runs in four stages and stops at the first stage that finds errors:
//! 1. TOML syntax (`toml_edit`, which keeps spans);
//! 2. schema version and migrations (on the `toml_edit` document);
//! 3. shape: serde into [`Config`], unknown keys get "did you mean";
//! 4. rules serde cannot express ([`validate`]), located by path.
//!
//! Every diagnostic carries a file, line and column when the source text is
//! known, so the GUI banner, `junc config validate` and agents can point at it.

use crate::diag::{Diagnostic, Location, SourceMap, codes};
use crate::migrate;
use crate::model::{Config, DefaultRole, Kind, MAX_BUS_CHANNELS, MAX_VOLUME, MatchKind, Rate};
use std::collections::{HashMap, HashSet};
use toml_edit::{Document, DocumentMut, Item};

/// Whether the file is a user config or a preset/template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    /// `junctions.toml`: template slots are not allowed.
    Config,
    /// Preset or template body: slots allowed, nested Junction ids may refer
    /// to Junctions outside the file.
    Preset,
}

/// A successfully loaded config plus non-fatal findings.
#[derive(Debug)]
pub struct Loaded {
    /// The config, migrated to the current schema.
    pub config: Config,
    /// The document after migration; save edits through it to keep comments.
    pub document: DocumentMut,
    /// Schema version the file had before migration.
    pub migrated_from: Option<u32>,
    /// Warnings.
    pub warnings: Vec<Diagnostic>,
}

/// Parses and validates `src`. `file` is only used in locations.
pub fn load(src: &str, file: &str) -> Result<Loaded, Vec<Diagnostic>> {
    load_as(src, file, Flavor::Config)
}

/// Like [`load`], for a given flavor.
pub fn load_as(src: &str, file: &str, flavor: Flavor) -> Result<Loaded, Vec<Diagnostic>> {
    let map = SourceMap::new(file, src);

    // 1. syntax
    let parsed = Document::parse(src).map_err(|e| {
        let loc = e.span().map(|s| map.locate(s.start));
        vec![
            Diagnostic::error(codes::TOML_SYNTAX, first_line(e.message()))
                .at(loc)
                .with_hint(
                    "Fix the TOML syntax at this position; the last good config stays in use.",
                ),
        ]
    })?;

    // 2. schema version and migrations
    let mut document = parsed.clone().into_mut();
    let migrated_from = migrate::migrate(&mut document).map_err(|d| {
        let loc = locate(&parsed, &map, &[Seg::Key("schema")]);
        vec![d.at(loc)]
    })?;
    // After a migration, positions refer to the original text only for keys
    // that did not move; shape errors are then reported without a location.
    let migrated_text;
    let (shape_src, shape_doc) = if migrated_from.is_some() {
        migrated_text = document.to_string();
        (migrated_text.as_str(), None)
    } else {
        (src, Some(&parsed))
    };

    // 3. shape
    let config: Config = toml::from_str(shape_src).map_err(|e| {
        let loc = shape_doc.and(e.span()).map(|s| map.locate(s.start));
        vec![shape_diagnostic(&e).at(loc)]
    })?;

    // 4. rules
    let findings = validate(&config, flavor);
    let locate_finding = |f: Finding| {
        let loc = shape_doc.and_then(|d| locate(d, &map, &f.path));
        f.diagnostic.at(loc)
    };
    let (errors, warnings): (Vec<_>, Vec<_>) = findings
        .into_iter()
        .map(locate_finding)
        .partition(Diagnostic::is_error);
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Loaded {
        config,
        document,
        migrated_from,
        warnings,
    })
}

/// Serializes a config into a fresh document (used for new files and for
/// exports). Edits to an existing file go through [`Loaded::document`].
pub fn to_toml(config: &Config) -> String {
    let body = toml::to_string_pretty(config).unwrap_or_default();
    format!(
        "# JUNC configuration. Edit by hand or with `junc`; the daemon picks up changes.\n{body}"
    )
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).trim().to_owned()
}

fn shape_diagnostic(e: &toml::de::Error) -> Diagnostic {
    let msg = first_line(e.message());
    if let Some((field, expected)) = parse_unknown_field(&msg) {
        let hint = match closest(&field, &expected) {
            Some(best) => format!(
                "Did you mean '{best}'? Allowed keys: {}.",
                expected.join(", ")
            ),
            None => format!("Allowed keys here: {}.", expected.join(", ")),
        };
        return Diagnostic::error(codes::UNKNOWN_KEY, format!("unknown key '{field}'"))
            .with_hint(hint);
    }
    if let Some(field) = parse_missing_field(&msg) {
        return Diagnostic::error(codes::MISSING_KEY, format!("missing key '{field}'")).with_hint(
            format!("Add `{field} = …`; see `junc schema` for its type."),
        );
    }
    Diagnostic::error(codes::INVALID_VALUE, msg)
        .with_hint("Check the value's type against `junc schema`.")
}

/// serde: "unknown field `volum`, expected one of `id`, `name`" (or "expected `x`").
fn parse_unknown_field(msg: &str) -> Option<(String, Vec<String>)> {
    let rest = msg.strip_prefix("unknown field `")?;
    let (field, rest) = rest.split_once('`')?;
    let expected = rest
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect();
    Some((field.to_owned(), expected))
}

/// serde: "missing field `id`".
fn parse_missing_field(msg: &str) -> Option<String> {
    let rest = msg.strip_prefix("missing field `")?;
    Some(rest.split_once('`')?.0.to_owned())
}

fn closest<'a>(word: &str, options: &'a [String]) -> Option<&'a str> {
    options
        .iter()
        .map(|o| (strsim::damerau_levenshtein(word, o), o))
        .filter(|(d, o)| *d <= 2.max(o.len() / 3))
        .min_by_key(|(d, _)| *d)
        .map(|(_, o)| o.as_str())
}

/// One step of a path into the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seg {
    /// A table key.
    Key(&'static str),
    /// An array index.
    Index(usize),
}

/// A rule violation with the path of the value it is about.
#[derive(Debug)]
pub struct Finding {
    /// Path to the offending value.
    pub path: Vec<Seg>,
    /// The diagnostic (without location).
    pub diagnostic: Diagnostic,
}

/// Location of the value (or, failing that, the key or enclosing table) at
/// `path` in a parsed document.
fn locate(doc: &Document<&str>, map: &SourceMap<'_>, path: &[Seg]) -> Option<Location> {
    let mut best = None;
    walk_item(doc.as_item(), path, &mut best);
    best.map(|s: std::ops::Range<usize>| map.locate(s.start))
}

type Span = Option<std::ops::Range<usize>>;

fn walk_item(item: &Item, path: &[Seg], best: &mut Span) {
    if let Some(s) = item.span() {
        *best = Some(s);
    }
    let Some((seg, rest)) = path.split_first() else {
        return;
    };
    match seg {
        Seg::Key(k) => {
            if let Some(t) = item.as_table_like() {
                walk_key(t, k, rest, best);
            }
        }
        Seg::Index(i) => {
            if let Some(t) = item.as_array_of_tables().and_then(|a| a.get(*i)) {
                if let Some(s) = t.span() {
                    *best = Some(s);
                }
                if let Some((Seg::Key(k), rest)) = rest.split_first() {
                    walk_key(t, k, rest, best);
                }
            } else if let Some(v) = item.as_array().and_then(|a| a.get(*i)) {
                if let Some(s) = v.span() {
                    *best = Some(s);
                }
                if let (Some(t), Some((Seg::Key(k), rest))) =
                    (v.as_inline_table(), rest.split_first())
                {
                    walk_key(t, k, rest, best);
                }
            }
        }
    }
}

fn walk_key(t: &dyn toml_edit::TableLike, k: &str, rest: &[Seg], best: &mut Span) {
    if let Some(s) = t.key(k).and_then(toml_edit::Key::span) {
        *best = Some(s);
    }
    if let Some(next) = t.get(k) {
        walk_item(next, rest, best);
    }
}

/// Checks the rules serde cannot express. Returns errors and warnings.
pub fn validate(config: &Config, flavor: Flavor) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut push = |path: Vec<Seg>, d: Diagnostic| {
        out.push(Finding {
            path,
            diagnostic: d,
        })
    };
    let jp = |j: usize| vec![Seg::Key("junction"), Seg::Index(j)];

    let mut ids = HashSet::new();
    let mut defaults: HashMap<&str, usize> = HashMap::new();
    for (j, junc) in config.junction.iter().enumerate() {
        let at = |k: &'static str| [jp(j), vec![Seg::Key(k)]].concat();
        check_id(&junc.id, at("id"), &mut push);
        if !ids.insert(junc.id.as_str()) {
            push(at("id"), dup(&format!("Junction id '{}'", junc.id)));
        }
        if junc.name.trim().is_empty() {
            push(
                at("name"),
                Diagnostic::error(codes::INVALID_VALUE, "name is empty")
                    .with_hint("Give the Junction a name people will see in their apps."),
            );
        }
        check_volume(junc.volume, at("volume"), &mut push);
        if !(1..=MAX_BUS_CHANNELS).contains(&junc.bus.channels) {
            push(
                [jp(j), vec![Seg::Key("bus"), Seg::Key("channels")]].concat(),
                Diagnostic::error(
                    codes::RANGE,
                    format!("bus.channels is {}", junc.bus.channels),
                )
                .with_hint(format!("Use 1–{MAX_BUS_CHANNELS} channels.")),
            );
        }
        if let Rate::Hz(hz) = junc.rate
            && !(44_100..=192_000).contains(&hz)
        {
            push(
                at("rate"),
                Diagnostic::error(codes::RANGE, format!("rate is {hz} Hz"))
                    .with_hint("Use \"follow\" or a rate from 44100 to 192000."),
            );
        }
        let role = match junc.default {
            DefaultRole::None => None,
            DefaultRole::Input => Some("input"),
            DefaultRole::Output => Some("output"),
        };
        if let Some(role) = role
            && let Some(prev) = defaults.insert(role, j)
        {
            push(
                at("default"),
                Diagnostic::error(
                    codes::DEFAULT_DUPLICATE,
                    format!(
                        "'{}' and '{}' are both the default {role}",
                        config.junction[prev].id, junc.id
                    ),
                )
                .with_hint(format!("Keep `default = \"{role}\"` on one Junction only.")),
            );
        }

        let mut input_ids = HashSet::new();
        for (i, input) in junc.input.iter().enumerate() {
            let ip = |k: &'static str| {
                [jp(j), vec![Seg::Key("input"), Seg::Index(i), Seg::Key(k)]].concat()
            };
            check_id(&input.id, ip("id"), &mut push);
            if !input_ids.insert(input.id.as_str()) {
                push(
                    ip("id"),
                    dup(&format!("Input id '{}' in '{}'", input.id, junc.id)),
                );
            }
            check_volume(input.volume, ip("volume"), &mut push);
            check_match(&input.matcher, ip("match"), flavor, &mut push);
            for (n, [from, to]) in input.links.iter().flatten().enumerate() {
                let lp = [ip("links"), vec![Seg::Index(n)]].concat();
                if *from == 0 || *from > MAX_BUS_CHANNELS {
                    push(lp.clone(), channel_err(*from, "input", MAX_BUS_CHANNELS));
                }
                if *to == 0 || *to > junc.bus.channels {
                    push(lp, channel_err(*to, "Bus", junc.bus.channels));
                }
            }
            if let Some(target) = &input.matcher.junction {
                if target == &junc.id {
                    push(
                        ip("match"),
                        Diagnostic::error(
                            codes::CYCLE,
                            format!("'{}' takes itself as an Input", junc.id),
                        )
                        .with_hint("A Junction cannot feed itself."),
                    );
                } else if flavor == Flavor::Config
                    && !config.junction.iter().any(|o| &o.id == target)
                {
                    push(
                        ip("match"),
                        Diagnostic::error(
                            codes::JUNCTION_UNKNOWN,
                            format!("no Junction with id '{target}'"),
                        )
                        .with_hint(suggest_ids(target, &junc.id, config)),
                    );
                }
            }
        }
        if junc.input.len() > 32 {
            push(
                at("input"),
                Diagnostic::warning(
                    codes::MANY_INPUTS,
                    format!("'{}' has {} Inputs", junc.id, junc.input.len()),
                )
                .with_hint("This works, but mixing many streams costs CPU in PipeWire."),
            );
        }

        let mut listen_ids = HashSet::new();
        for (l, listen) in junc.listen.iter().enumerate() {
            let lp0 = |k: &'static str| {
                [jp(j), vec![Seg::Key("listen"), Seg::Index(l), Seg::Key(k)]].concat()
            };
            check_id(&listen.id, lp0("id"), &mut push);
            if !listen_ids.insert(listen.id.as_str()) {
                push(
                    lp0("id"),
                    dup(&format!("Listen id '{}' in '{}'", listen.id, junc.id)),
                );
            }
            check_volume(listen.volume, lp0("volume"), &mut push);
            if listen.target.trim().is_empty() {
                push(
                    lp0("target"),
                    Diagnostic::error(codes::INVALID_VALUE, "Listen target is empty").with_hint(
                        "Use \"default\" or an output's node.name from `junc describe`.",
                    ),
                );
            }
            for (n, [from, to]) in listen.links.iter().flatten().enumerate() {
                let lp = [lp0("links"), vec![Seg::Index(n)]].concat();
                if *from == 0 || *from > junc.bus.channels {
                    push(lp.clone(), channel_err(*from, "Bus", junc.bus.channels));
                }
                if *to == 0 || *to > MAX_BUS_CHANNELS {
                    push(lp, channel_err(*to, "output", MAX_BUS_CHANNELS));
                }
            }
        }

        match (junc.kind, &junc.cable) {
            (Kind::Cable, None) => push(at("kind"), Diagnostic::error(codes::CABLE,
                format!("Cable '{}' has no `cable.to`", junc.id))
                .with_hint("Add `[junction.cable]` with `to = { binary = \"…\" }`: the app that hears the Cable.")),
            (Kind::Cable, Some(c)) => {
                check_match(&c.to, [jp(j), vec![Seg::Key("cable"), Seg::Key("to")]].concat(), flavor, &mut push);
                if junc.input.len() != 1 || junc.input[0].matcher.kinds() != [MatchKind::App] {
                    push(at("input"), Diagnostic::error(codes::CABLE,
                        format!("Cable '{}' needs exactly one app Input", junc.id))
                        .with_hint("A Cable carries one app into another; use a Junction to mix more."));
                }
            }
            (Kind::Junction, Some(_)) => push(at("cable"), Diagnostic::error(codes::CABLE,
                format!("'{}' has a `cable` table but kind = \"junction\"", junc.id))
                .with_hint("Set kind = \"cable\", or remove the `cable` table.")),
            (Kind::Junction, None) => {}
        }
    }
    if flavor == Flavor::Config {
        out.extend(find_cycles(config));
    }
    out
}

fn dup(what: &str) -> Diagnostic {
    Diagnostic::error(codes::ID_DUPLICATE, format!("{what} is used twice"))
        .with_hint("Ids must be unique; rename one of them.")
}

fn channel_err(ch: u8, side: &str, max: u8) -> Diagnostic {
    Diagnostic::error(
        codes::LINK_CHANNEL,
        format!("Link uses {side} channel {ch}"),
    )
    .with_hint(format!("{side} channels are numbered 1–{max}."))
}

fn check_id(id: &str, path: Vec<Seg>, push: &mut impl FnMut(Vec<Seg>, Diagnostic)) {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-');
    if !ok {
        push(
            path,
            Diagnostic::error(codes::ID_FORMAT, format!("id '{id}' is not valid")).with_hint(
                format!(
                    "Use lowercase letters, digits and '-', e.g. '{}'.",
                    slugify(id)
                ),
            ),
        );
    }
}

fn check_volume(v: u16, path: Vec<Seg>, push: &mut impl FnMut(Vec<Seg>, Diagnostic)) {
    if v > MAX_VOLUME {
        push(
            path,
            Diagnostic::error(codes::RANGE, format!("volume is {v}"))
                .with_hint(format!("Volume is a percentage from 0 to {MAX_VOLUME}.")),
        );
    }
}

fn check_match(
    m: &crate::model::Match,
    path: Vec<Seg>,
    flavor: Flavor,
    push: &mut impl FnMut(Vec<Seg>, Diagnostic),
) {
    let kinds = m.kinds();
    if kinds.len() != 1 {
        let names: Vec<&str> = kinds.iter().map(|k| k.label()).collect();
        let msg = if kinds.is_empty() {
            "match has no keys".to_owned()
        } else {
            format!("match mixes {} keys", names.join(" and "))
        };
        push(path.clone(), Diagnostic::error(codes::MATCH_KIND, msg).with_hint(
            "Use app keys (app, binary, role, node), or device, or monitor, or junction — exactly one kind.",
        ));
    }
    if kinds.contains(&MatchKind::Slot) && flavor == Flavor::Config {
        push(
            path,
            Diagnostic::error(codes::SLOT_OUTSIDE_PRESET, "template slot in a config").with_hint(
                "Slots are filled when a template is applied; use a concrete match here.",
            ),
        );
    }
}

fn suggest_ids(target: &str, own: &str, config: &Config) -> String {
    let ids: Vec<String> = config
        .junction
        .iter()
        .map(|j| j.id.clone())
        .filter(|id| id != own)
        .collect();
    match closest(target, &ids) {
        Some(best) => format!("Did you mean '{best}'?"),
        None if ids.is_empty() => "There are no other Junctions to nest.".to_owned(),
        None => format!("Existing Junctions: {}.", ids.join(", ")),
    }
}

/// Turns free text into a valid id: lowercase ASCII, digits, single dashes.
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() {
        "junction".to_owned()
    } else {
        out
    }
}

/// Nesting loops longer than one step (self-nesting is reported per Input).
fn find_cycles(config: &Config) -> Vec<Finding> {
    let edges: HashMap<&str, Vec<(&str, usize, usize)>> = config
        .junction
        .iter()
        .enumerate()
        .map(|(j, junc)| {
            let targets = junc
                .input
                .iter()
                .enumerate()
                .filter_map(|(i, inp)| inp.matcher.junction.as_deref().map(|t| (t, j, i)))
                .filter(|(t, _, _)| *t != junc.id)
                .collect();
            (junc.id.as_str(), targets)
        })
        .collect();
    let mut out = Vec::new();
    let mut reported = HashSet::new();
    for start in edges.keys() {
        let mut stack = vec![(*start, vec![*start])];
        while let Some((node, trail)) = stack.pop() {
            for (next, j, i) in edges.get(node).into_iter().flatten() {
                if *next == *start {
                    let mut key: Vec<&str> = trail.clone();
                    key.sort_unstable();
                    if reported.insert(key) {
                        let loop_text = [trail.as_slice(), &[*start]].concat().join(" → ");
                        out.push(Finding {
                            path: vec![
                                Seg::Key("junction"),
                                Seg::Index(*j),
                                Seg::Key("input"),
                                Seg::Index(*i),
                                Seg::Key("match"),
                            ],
                            diagnostic: Diagnostic::error(
                                codes::CYCLE,
                                format!("nesting loop: {loop_text}"),
                            )
                            .with_hint(
                                "Remove one of these Junction Inputs; audio would feed back.",
                            ),
                        });
                    }
                } else if !trail.contains(next) {
                    stack.push((next, [trail.as_slice(), &[*next]].concat()));
                }
            }
        }
    }
    out
}
