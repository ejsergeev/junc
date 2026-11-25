// SPDX-License-Identifier: GPL-3.0-or-later

//! Presets and templates (F22, F24): built-in templates, slot filling,
//! export/import round trips, and the errors a hand-written preset gets.

use junc_core::config::{self, Flavor};
use junc_core::diag::Diagnostic;
use junc_core::model::{Config, Kind, Match};
use junc_core::preset::{self, BUILTIN, Fill, SlotKind};
use std::collections::BTreeMap;
use std::path::Path;

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn show(d: &[Diagnostic]) -> String {
    d.iter().map(|d| format!("  {d}\n")).collect()
}

/// A value of the right kind for each slot.
fn sample(kind: SlotKind, id: &str) -> Fill {
    Fill::value(match kind {
        SlotKind::HardwareInput => format!("alsa_input.{id}"),
        SlotKind::App => format!("{id}-app"),
        SlotKind::Output => "default".to_owned(),
        SlotKind::Monitor => format!("alsa_output.{id}"),
    })
}

#[test]
fn the_builtin_list_is_the_templates_folder() {
    let mut files: Vec<String> = std::fs::read_dir(root().join("templates"))
        .expect("templates/")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".toml"))
        .map(|n| n.trim_end_matches(".toml").to_owned())
        .collect();
    files.sort();
    let mut ids: Vec<String> = BUILTIN.iter().map(|(id, _)| (*id).to_owned()).collect();
    ids.sort();
    assert_eq!(files, ids, "templates/ and preset::BUILTIN differ");
    // F24: the set the PRD names.
    for id in [
        "cable",
        "mic-plus-app",
        "combine-inputs",
        "separate-tracks",
        "stream-mix",
        "start-empty",
    ] {
        assert!(ids.iter().any(|i| i == id), "{id} missing");
    }
}

#[test]
fn every_builtin_template_validates_and_fills_into_a_valid_junction() {
    for (id, text) in BUILTIN {
        let loaded = preset::load(text, &format!("{id}.toml"))
            .unwrap_or_else(|d| panic!("{id}:\n{}", show(&d)));
        assert!(loaded.warnings.is_empty(), "{id}: {:?}", loaded.warnings);
        let p = loaded.preset;
        assert!(p.header.builtin, "{id}: builtin = true");
        assert!(p.header.description.is_some(), "{id}: description");
        let fills: BTreeMap<String, Fill> = p
            .header
            .slot
            .iter()
            .map(|s| (s.id.clone(), sample(s.kind, &s.id)))
            .collect();
        let j = p.fill(&fills).unwrap_or_else(|d| panic!("{id}: {d}"));
        assert!(!j.name.contains('{'), "{id}: name not filled: {}", j.name);
        let cfg = Config {
            junction: vec![j],
            ..Config::default()
        };
        let errors: Vec<_> = config::validate(&cfg, Flavor::Config)
            .into_iter()
            .filter(|f| f.diagnostic.is_error())
            .map(|f| f.diagnostic)
            .collect();
        assert!(errors.is_empty(), "{id} filled:\n{}", show(&errors));
        // And the filled Junction is an ordinary config.
        let text = config::to_toml(&cfg);
        let back = config::load(&text, "t.toml").unwrap_or_else(|d| panic!("{id}:\n{}", show(&d)));
        assert_eq!(back.config, cfg, "{id}");
    }
}

#[test]
fn mic_plus_app_fills_matches_and_name() {
    let p = preset::builtin("mic-plus-app").expect("template");
    let fills = BTreeMap::from([
        (
            "mic".to_owned(),
            Fill {
                value: "alsa_input.usb-mic".into(),
                label: Some("USB Microphone".into()),
            },
        ),
        ("app".to_owned(), Fill::value("spotify")),
    ]);
    let j = p.fill(&fills).expect("fill");
    assert_eq!(j.id, "app-mic");
    assert_eq!(j.name, "spotify + USB Microphone");
    assert_eq!(
        j.input[0].matcher,
        Match {
            device: Some("alsa_input.usb-mic".into()),
            ..Match::default()
        }
    );
    assert_eq!(
        j.input[1].matcher,
        Match {
            binary: Some("spotify".into()),
            ..Match::default()
        }
    );
}

