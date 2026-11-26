// SPDX-License-Identifier: GPL-3.0-or-later

//! The onboarding system check (design board 8, UX onboarding): the rows
//! of `junc doctor --json` that say whether JUNC can work here. The GUI
//! runs the CLI rather than D-Bus: `doctor` works without juncd and never
//! starts it (architecture §5.2), and `junc-gui` depends on `junc`.

use std::path::PathBuf;
use std::process::Command;

/// The checks onboarding shows, in the board's order: the audio server,
/// its session manager and the two compatibility layers through which
/// PulseAudio and JACK apps see Junctions.
pub const ONBOARDING: [&str; 4] = ["pipewire", "wireplumber", "pipewire-pulse", "jack"];

/// One row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The doctor check's id.
    pub id: String,
    /// `ok`, `mismatch` (doctor's `warn`), `missing` or `down`.
    pub state: &'static str,
    /// The command that fixes it, if doctor knows one.
    pub fix: String,
}

/// The onboarding rows from doctor's JSON (the `junc` envelope or the bare
/// list of checks). Several findings for one id keep the worst.
pub fn parse(json: &str) -> Result<Vec<Row>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let list = v
        .pointer("/data/checks")
        .unwrap_or(&v)
        .as_array()
        .ok_or("no checks in doctor's answer")?;
    let rank = |s: &str| match s {
        "ok" => 0,
        "mismatch" => 1,
        "down" => 2,
        _ => 3,
    };
    let mut rows: Vec<Row> = Vec::new();
    for id in ONBOARDING {
        for c in list.iter().filter(|c| c["id"] == id) {
            let state = match c["status"].as_str().unwrap_or("") {
                "ok" => "ok",
                "warn" => "mismatch",
                "down" => "down",
                _ => "missing",
            };
            let row = Row {
                id: id.to_owned(),
                state,
                fix: c["fix"].as_str().unwrap_or("").to_owned(),
            };
            match rows.iter_mut().find(|r| r.id == id) {
                Some(r) if rank(state) > rank(r.state) => *r = row,
                Some(_) => {}
                None => rows.push(row),
            }
        }
    }
    Ok(rows)
}

/// Whether onboarding may go on: nothing missing or stopped.
pub fn passed(rows: &[Row]) -> bool {
    !rows.is_empty() && rows.iter().all(|r| matches!(r.state, "ok" | "mismatch"))
}

/// The `junc` next to this program (a build tree, a package's
/// `/usr/bin`), else the one on `PATH`.
fn junc() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("junc")))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("junc"))
}

/// Runs `junc doctor --json` and returns the onboarding rows. Blocks for
/// as long as doctor takes (it asks pacman and PipeWire): call it off the
/// GUI thread.
pub fn run() -> Result<Vec<Row>, String> {
    let out = Command::new(junc())
        .args(["doctor", "--json"])
        .output()
        .map_err(|e| format!("junc doctor: {e}"))?;
    parse(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn golden(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/../../tests/golden/doctor/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("golden")
    }

    #[test]
    fn a_healthy_system_passes_with_four_rows() {
        let rows = parse(&golden("healthy")).expect("parse");
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ONBOARDING);
        assert!(passed(&rows));
    }

    #[test]
    fn missing_or_stopped_pieces_hold_onboarding_with_their_fix() {
        let rows = parse(&golden("broken")).expect("parse");
        assert!(!passed(&rows));
        assert!(
            rows.iter()
                .any(|r| matches!(r.state, "missing" | "down") && !r.fix.is_empty()),
            "{rows:?}"
        );
        let envelope = r#"{"junc":1,"ok":true,"data":{"checks":[
            {"id":"pipewire","status":"ok","detail":"running"},
            {"id":"jack","status":"warn","detail":"jack2","fix":"sudo pacman -S pipewire-jack"},
            {"id":"jack","status":"ok","detail":"x"},
            {"id":"juncd","status":"down","detail":"not running"}]}}"#;
        let rows = parse(envelope).expect("envelope");
        assert_eq!(rows.len(), 2, "only onboarding's checks");
        assert_eq!(
            (rows[1].state, rows[1].fix.as_str()),
            ("mismatch", "sudo pacman -S pipewire-jack"),
            "the worst finding wins"
        );
        assert!(passed(&rows), "a warning does not hold onboarding");
        assert!(!passed(&[]));
        assert!(parse("not json").is_err());
    }
}
