// SPDX-License-Identifier: GPL-3.0-or-later

//! Presets and templates (docs/architecture.md §6.3, F22, F24).
//!
//! One format for both: a `[preset]` header and a single `[junction]` table.
//! A template is a preset whose matches name **slots**, filled when it is
//! used: `match = { slot = "mic" }` on an Input or a Cable's `to`, and
//! `target = "{out}"` on a Listen output. `{slot}` in the Junction's name is
//! replaced by the value (or label) given for the slot. Filling yields an
//! ordinary Junction; nothing in it refers back to the template.
//!
//! The Junction may leave out `id`: the one it gets when imported is made
//! from its name and kept unique in the config.

use crate::config::{self, Finding, Flavor, Seg};
use crate::diag::{Diagnostic, SourceMap, codes};
use crate::model::{Config, Junction, Kind, Match, SCHEMA_VERSION};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The `[preset]` header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Header {
    /// Name shown in the template and import lists.
    pub name: String,
    /// One sentence on what it makes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// An example of what to pick, e.g. "e.g. USB Microphone + Spotify".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
    /// Icon name for the GUI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Shipped with JUNC (`/usr/share/junc/templates`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub builtin: bool,
    /// What the user picks when the template is used, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slot: Vec<Slot>,
}

/// Something to pick when a template is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    /// Name used in `match = { slot = "…" }`, `target = "{…}"` and `{…}` in
    /// the Junction's name.
    pub id: String,
    /// What kind of thing to pick.
    pub kind: SlotKind,
    /// The question to ask, e.g. "Pick a mic".
    pub prompt: String,
}

/// What a slot is filled with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SlotKind {
    /// A hardware input, by `node.name` (becomes `match = { device = … }`).
    HardwareInput,
    /// An app, by binary (becomes `match = { binary = … }`).
    App,
    /// An output, by `node.name` or `default` (a Listen `target`).
    Output,
    /// What an output plays, by sink `node.name` (`match = { monitor = … }`).
    Monitor,
}

impl SlotKind {
    /// The kebab-case name, as written in files.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HardwareInput => "hardware-input",
            Self::App => "app",
            Self::Output => "output",
            Self::Monitor => "monitor",
        }
    }

    /// The match a value of this kind becomes (not for outputs).
    fn matcher(self, value: &str) -> Match {
        let v = Some(value.to_owned());
        match self {
            Self::HardwareInput => Match {
                device: v,
                ..Match::default()
            },
            Self::Monitor => Match {
                monitor: v,
                ..Match::default()
            },
            Self::App | Self::Output => Match {
                binary: v,
                ..Match::default()
            },
        }
    }
}

/// A loaded preset or template.
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    /// The `[preset]` header.
    pub header: Header,
    /// The Junction it makes; `id` is the file's or one made from the name.
    pub junction: Junction,
}

/// A loaded preset plus non-fatal findings.
#[derive(Debug)]
pub struct Loaded {
    /// The preset.
    pub preset: Preset,
    /// Warnings.
    pub warnings: Vec<Diagnostic>,
}

/// What the user picked for a slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fill {
    /// Binary, `node.name`, or `default` for an output.
    pub value: String,
    /// Name to show in the Junction's name (e.g. the device's description);
    /// the value when `None`.
    pub label: Option<String>,
}

impl Fill {
    /// A fill whose label is its value.
    pub fn value(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: None,
        }
    }
}

/// The file layout, before the Junction is read.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    /// Checked by the migration step.
    #[serde(rename = "schema")]
    _schema: u32,
    preset: Header,
    junction: toml::Table,
}

/// The file layout when writing.
#[derive(Serialize)]
struct Out<'a> {
    schema: u32,
    preset: &'a Header,
    junction: &'a Junction,
}

/// `{name}` placeholders in `text`.
fn placeholders(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else { break };
        out.push(&after[..end]);
        rest = &after[end + 1..];
    }
    out
}

/// The slot a Listen target names (`"{out}"`), if it is a placeholder.
fn target_slot(target: &str) -> Option<&str> {
    target.strip_prefix('{')?.strip_suffix('}')
}