#[test]
fn the_cable_template_makes_a_cable() {
    let p = preset::builtin("cable").expect("template");
    let fills = BTreeMap::from([
        ("from".to_owned(), Fill::value("firefox")),
        ("to".to_owned(), Fill::value("Discord")),
    ]);
    let j = p.fill(&fills).expect("fill");
    assert_eq!(j.kind, Kind::Cable);
    assert_eq!(j.name, "firefox to Discord");
    assert_eq!(
        j.cable.expect("cable").to.binary.as_deref(),
        Some("Discord")
    );
}

#[test]
fn fill_refuses_missing_and_unknown_slots() {
    let p = preset::load(OUTPUT_SLOT, "p.toml").expect("valid").preset;
    let only_mic = BTreeMap::from([("mic".to_owned(), Fill::value("m"))]);
    let e = p.fill(&only_mic).expect_err("out missing");
    assert_eq!(e.code, "E_PRESET_FILL");
    assert!(e.message.contains("'out'"), "{e}");
    let mut extra = only_mic.clone();
    extra.insert("out".into(), Fill::value("default"));
    extra.insert("guitar".into(), Fill::value("g"));
    let e = p.fill(&extra).expect_err("no such slot");
    assert_eq!(e.code, "E_PRESET_FILL");
    assert!(
        e.hint.as_deref().unwrap_or("").contains("mic, out"),
        "{e:?}"
    );
}

#[test]
fn a_cable_preset_takes_its_apps_from_slots() {
    let text = r#"schema = 1
[preset]
name = "Cable"
[[preset.slot]]
id = "from"
kind = "app"
prompt = "Pick the app that plays"
[[preset.slot]]
id = "to"
kind = "app"
prompt = "Pick the app that listens"
[junction]
name = "{from} to {to}"
kind = "cable"
[[junction.input]]
id = "from"
match = { slot = "from" }
[junction.cable]
to = { slot = "to" }
"#;
    let p = preset::load(text, "p.toml").expect("valid").preset;
    let j = p
        .fill(&BTreeMap::from([
            ("from".to_owned(), Fill::value("firefox")),
            ("to".to_owned(), Fill::value("Discord")),
        ]))
        .expect("fill");
    assert_eq!(j.kind, Kind::Cable);
    assert_eq!(j.name, "firefox to Discord");
    assert_eq!(
        j.cable.expect("cable").to.binary.as_deref(),
        Some("Discord")
    );
    // Only an app slot can be what the Cable carries.
    let wrong = text.replace(
        "kind = \"app\"\nprompt = \"Pick the app that plays\"",
        "kind = \"monitor\"\nprompt = \"Pick the app that plays\"",
    );
    assert_eq!(error(&wrong).0, "E_PRESET_SLOT");
}

#[test]
fn export_and_import_round_trip_every_fixture_junction() {
    for f in ["full", "minimal", "nested"] {
        let path = root().join(format!("tests/fixtures/configs/valid/{f}.toml"));
        let text = std::fs::read_to_string(&path).expect("fixture");
        let cfg = config::load(&text, &format!("{f}.toml"))
            .expect("valid")
            .config;
        for j in &cfg.junction {
            let exported = preset::Preset::from_junction(j);
            let text = exported.to_toml();
            let back = preset::load(&text, "export.toml")
                .unwrap_or_else(|d| panic!("{f}/{}:\n{}\n{text}", j.id, show(&d)));
            assert_eq!(back.preset, exported, "{f}/{}:\n{text}", j.id);
            let imported = back.preset.fill(&BTreeMap::new()).expect("no slots");
            assert_eq!(&imported, j, "{f}/{}", j.id);
        }
    }
}

#[test]
fn exports_name_what_is_tied_to_this_machine() {
    let cfg = config::load(
        r#"schema = 1
[[junction]]
id = "a"
name = "A"
[[junction.input]]
id = "mic"
match = { device = "alsa_input.usb" }
[[junction.input]]
id = "app"
match = { binary = "spotify" }
[[junction.listen]]
id = "hp"
target = "alsa_output.usb"
[[junction.listen]]
id = "speakers"
target = "default"
"#,
        "t.toml",
    )
    .expect("config")
    .config;
    let p = preset::Preset::from_junction(&cfg.junction[0]);
    assert_eq!(p.device_specific(), ["input/mic", "listen/hp"]);
}

