// SPDX-License-Identifier: GPL-3.0-or-later

//! Screenshot goldens (architecture §11.3): each scenario renders
//! offscreen with the bundled font (`--snapshot`) and must match
//! `tests/golden/gui/<scenario>.png`. A small share of pixels may differ
//! slightly (anti-aliasing between architectures); a changed layout or
//! colour does not pass. `JUNC_BLESS=1` rewrites the goldens, which are
//! committed on their own.

use junc_gui::snapshot;
use std::path::PathBuf;
use std::process::Command;

/// Pixels may differ by up to this much per channel…
const THRESHOLD: u8 = 24;
/// …in up to this share of the image.
const MAX_SHARE: f64 = 0.002;

fn golden_dir() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/golden/gui"
    ))
}

fn check(scenario: &str, extra: &[&str]) {
    let out = std::env::temp_dir().join(format!("junc-snap-{scenario}-{}.png", std::process::id()));
    let status = Command::new(env!("CARGO_BIN_EXE_junc-gui"))
        .args(["--scenario", scenario, "--snapshot"])
        .arg(&out)
        .args(extra)
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .status()
        .expect("junc-gui");
    assert!(status.success(), "{scenario}: junc-gui failed");
    let golden = golden_dir().join(format!("{scenario}.png"));
    if std::env::var_os("JUNC_BLESS").is_some() {
        std::fs::create_dir_all(golden_dir()).expect("mkdir");
        std::fs::copy(&out, &golden).expect("bless");
    }
    let (w, h, got) = snapshot::read_png(&out).expect("render");
    let (gw, gh, want) = snapshot::read_png(&golden).unwrap_or_else(|e| {
        panic!("{e} (JUNC_BLESS=1 cargo test -p junc-gui --features scenarios --test snapshots)")
    });
    assert_eq!((w, h), (gw, gh), "{scenario}: size changed");
    let share = snapshot::difference(&got, &want, THRESHOLD);
    let kept = std::env::temp_dir().join(format!("junc-snap-{scenario}-failed.png"));
    if share > MAX_SHARE {
        let _ = std::fs::copy(&out, &kept);
    }
    let _ = std::fs::remove_file(&out);
    assert!(
        share <= MAX_SHARE,
        "{scenario}: {:.3} % of pixels differ from {} (render kept at {})",
        share * 100.0,
        golden.display(),
        kept.display()
    );
}

#[test]
fn the_default_scenario_matches_its_golden() {
    check("default", &[]);
}

#[test]
fn the_component_gallery_matches_its_golden() {
    check("gallery", &[]);
}

#[test]
fn renders_are_deterministic() {
    let render = |n: u32| {
        let out =
            std::env::temp_dir().join(format!("junc-snap-det-{n}-{}.png", std::process::id()));
        let ok = Command::new(env!("CARGO_BIN_EXE_junc-gui"))
            .args([
                "--scenario",
                "default",
                "--theme",
                "light",
                "--size",
                "900x600",
                "--snapshot",
            ])
            .arg(&out)
            .status()
            .expect("junc-gui")
            .success();
        assert!(ok);
        let (w, h, px) = snapshot::read_png(&out).expect("png");
        let _ = std::fs::remove_file(&out);
        assert_eq!((w, h), (900, 600), "--size");
        px
    };
    assert_eq!(
        render(1),
        render(2),
        "the same scenario renders the same pixels"
    );
}

#[test]
fn a_real_change_fails_the_comparison() {
    // The golden is Tokyo Night; the same scenario in Flexoki Light must
    // be far outside the tolerance.
    let out = std::env::temp_dir().join(format!("junc-snap-neg-{}.png", std::process::id()));
    let ok = Command::new(env!("CARGO_BIN_EXE_junc-gui"))
        .args(["--scenario", "default", "--theme", "light", "--snapshot"])
        .arg(&out)
        .status()
        .expect("junc-gui")
        .success();
    assert!(ok);
    let (_, _, light) = snapshot::read_png(&out).expect("png");
    let _ = std::fs::remove_file(&out);
    let (_, _, golden) = snapshot::read_png(&golden_dir().join("default.png")).expect("golden");
    assert!(snapshot::difference(&light, &golden, THRESHOLD) > 0.5);
}
