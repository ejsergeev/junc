// SPDX-License-Identifier: GPL-3.0-or-later

//! Config loader against the fixtures in `tests/fixtures/configs/`.
//!
//! `valid/*.toml` must load without errors. Each `invalid/*.toml` starts with
//! `# expect: CODE [LINE:COL]` and must fail with that code (at that position).

use junc_core::config::{self, Flavor};
use junc_core::diag::Diagnostic;
use junc_core::model::{Kind, MatchKind, Rate};
use std::fs;
use std::path::{Path, PathBuf};

fn fixtures(sub: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/configs")
        .join(sub);
    let mut files: Vec<_> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()
        .unwrap_or_default();
    files.retain(|p| p.extension().is_some_and(|x| x == "toml"));
    files.sort();
    assert!(!files.is_empty(), "no fixtures in {}", dir.display());
    files
}

fn name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn show(diags: &[Diagnostic]) -> String {
    diags
        .iter()
        .map(|d| format!("  {d}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn valid_fixtures_load() {
    for path in fixtures("valid") {
        let src = fs::read_to_string(&path).expect("readable fixture");
        if let Err(d) = config::load(&src, &name(&path)) {
            panic!("{} should load:\n{}", name(&path), show(&d));
        }
    }
}

#[test]
fn invalid_fixtures_fail_where_expected() {
    let mut failures = Vec::new();
    for path in fixtures("invalid") {
        let src = fs::read_to_string(&path).expect("readable fixture");
        let expect = src
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("# expect: "))
            .unwrap_or_else(|| panic!("{} lacks a '# expect:' first line", name(&path)));
        let mut parts = expect.split_whitespace();
        let code = parts.next().expect("expected code");
        let pos = parts.next();
        let diags = match config::load(&src, &name(&path)) {
            Ok(_) => {
                failures.push(format!("{}: loaded, expected {code}", name(&path)));
                continue;
            }
            Err(d) => d,
        };
        let hit = diags.iter().find(|d| d.code == code);
        match (hit, pos) {
            (None, _) => failures.push(format!(
                "{}: expected {code}, got\n{}",
                name(&path),
                show(&diags)
            )),
            (Some(d), Some(pos)) => {
                let got = d
                    .location
                    .as_ref()
                    .map(|l| format!("{}:{}", l.line, l.column));
                if got.as_deref() != Some(pos) {
                    failures.push(format!(
                        "{}: {code} at {got:?}, expected {pos}",
                        name(&path)
                    ));
                }
                if d.hint.is_none() {
                    failures.push(format!("{}: {code} has no hint", name(&path)));
                }
            }
            (Some(_), None) => {}
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn full_example_reads_as_documented() {
    let src = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/configs/valid/full.toml"),
    )
    .expect("fixture");
    let cfg = config::load(&src, "full.toml").expect("loads").config;
    assert_eq!(cfg.junction.len(), 2);
    let zoom = &cfg.junction[0];
    assert_eq!(zoom.bus.channels, 4);
    assert_eq!(zoom.rate, Rate::default());
    assert_eq!(zoom.input[0].volume, 70);
    assert!(zoom.input[0].exclusive);
    assert_eq!(zoom.input[1].volume, 100, "volume defaults to 100");
    assert_eq!(zoom.input[2].suppressed, vec![[1, 2]]);
    assert_eq!(zoom.input[2].matcher.kinds(), vec![MatchKind::Device]);
    let cable = &cfg.junction[1];
    assert_eq!(cable.kind, Kind::Cable);
    assert_eq!(
        cable.cable.as_ref().map(|c| c.to.binary.as_deref()),
        Some(Some("obs"))
    );
}

#[test]
fn unknown_key_suggests_the_right_one() {
    let src = "schema = 1\n[[junction]]\nid = \"z\"\nname = \"Z\"\nvolum = 80\n";
    let d = config::load(src, "j.toml").expect_err("unknown key");
    let hint = d[0].hint.clone().unwrap_or_default();
    assert!(hint.starts_with("Did you mean 'volume'?"), "{hint}");
}

#[test]
fn v0_migrates_and_keeps_comments() {
    let src = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/configs/migrate/v0.toml"),
    )
    .expect("fixture");
    let loaded = config::load(&src, "v0.toml").expect("migrates");
    assert_eq!(loaded.migrated_from, Some(0));
    assert_eq!(loaded.config.schema, 1);
    let out = loaded.document.to_string();
    assert!(out.contains("# keep this comment"), "{out}");
    assert!(
        out.find("schema = 1").unwrap_or(usize::MAX) < out.find("[[junction]]").unwrap_or(0),
        "{out}"
    );
}

#[test]
fn slots_are_allowed_in_presets() {
    let src = "schema = 1\n[[junction]]\nid = \"t\"\nname = \"T\"\n[[junction.input]]\nid = \"mic\"\nmatch = { slot = \"mic\" }\n";
    assert!(config::load_as(src, "t.toml", Flavor::Preset).is_ok());
}

#[test]
fn round_trip_through_to_toml() {
    let src = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/configs/valid/full.toml"),
    )
    .expect("fixture");
    let cfg = config::load(&src, "full.toml").expect("loads").config;
    let again = config::load(&config::to_toml(&cfg), "out.toml")
        .expect("reloads")
        .config;
    assert_eq!(cfg, again);
}

#[test]
fn slugify_makes_valid_ids() {
    assert_eq!(
        config::slugify("Virtual Mic for Zoom"),
        "virtual-mic-for-zoom"
    );
    assert_eq!(config::slugify("Firefox → OBS"), "firefox-obs");
    assert_eq!(config::slugify("Стрим"), "junction");
}

/// `cargo test -p junc-core --test config_fixtures -- --ignored --nocapture`
/// prints every invalid fixture's diagnostics, to review their wording.
#[test]
#[ignore = "prints diagnostics for review"]
fn print_invalid_fixture_messages() {
    for path in fixtures("invalid") {
        let src = fs::read_to_string(&path).expect("readable fixture");
        if let Err(d) = config::load(&src, &name(&path)) {
            println!("{}", show(&d));
        }
    }
}

#[test]
fn published_schema_is_current() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/junc-config-v1.json");
    let on_disk = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        on_disk == junc_core::schema::json_schema(),
        "{} is stale: run `cargo xtask schema` and commit the result",
        path.display()
    );
}

#[test]
fn the_same_file_always_reports_the_same_loop() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/configs/invalid/cycle.toml");
    let src = fs::read_to_string(&path).expect("fixture");
    // Every load builds fresh hash maps with new seeds; the report must not
    // depend on them.
    for _ in 0..32 {
        let d = config::load(&src, "cycle.toml").expect_err("a loop");
        assert_eq!(d[0].message, "nesting loop: a → b → a");
    }
}