/// Parses and validates a preset or template. `file` is only used in
/// locations.
pub fn load(src: &str, file: &str) -> Result<Loaded, Vec<Diagnostic>> {
    let map = SourceMap::new(file, src);
    let parsed = toml_edit::Document::parse(src).map_err(|e| {
        let loc = e.span().map(|s| map.locate(s.start));
        vec![
            Diagnostic::error(codes::TOML_SYNTAX, config::first_line(e.message()))
                .at(loc)
                .with_hint("Fix the TOML syntax at this position."),
        ]
    })?;
    let at = |path: &[Seg]| config::locate(&parsed, &map, path);

    let mut doc = parsed.clone().into_mut();
    let migrated =
        crate::migrate::migrate(&mut doc).map_err(|d| vec![d.at(at(&[Seg::Key("schema")]))])?;
    let migrated_text;
    let shape_src = if migrated.is_some() {
        migrated_text = doc.to_string();
        migrated_text.as_str()
    } else {
        src
    };
    let located = migrated.is_none();

    let f: File = toml::from_str(shape_src).map_err(|e| {
        let loc = e.span().filter(|_| located).map(|s| map.locate(s.start));
        vec![config::shape_diagnostic(&e).at(loc)]
    })?;
    let mut table = f.junction;
    if !table.contains_key("id") {
        let name = table
            .get("name")
            .and_then(toml::Value::as_str)
            .unwrap_or("");
        let id = config::slugify(&strip_placeholders(name));
        table.insert("id".into(), id.into());
    }
    // The Junction table has no spans of its own here: shape errors in it
    // point at `[junction]`.
    let junction: Junction = toml::Value::Table(table)
        .try_into()
        .map_err(|e| vec![config::shape_diagnostic(&e).at(at(&[Seg::Key("junction")]))])?;

    let preset = Preset {
        header: f.preset,
        junction,
    };
    let findings = check(&preset);
    let (errors, warnings): (Vec<_>, Vec<_>) = findings
        .into_iter()
        .map(|f| {
            let loc = if located { at(&f.path) } else { None };
            f.diagnostic.at(loc)
        })
        .partition(Diagnostic::is_error);
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Loaded { preset, warnings })
}

/// The name without `{slot}` placeholders, for making an id.
fn strip_placeholders(name: &str) -> String {
    let mut s = name.to_owned();
    for p in placeholders(name) {
        s = s.replace(&format!("{{{p}}}"), p);
    }
    s
}