const OUTPUT_SLOT: &str = r#"schema = 1

[preset]
name = "Mic to an output"

[[preset.slot]]
id = "mic"
kind = "hardware-input"
prompt = "Pick a mic"

[[preset.slot]]
id = "out"
kind = "output"
prompt = "Where to hear it"

[junction]
name = "{mic} on {out}"

[[junction.input]]
id = "mic"
match = { slot = "mic" }

[[junction.listen]]
id = "out"
target = "{out}"
"#;

#[test]
fn an_output_slot_fills_a_listen_target() {
    let p = preset::load(OUTPUT_SLOT, "p.toml").expect("valid").preset;
    assert_eq!(p.junction.id, "mic-on-out", "id made from the name");
    let j = p
        .fill(&BTreeMap::from([
            ("mic".to_owned(), Fill::value("alsa_input.x")),
            ("out".to_owned(), Fill::value("default")),
        ]))
        .expect("fill");
    assert_eq!(j.listen[0].target, "default");
    assert_eq!(j.name, "alsa_input.x on default");
}

/// `(code, line)` of the first error loading `text`.
fn error(text: &str) -> (String, usize) {
    let d = preset::load(text, "p.toml").expect_err("invalid");
    (
        d[0].code.to_owned(),
        d[0].location.as_ref().map_or(0, |l| l.line),
    )
}

#[test]
fn slot_mistakes_are_reported_where_they_are() {
    // A match names a slot nobody declared (line 21).
    assert_eq!(
        error(&OUTPUT_SLOT.replace(
            r#"match = { slot = "mic" }"#,
            r#"match = { slot = "mike" }"#
        )),
        ("E_PRESET_SLOT".into(), 21),
    );
    // An output slot cannot feed an Input.
    assert_eq!(
        error(&OUTPUT_SLOT.replace(r#"match = { slot = "mic" }"#, r#"match = { slot = "out" }"#)),
        ("E_PRESET_SLOT".into(), 21),
    );
    // A declared slot nothing uses (the second slot, line 11).
    assert_eq!(
        error(&OUTPUT_SLOT.replace(r#"target = "{out}""#, r#"target = "default""#)),
        ("E_PRESET_SLOT".into(), 11),
    );
    // The name uses a slot that does not exist.
    assert_eq!(
        error(&OUTPUT_SLOT.replace("{mic} on {out}", "{mic} on {speaker}")),
        ("E_PRESET_SLOT".into(), 17),
    );
    // Config rules still apply, at their place in the preset (line 20: id).
    assert_eq!(
        error(&OUTPUT_SLOT.replace(
            r#"id = "mic"
match"#,
            r#"id = "Mic"
match"#
        )),
        ("E_CONFIG_ID_FORMAT".into(), 20),
    );
    // Shape errors in the header are located; in the Junction they point
    // at [junction].
    assert_eq!(
        error(&OUTPUT_SLOT.replace("prompt = \"Pick a mic\"", "promt = \"Pick a mic\"")),
        ("E_CONFIG_UNKNOWN_KEY".into(), 9),
    );
    assert_eq!(
        error(&OUTPUT_SLOT.replace("name = \"{mic} on {out}\"", "nam = \"x\"")),
        ("E_CONFIG_UNKNOWN_KEY".into(), 16),
    );
    // A slot in a plain config is still refused.
    let cfg = "schema = 1\n[[junction]]\nid = \"a\"\nname = \"A\"\n[[junction.input]]\nid = \"m\"\nmatch = { slot = \"mic\" }\n";
    let d = config::load(cfg, "c.toml").expect_err("slot in config");
    assert_eq!(d[0].code, "E_CONFIG_SLOT_OUTSIDE_PRESET");
}

#[test]
fn a_name_with_braces_exports_and_imports_as_text() {
    let cfg = config::load(
        "schema = 1\n[[junction]]\nid = \"a\"\nname = \"Mix {A}\"\n",
        "t.toml",
    )
    .expect("config")
    .config;
    let text = preset::Preset::from_junction(&cfg.junction[0]).to_toml();
    let p = preset::load(&text, "p.toml").expect("valid").preset;
    assert_eq!(p.fill(&BTreeMap::new()).expect("fill").name, "Mix {A}");
}