/// The config rules (on a config holding just this Junction) and the slot
/// rules. Paths point into the preset file.
fn check(p: &Preset) -> Vec<Finding> {
    let cfg = Config {
        schema: SCHEMA_VERSION,
        junction: vec![p.junction.clone()],
    };
    let mut out: Vec<Finding> = config::validate(&cfg, Flavor::Preset)
        .into_iter()
        .map(|mut f| {
            // `junction[0].…` in a config is `junction.…` here.
            if f.path.get(1) == Some(&Seg::Index(0)) {
                f.path.remove(1);
            }
            f
        })
        .collect();
    let mut push = |path: Vec<Seg>, d: Diagnostic| {
        out.push(Finding {
            path,
            diagnostic: d,
        });
    };

    let mut kinds: BTreeMap<&str, SlotKind> = BTreeMap::new();
    for (i, s) in p.header.slot.iter().enumerate() {
        let path = vec![
            Seg::Key("preset"),
            Seg::Key("slot"),
            Seg::Index(i),
            Seg::Key("id"),
        ];
        if config::slugify(&s.id) != s.id {
            push(
                path.clone(),
                Diagnostic::error(codes::ID_FORMAT, format!("slot id '{}' is not valid", s.id))
                    .with_hint(format!(
                        "Use lowercase letters, digits and '-', e.g. '{}'.",
                        config::slugify(&s.id)
                    )),
            );
        }
        if kinds.insert(&s.id, s.kind).is_some() {
            push(
                path,
                Diagnostic::error(
                    codes::ID_DUPLICATE,
                    format!("slot '{}' is declared twice", s.id),
                )
                .with_hint("Give each slot its own id."),
            );
        }
    }

    let mut used = BTreeSet::new();
    let mut use_slot =
        |id: &str, allowed: &[SlotKind], path: Vec<Seg>, used: &mut BTreeSet<String>| {
            used.insert(id.to_owned());
            match kinds.get(id) {
                None => push(path, unknown_slot(id, &kinds)),
                Some(k) if !allowed.contains(k) => push(
                    path,
                    Diagnostic::error(
                        codes::PRESET_SLOT,
                        format!(
                            "slot '{id}' is a {} slot and cannot be used here",
                            k.as_str()
                        ),
                    )
                    .with_hint(format!(
                        "Here a slot must be: {}.",
                        allowed
                            .iter()
                            .map(|k| k.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                ),
                Some(_) => {}
            }
        };
    let j = &p.junction;
    // A Cable carries one app; a Junction takes anything that plays.
    let input_kinds: &[SlotKind] = if j.kind == Kind::Cable {
        &[SlotKind::App]
    } else {
        &[SlotKind::HardwareInput, SlotKind::App, SlotKind::Monitor]
    };
    for (i, input) in j.input.iter().enumerate() {
        if let Some(id) = &input.matcher.slot {
            use_slot(
                id,
                input_kinds,
                vec![
                    Seg::Key("junction"),
                    Seg::Key("input"),
                    Seg::Index(i),
                    Seg::Key("match"),
                ],
                &mut used,
            );
        }
    }
    if let Some(id) = j.cable.as_ref().and_then(|c| c.to.slot.as_ref()) {
        use_slot(
            id,
            &[SlotKind::App],
            vec![Seg::Key("junction"), Seg::Key("cable"), Seg::Key("to")],
            &mut used,
        );
    }
    for (i, l) in j.listen.iter().enumerate() {
        if let Some(id) = target_slot(&l.target) {
            use_slot(
                id,
                &[SlotKind::Output],
                vec![
                    Seg::Key("junction"),
                    Seg::Key("listen"),
                    Seg::Index(i),
                    Seg::Key("target"),
                ],
                &mut used,
            );
        }
    }
    // Without slots, braces in the name are just text.
    for id in placeholders(&j.name)
        .into_iter()
        .filter(|_| !kinds.is_empty())
    {
        if !kinds.contains_key(id) {
            push(
                vec![Seg::Key("junction"), Seg::Key("name")],
                unknown_slot(id, &kinds),
            );
        }
    }
    for (i, s) in p.header.slot.iter().enumerate() {
        if !used.contains(&s.id) {
            push(
                vec![Seg::Key("preset"), Seg::Key("slot"), Seg::Index(i)],
                Diagnostic::error(codes::PRESET_SLOT, format!("slot '{}' is not used", s.id))
                    .with_hint("Use it in a match or a Listen target, or remove it."),
            );
        }
    }
    out
}

fn unknown_slot(id: &str, kinds: &BTreeMap<&str, SlotKind>) -> Diagnostic {
    let declared: Vec<&str> = kinds.keys().copied().collect();
    Diagnostic::error(codes::PRESET_SLOT, format!("no slot '{id}'")).with_hint(
        if declared.is_empty() {
            format!("Declare it under [[preset.slot]] with id = \"{id}\".")
        } else {
            format!("Declared slots: {}.", declared.join(", "))
        },
    )
}

impl Preset {
    /// Fills the slots and returns the Junction to add. Every slot needs a
    /// value; values for slots the preset does not have are refused.
    pub fn fill(&self, fills: &BTreeMap<String, Fill>) -> Result<Junction, Diagnostic> {
        let slots = &self.header.slot;
        if let Some(extra) = fills.keys().find(|k| !slots.iter().any(|s| &s.id == *k)) {
            let declared: Vec<&str> = slots.iter().map(|s| s.id.as_str()).collect();
            return Err(Diagnostic::error(
                codes::PRESET_FILL,
                format!("'{}' has no slot '{extra}'", self.header.name),
            )
            .with_hint(if declared.is_empty() {
                "It has no slots to fill.".to_owned()
            } else {
                format!("Its slots: {}.", declared.join(", "))
            }));
        }
        let missing: Vec<&Slot> = slots
            .iter()
            .filter(|s| !fills.contains_key(&s.id))
            .collect();
        if !missing.is_empty() {
            return Err(Diagnostic::error(
                codes::PRESET_FILL,
                format!(
                    "'{}' needs a value for {}",
                    self.header.name,
                    missing
                        .iter()
                        .map(|s| format!("'{}' ({})", s.id, s.prompt))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
            .with_hint(format!(
                "Give each as --slot ID=VALUE, e.g. --slot {}=….",
                missing[0].id
            )));
        }
        let kind = |id: &str| slots.iter().find(|s| s.id == id).map(|s| s.kind);
        let mut j = self.junction.clone();
        let fill_match = |m: &mut Match| {
            if let Some(id) = m.slot.take()
                && let (Some(k), Some(f)) = (kind(&id), fills.get(&id))
            {
                let profile = m.profile.take();
                *m = k.matcher(&f.value);
                m.profile = profile;
            }
        };
        for input in &mut j.input {
            fill_match(&mut input.matcher);
        }
        if let Some(c) = &mut j.cable {
            fill_match(&mut c.to);
        }
        for l in &mut j.listen {
            if let Some(f) = target_slot(&l.target).and_then(|id| fills.get(id)) {
                l.target.clone_from(&f.value);
            }
        }
        for (id, f) in fills {
            j.name = j
                .name
                .replace(&format!("{{{id}}}"), f.label.as_deref().unwrap_or(&f.value));
        }
        Ok(j)
    }

    /// A preset of an existing Junction (export). It keeps everything,
    /// including device-specific matches; see [`Preset::device_specific`].
    pub fn from_junction(j: &Junction) -> Self {
        Self {
            header: Header {
                name: j.name.clone(),
                description: None,
                example: None,
                icon: None,
                builtin: false,
                slot: Vec::new(),
            },
            junction: j.clone(),
        }
    }

    /// The preset file text.
    pub fn to_toml(&self) -> String {
        let out = Out {
            schema: SCHEMA_VERSION,
            preset: &self.header,
            junction: &self.junction,
        };
        let body = toml::to_string(&out).unwrap_or_default();
        // Written the way the docs show it: short tables inline.
        let body = match body.parse::<toml_edit::DocumentMut>() {
            Ok(mut doc) => {
                inline_short_tables(doc.as_table_mut());
                doc.to_string()
            }
            Err(_) => body,
        };
        format!("# JUNC preset. Import it with `junc preset import FILE`.\n{body}")
    }

    /// Inputs and Listen outputs tied to this machine's devices: they match
    /// by name on another machine and show offline until it has the device.
    pub fn device_specific(&self) -> Vec<String> {
        let j = &self.junction;
        j.input
            .iter()
            .filter(|i| i.matcher.device.is_some() || i.matcher.monitor.is_some())
            .map(|i| format!("input/{}", i.id))
            .chain(
                j.listen
                    .iter()
                    .filter(|l| l.target != "default" && target_slot(&l.target).is_none())
                    .map(|l| format!("listen/{}", l.id)),
            )
            .collect()
    }
}

/// Turns `match`, `bus` and `to` tables into inline tables, everywhere, and
/// puts `match` right after `id`.
fn inline_short_tables(t: &mut toml_edit::Table) {
    for (mut key, item) in t.iter_mut() {
        let short = matches!(key.get(), "match" | "bus" | "to");
        match item {
            toml_edit::Item::Table(inner) if short => {
                let inline = inner.clone().into_inline_table();
                *item = toml_edit::Item::Value(toml_edit::Value::InlineTable(inline));
                let decor = key.leaf_decor_mut();
                decor.set_prefix("");
                decor.set_suffix(" ");
            }
            toml_edit::Item::Table(inner) => inline_short_tables(inner),
            toml_edit::Item::ArrayOfTables(a) => a.iter_mut().for_each(inline_short_tables),
            _ => {}
        }
    }
    let rank = |k: &toml_edit::Key| match k.get() {
        "id" => 0,
        "match" => 1,
        _ => 2,
    };
    t.sort_values_by(|k1, _, k2, _| rank(k1).cmp(&rank(k2)));
}

/// Built-in templates (F24), as shipped in `templates/` and installed to
/// `/usr/share/junc/templates`: `(id, file text)`, in the order to offer
/// them.
pub const BUILTIN: &[(&str, &str)] = &[
    (
        "mic-plus-app",
        include_str!("../../../templates/mic-plus-app.toml"),
    ),
    ("cable", include_str!("../../../templates/cable.toml")),
    (
        "combine-inputs",
        include_str!("../../../templates/combine-inputs.toml"),
    ),
    (
        "separate-tracks",
        include_str!("../../../templates/separate-tracks.toml"),
    ),
    (
        "stream-mix",
        include_str!("../../../templates/stream-mix.toml"),
    ),
    (
        "start-empty",
        include_str!("../../../templates/start-empty.toml"),
    ),
];

/// A built-in template by id.
pub fn builtin(id: &str) -> Option<Preset> {
    let (_, text) = BUILTIN.iter().find(|(i, _)| *i == id)?;
    load(text, &format!("{id}.toml")).ok().map(|l| l.preset)
}
