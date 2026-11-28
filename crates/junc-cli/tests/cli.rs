// SPDX-License-Identifier: GPL-3.0-or-later

//! `junc` end to end against juncd on a private bus. Without PipeWire (an
//! empty runtime dir) juncd is the "mock daemon" of the plan: the real
//! config handling and D-Bus API, no audio graph. With
//! `JUNC_LIVE_PIPEWIRE=1` (scripts/ci/live-pipewire.sh) the smoke test runs
//! against the isolated PipeWire.
//!
//! juncd comes from the same target directory; `cargo test --workspace`
//! builds it.

use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const CONFIG: &str = r#"# My setup; keep this note.
schema = 1

[[junction]]
id = "obs"
name = "OBS Mix" # streaming
"#;

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Env {
    dir: PathBuf,
    address: String,
    _bus: Kill,
    _daemon: Option<Kill>,
}

fn bin(name: &str) -> PathBuf {
    let junc = PathBuf::from(env!("CARGO_BIN_EXE_junc"));
    let path = junc.with_file_name(name);
    assert!(
        path.exists(),
        "{} is missing; run `cargo build -p juncd` (cargo test --workspace does)",
        path.display()
    );
    path
}

fn private_bus() -> Option<(Kill, String)> {
    let mut child = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--nopidfile", "--print-address=1"])
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let mut line = String::new();
    BufReader::new(child.stdout.take()?)
        .read_line(&mut line)
        .ok()?;
    Some((Kill(child), line.trim().to_owned()))
}

/// A private bus, and juncd on it unless `daemon` is false.
fn start(tag: &str, daemon: bool, live: bool) -> Option<Env> {
    let Some((bus, address)) = private_bus() else {
        assert!(
            std::env::var_os("CI").is_none(),
            "dbus-daemon is missing in CI"
        );
        eprintln!("skipped: no dbus-daemon");
        return None;
    };
    let dir = std::env::temp_dir().join(format!("junc-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("run")).expect("tmp dir");
    std::fs::write(dir.join("junctions.toml"), CONFIG).expect("config");
    let daemon = daemon.then(|| {
        let mut c = Command::new(bin("juncd"));
        c.arg("--config")
            .arg(dir.join("junctions.toml"))
            .arg("--state-dir")
            .arg(&dir)
            .env("DBUS_SESSION_BUS_ADDRESS", &address)
            .stderr(std::fs::File::create(dir.join("juncd.log")).expect("log"));
        if !live {
            c.env("PIPEWIRE_RUNTIME_DIR", dir.join("run"))
                .env_remove("PIPEWIRE_REMOTE");
        }
        Kill(c.spawn().expect("juncd"))
    });
    let env = Env {
        dir,
        address,
        _bus: bus,
        _daemon: daemon,
    };
    if env._daemon.is_some() {
        wait(|| env.junc(&["status"]).code == 0);
    }
    Some(env)
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Out {
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| panic!("{e}: {}", self.stdout))
    }
}

impl Env {
    fn junc(&self, args: &[&str]) -> Out {
        let out = Command::new(env!("CARGO_BIN_EXE_junc"))
            .args(args)
            .env("DBUS_SESSION_BUS_ADDRESS", &self.address)
            .stdin(Stdio::null())
            .output()
            .expect("junc");
        Out {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    /// Runs `junc`, expects success, returns stdout.
    fn ok(&self, args: &[&str]) -> String {
        let out = self.junc(args);
        assert_eq!(out.code, 0, "junc {args:?}: {}{}", out.stdout, out.stderr);
        out.stdout
    }

    fn status(&self, name: &str) -> Value {
        let doc = self.junc(&["--json", "status", name]).json();
        assert_eq!(doc["ok"], true, "{doc}");
        doc["data"]["junction"].clone()
    }

    fn file(&self) -> String {
        std::fs::read_to_string(self.dir.join("junctions.toml")).expect("config file")
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn wait(mut f: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < until, "timed out");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn commands_change_the_config_and_keep_comments() {
    let Some(e) = start("cmds", true, false) else {
        return;
    };

    assert!(
        e.ok(&["create", "junction", "Zoom Mic"])
            .contains("(zoom-mic)")
    );
    e.ok(&[
        "input",
        "add",
        "zoom-mic",
        "--device",
        "alsa_input.usb-mic",
        "--id",
        "mic",
    ]);
    e.ok(&[
        "input",
        "add",
        "Zoom Mic",
        "--app",
        "spotify",
        "--exclusive",
    ]);
    e.ok(&[
        "input",
        "add",
        "zoom-mic",
        "--monitor",
        "alsa_output.speakers",
    ]);
    e.ok(&["input", "add", "OBS Mix", "--junction", "Zoom Mic"]);
    e.ok(&["listen", "add", "zoom-mic", "default", "--id", "hp"]);
    e.ok(&["volume", "zoom-mic", "80"]);
    e.ok(&["volume", "zoom-mic", "spotify", "70%"]);
    e.ok(&["volume", "zoom-mic", "--listen", "hp", "50"]);
    e.ok(&["bus", "add", "zoom-mic", "--count", "2"]);
    e.ok(&["bus", "remove", "zoom-mic", "6"]);
    e.ok(&["link", "zoom-mic", "mic:1", "3"]);
    e.ok(&["link", "zoom-mic", "5", "hp:2"]);
    e.ok(&["unlink", "zoom-mic", "5", "hp:2"]);
    e.ok(&["input", "disable", "zoom-mic", "alsa-output-speakers"]);
    e.ok(&["switch", "zoom-mic"]);

    let j = e.status("zoom-mic");
    assert_eq!(j["name"], "Zoom Mic");
    assert_eq!(j["volume"], 80);
    assert_eq!(j["channels"], 5);
    assert_eq!(j["default"], "input");
    assert_eq!(j["enabled"], true);
    assert_eq!(j["inputs"][0]["links"], serde_json::json!([[1, 3]]));
    assert_eq!(j["inputs"][1]["volume"], 70);
    assert_eq!(j["inputs"][1]["exclusive"], true);
    assert_eq!(j["inputs"][2]["enabled"], false);
    assert_eq!(j["listens"][0]["volume"], 50);
    assert_eq!(j["listens"][0]["custom_links"], true);
    assert_eq!(
        e.status("obs")["inputs"][0]["match"]["junction"],
        "zoom-mic"
    );

    e.ok(&["default", "zoom-mic", "--clear"]);
    e.ok(&["toggle", "zoom-mic"]);
    assert_eq!(e.status("zoom-mic")["enabled"], false);
    e.ok(&["rename", "zoom-mic", "Meetings"]);
    assert!(e.ok(&["duplicate", "meetings"]).contains("meetings-copy"));
    e.ok(&["create", "cable", "spotify", "obs"]);
    e.ok(&["delete", "meetings-copy", "--yes"]);
    e.ok(&["input", "remove", "obs", "zoom-mic"]);

    let list = e.junc(&["--json", "list"]).json();
    let ids: Vec<&str> = list["data"]["junctions"]
        .as_array()
        .expect("junctions")
        .iter()
        .filter_map(|j| j["id"].as_str())
        .collect();
    assert_eq!(ids, ["obs", "zoom-mic", "spotify-to-obs"]);
    assert!(list["revision"].as_u64().expect("revision") > 20);

    let text = e.ok(&["list"]);
    assert!(text.starts_with("NAME"), "{text}");
    assert!(text.contains("Meetings"), "{text}");

    let file = e.file();
    for want in [
        "# My setup; keep this note.",
        "name = \"OBS Mix\" # streaming",
        "name = \"Meetings\"",
        "match = { binary = \"spotify\" }",
        "cable = { to = { binary = \"obs\" } }",
    ] {
        assert!(file.contains(want), "missing {want:?} in:\n{file}");
    }
    assert!(
        !file.contains("rate = "),
        "defaults are not written:\n{file}"
    );
}

#[test]
fn failures_have_codes_hints_and_exit_codes() {
    let Some(e) = start("errors", true, false) else {
        return;
    };

    let out = e.junc(&["enable", "obs-mixx"]);
    assert_eq!(out.code, 4, "{}", out.stderr);
    assert!(
        out.stderr.contains("Did you mean 'obs'?")
            || out.stderr.contains("Did you mean 'OBS Mix'?"),
        "{}",
        out.stderr
    );

    let doc = e.junc(&["--json", "volume", "obs", "900"]);
    assert_eq!(doc.code, 3);
    let v = doc.json();
    assert_eq!(v["ok"], false);
    assert_eq!(v["junc"], 1);
    assert_eq!(v["error"]["code"], "E_CONFIG_RANGE");
    assert!(
        v["error"]["hint"]
            .as_str()
            .is_some_and(|h| h.contains("150"))
    );
    assert!(v["error"]["location"]["line"].as_u64().is_some(), "{v}");

    let out = e.junc(&["delete", "obs"]);
    assert_eq!(out.code, 8, "stdin is not a terminal: {}", out.stderr);
    assert!(out.stderr.contains("--yes"));

    assert_eq!(e.junc(&["link", "obs", "1", "2"]).code, 2);
    assert_eq!(e.junc(&["frobnicate"]).code, 2);
    let usage = e.junc(&["--json", "frobnicate"]);
    assert_eq!(
        (usage.code, usage.json()["error"]["code"].as_str()),
        (2, Some("E_USAGE"))
    );

    let ok = e.junc(&["--json", "disable", "obs"]).json();
    assert_eq!(ok["ok"], true);
    assert_eq!(
        ok["data"],
        serde_json::json!({ "id": "obs", "enabled": false })
    );
    assert!(e.junc(&["--quiet", "enable", "obs"]).stdout.is_empty());

    // No daemon on the bus.
    let Some(none) = start("nodaemon", false, false) else {
        return;
    };
    let out = none.junc(&["--json", "list"]);
    assert_eq!(out.code, 6);
    assert_eq!(out.json()["error"]["code"], "E_DAEMON_UNAVAILABLE");
}

/// `junc enable X` stays fast. The 50 ms target of A10/A11 is gated on the
/// release build by junc-bench (scripts/ci/bench.sh); this debug build runs
/// next to the other tests on a shared runner, so it only catches gross
/// regressions. A shared runner can stall a whole round, so a slow round is
/// measured once more before the test fails; the gate stays the same.
#[test]
fn enable_stays_fast() {
    let Some(e) = start("time", true, false) else {
        return;
    };
    let mut p95 = Duration::MAX;
    for round in 1..=2 {
        // Forty runs, so p95 does not hinge on one slow start.
        let mut times: Vec<Duration> = (0..40)
            .map(|i| {
                let t = Instant::now();
                e.ok(&[if i % 2 == 0 { "disable" } else { "enable" }, "obs"]);
                t.elapsed()
            })
            .collect();
        times.sort();
        p95 = times[(times.len() * 95).div_ceil(100) - 1];
        eprintln!(
            "junc enable, round {round}: p50 {:?}, p95 {p95:?}",
            times[times.len() / 2]
        );
        if p95 < Duration::from_millis(250) {
            return;
        }
    }
    panic!("p95 {p95:?} in both rounds");
}

/// Hardware layer 2 smoke test: the CLI routes a device on the isolated
/// PipeWire.
#[test]
fn live_smoke() {
    if std::env::var("JUNC_LIVE_PIPEWIRE").as_deref() != Ok("1") {
        eprintln!("skipped: run scripts/ci/live-pipewire.sh");
        return;
    }
    let Some(e) = start("live", true, true) else {
        return;
    };
    e.ok(&["create", "junction", "Live Mic"]);
    e.ok(&[
        "input",
        "add",
        "live-mic",
        "--device",
        "fake.hwin",
        "--id",
        "hw",
    ]);
    e.ok(&["listen", "add", "live-mic", "default"]);
    wait(|| {
        let j = e.status("live-mic");
        j["status"] == "active"
            && j["inputs"][0]["status"] == "active"
            && j["inputs"][0]["links"] == serde_json::json!([[1, 1], [1, 2]])
            && j["listens"][0]["status"] == "active"
    });
    assert!(e.ok(&["status", "live-mic"]).contains("device fake.hwin"));
    e.ok(&["disable", "live-mic"]);
    wait(|| e.status("live-mic")["inputs"][0]["status"] == "disabled");
    e.ok(&["delete", "live-mic", "--yes"]);
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Compares `json` with tests/golden/<path>; `JUNC_BLESS=1` rewrites it.
fn golden(path: &str, json: &str) {
    let file = repo().join("tests/golden").join(path);
    if std::env::var_os("JUNC_BLESS").is_some() {
        std::fs::create_dir_all(file.parent().expect("dir")).expect("mkdir");
        std::fs::write(&file, json).expect("write golden");
    }
    let want = std::fs::read_to_string(&file).unwrap_or_default();
    assert!(
        want == json,
        "{} differs (JUNC_BLESS=1 cargo test -p junc-cli --test cli rewrites it):\n{json}",
        file.display()
    );
}

/// Runs junc in the repository root without a bus (offline commands).
fn offline(args: &[&str]) -> Out {
    let out = Command::new(env!("CARGO_BIN_EXE_junc"))
        .args(args)
        .current_dir(repo())
        .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent")
        .output()
        .expect("junc");
    Out {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

#[test]
fn schema_is_the_published_one() {
    let out = offline(&["schema"]);
    assert_eq!(out.code, 0);
    let published =
        std::fs::read_to_string(repo().join("schema/junc-config-v1.json")).expect("schema");
    assert_eq!(out.stdout.trim_end(), published.trim_end());
    let doc = offline(&["--json", "schema"]).json();
    assert_eq!(doc["data"]["title"], "JUNC configuration");
    assert_eq!(offline(&["schema", "--version", "2"]).code, 2);
}

#[test]
fn validation_results_match_the_golden_files() {
    let dir = repo().join("tests/fixtures/configs/invalid");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("fixtures")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".toml"))
        .collect();
    names.sort();
    for name in names {
        let rel = format!("tests/fixtures/configs/invalid/{name}");
        let out = offline(&["--json", "config", "validate", &rel]);
        assert_eq!(out.code, 3, "{rel}: {}", out.stdout);
        golden(
            &format!("validate/{}.json", name.trim_end_matches(".toml")),
            &out.stdout,
        );
    }
    for name in ["minimal", "full", "nested"] {
        let rel = format!("tests/fixtures/configs/valid/{name}.toml");
        let out = offline(&["--json", "config", "validate", &rel]);
        assert_eq!(out.code, 0, "{rel}: {}", out.stdout);
        assert_eq!(out.json()["data"]["valid"], true);
    }
}

#[test]
fn describe_matches_the_golden_file_and_apply_checks_revisions() {
    let Some(e) = start("describe", true, false) else {
        return;
    };
    let full = std::fs::read_to_string(repo().join("tests/fixtures/configs/valid/full.toml"))
        .expect("fixture");
    let file = e.dir.join("full.toml");
    std::fs::write(&file, &full).expect("write");
    let path = file.to_string_lossy().into_owned();

    // A stale revision is refused; the right one applies the text as given.
    let out = e.junc(&["--json", "apply", &path, "--expect-revision", "99"]);
    assert_eq!(out.code, 5, "{}", out.stdout);
    let doc = e
        .junc(&["--json", "apply", &path, "--expect-revision", "1"])
        .json();
    assert_eq!(doc["revision"], 2, "{doc}");
    assert_eq!(doc["data"]["applied"], true);
    assert!(
        doc["data"]["plan"]["summary"][0]
            .as_str()
            .is_some_and(|s| s.starts_with("Delete “OBS Mix”")),
        "{doc}"
    );
    assert_eq!(e.file(), full);

    // No PipeWire: a dry run cannot plan, and says so.
    let out = e.junc(&["--json", "apply", &path, "--dry-run"]);
    assert_eq!(out.code, 7, "{}", out.stdout);

    let mut d = e.junc(&["--json", "describe"]).json();
    d["data"]["config_file"] = "<config>".into();
    d["data"]["daemon_version"] = "<version>".into();
    golden(
        "describe/full.json",
        &(serde_json::to_string_pretty(&d).expect("json") + "\n"),
    );
}

/// Hardware layer 2: a dry run changes neither the graph nor the config;
/// doctor sees the isolated session as healthy.
#[test]
fn live_dry_run_changes_nothing_and_doctor_is_happy() {
    if std::env::var("JUNC_LIVE_PIPEWIRE").as_deref() != Ok("1") {
        eprintln!("skipped: run scripts/ci/live-pipewire.sh");
        return;
    }
    let Some(e) = start("dryrun", true, true) else {
        return;
    };
    let graph = || {
        let out = Command::new("pw-dump").output().expect("pw-dump");
        let g = junc_core::graph::Graph::from_pw_dump(&String::from_utf8_lossy(&out.stdout))
            .expect("graph");
        let mut nodes: Vec<String> = g
            .nodes
            .values()
            .filter(|n| n.is_junc())
            .map(|n| n.name().to_owned())
            .collect();
        let mut links: Vec<String> = g
            .links
            .values()
            .filter_map(|l| l.props.get("junc.ref").map(str::to_owned))
            .collect();
        nodes.sort();
        links.sort();
        (nodes, links)
    };
    e.ok(&["create", "junction", "Base"]);
    // Wait until juncd has finished routing: two equal snapshots in a row.
    let mut last = graph();
    wait(|| {
        std::thread::sleep(Duration::from_millis(300));
        let now = graph();
        let settled = now == last && now.1.iter().filter(|r| r.starts_with("sink/")).count() == 4;
        last = now;
        settled
    });
    let before = (graph(), e.file());

    let proposed = e.dir.join("proposed.toml");
    std::fs::write(
        &proposed,
        format!(
            "{}\n[[junction.input]]\nid = \"hw\"\nmatch = {{ device = \"fake.hwin\" }}\n\n\
             [[junction]]\nid = \"extra\"\nname = \"Extra\"\n",
            e.file().trim_end()
        ),
    )
    .expect("write");
    let doc = e
        .junc(&["--json", "apply", &proposed.to_string_lossy(), "--dry-run"])
        .json();
    assert_eq!(doc["ok"], true, "{doc}");
    let summary: Vec<&str> = doc["data"]["plan"]["summary"]
        .as_array()
        .expect("summary")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        summary
            .iter()
            .any(|s| s.starts_with("Add ") && s.ends_with("to “Base”")),
        "{summary:?}"
    );
    assert!(
        summary.contains(&"Create Junction “Extra” (2 channels)"),
        "{summary:?}"
    );
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!((graph(), e.file()), before, "a dry run changed something");

    let checks = e.junc(&["--json", "doctor"]).json();
    for c in checks["data"]["checks"].as_array().expect("checks") {
        if c["id"] != "jack" {
            assert_eq!(c["status"], "ok", "{c}");
        }
    }
}

/// F2: a nesting loop is refused with E_CONFIG_CYCLE before anything
/// changes, whichever way it arrives.
#[test]
fn nesting_loops_are_refused_before_any_change() {
    let Some(e) = start("cycle", true, false) else {
        return;
    };
    e.ok(&["create", "junction", "A"]);
    e.ok(&["create", "junction", "B"]);
    e.ok(&["input", "add", "b", "--junction", "a"]);
    let before = (
        e.file(),
        e.junc(&["--json", "status"]).json()["revision"].clone(),
    );

    // Through an edit command.
    let out = e.junc(&["--json", "input", "add", "a", "--junction", "b"]);
    assert_eq!(out.code, 3, "{}", out.stdout);
    let v = out.json();
    assert_eq!(v["error"]["code"], "E_CONFIG_CYCLE");
    assert_eq!(v["error"]["message"], "nesting loop: a → b → a");

    // Through apply, as an agent would.
    let fixed = "schema = 1\n\
                 [[junction]]\nid = \"a\"\nname = \"A\"\n\
                 [[junction.input]]\nid = \"b\"\nmatch = { junction = \"b\" }\n\
                 [[junction]]\nid = \"b\"\nname = \"B\"\n\
                 [[junction.input]]\nid = \"a\"\nmatch = { junction = \"a\" }\n";
    let looped = e.dir.join("loop.toml");
    std::fs::write(&looped, fixed).expect("write");
    let out = e.junc(&["--json", "apply", &looped.to_string_lossy()]);
    assert_eq!(out.code, 3, "{}", out.stdout);
    assert_eq!(out.json()["error"]["code"], "E_CONFIG_CYCLE");
    let out = e.junc(&["--json", "apply", &looped.to_string_lossy(), "--dry-run"]);
    assert_eq!(out.code, 3, "a dry run validates first: {}", out.stdout);

    assert_eq!(
        (
            e.file(),
            e.junc(&["--json", "status"]).json()["revision"].clone()
        ),
        before,
        "nothing changed"
    );

    // Through a hand edit: reported, and the last good config stays in use.
    std::fs::write(e.dir.join("junctions.toml"), fixed).expect("hand edit");
    wait(|| {
        let d = e.junc(&["--json", "status"]).json();
        d["data"]["config_error"][0]["code"] == "E_CONFIG_CYCLE"
    });
    let a = e.status("a");
    assert!(a["inputs"].as_array().is_none_or(Vec::is_empty), "{a}");
}

/// F9: `junc service` drives `systemctl --user` on juncd.service (a fake
/// systemctl here, recording its calls).
#[test]
fn service_commands_drive_systemctl() {
    let dir = std::env::temp_dir().join(format!("junc-cli-service-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let fake = dir.join("systemctl");
    std::fs::write(
        &fake,
        r#"#!/bin/sh
echo "$*" >> "$JUNC_TEST_DIR/calls"
state=$(cat "$JUNC_TEST_DIR/state" 2>/dev/null || echo disabled)
case "$2" in
  is-enabled) echo "$state"; [ "$state" = enabled ];;
  is-active) if [ "$state" = enabled ]; then echo active; else echo inactive; exit 3; fi;;
  enable) [ -e "$JUNC_TEST_DIR/missing" ] && { echo "Failed to enable unit: Unit file juncd.service does not exist." >&2; exit 1; }
          echo enabled > "$JUNC_TEST_DIR/state";;
  disable) echo disabled > "$JUNC_TEST_DIR/state";;
esac
"#,
    )
    .expect("fake systemctl");
    Command::new("chmod")
        .arg("755")
        .arg(&fake)
        .status()
        .expect("chmod");
    let path = format!(
        "{}:{}",
        dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let junc = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_junc"))
            .args(args)
            .env("PATH", &path)
            .env("JUNC_TEST_DIR", &dir)
            .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent")
            .output()
            .expect("junc");
        Out {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    };

    let st = junc(&["--json", "service", "status"]).json();
    assert_eq!(st["data"]["enabled"], false, "{st}");
    assert_eq!(st["data"]["active"], "inactive");

    let on = junc(&["--json", "service", "enable"]);
    assert_eq!(on.code, 0, "{}{}", on.stdout, on.stderr);
    let v = on.json();
    assert_eq!(
        (v["data"]["enabled"].clone(), v["data"]["active"].clone()),
        (true.into(), "active".into())
    );
    assert!(
        junc(&["service", "status"])
            .stdout
            .contains("starts at login")
    );

    let off = junc(&["--json", "service", "disable"]).json();
    assert_eq!(off["data"]["enabled"], false);

    let calls = std::fs::read_to_string(dir.join("calls")).expect("calls");
    assert!(
        calls.contains("--user enable --now juncd.service"),
        "{calls}"
    );
    assert!(
        calls.contains("--user disable --now juncd.service"),
        "{calls}"
    );

    std::fs::write(dir.join("missing"), "").expect("flag");
    let out = junc(&["--json", "service", "enable"]);
    assert_eq!(out.code, 4, "{}", out.stdout);
    assert_eq!(
        out.json()["error"]["hint"],
        "Install the junc package (it ships the user unit)."
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A process that plays or records with app properties, for import scans.
fn pw_cat_app(mode: &str, props: &str) -> Kill {
    Kill(
        Command::new("pw-cat")
            .args([mode, "--raw", "--format", "s16", "-P", props])
            .arg("/dev/zero")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("pw-cat"),
    )
}

/// F13 (B5): null sinks from `pactl load-module` and a `pw-loopback`
/// become Junctions or Cables with what they connect; the old setups stay.
#[test]
fn import_turns_null_sinks_and_loopbacks_into_junctions_and_cables() {
    if std::env::var("JUNC_LIVE_PIPEWIRE").as_deref() != Ok("1") {
        eprintln!("skipped: run scripts/ci/live-pipewire.sh");
        return;
    }
    let Some(e) = start("import", true, true) else {
        return;
    };
    let pactl = |args: &[&str]| {
        let out = Command::new("pactl").args(args).output().expect("pactl");
        assert!(out.status.success(), "pactl {args:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    let mix = pactl(&[
        "load-module",
        "module-null-sink",
        "sink_name=discord_mix",
        "sink_properties=device.description=Discord_Mix",
    ]);
    let cab = pactl(&["load-module", "module-null-sink", "sink_name=cab_sink"]);
    let loopback = Kill(
        Command::new("pw-loopback")
            .args(["-n", "mylb", "--capture-props=target.object=fake.hwin"])
            .arg("--playback-props=target.object=fake.hp")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("pw-loopback"),
    );
    let apps = [
        pw_cat_app(
            "-p",
            "{ node.name=imp-ff application.name=Firefox application.process.binary=firefox target.object=discord_mix }",
        ),
        pw_cat_app(
            "-p",
            "{ node.name=imp-sp application.name=Spotify application.process.binary=spotify target.object=cab_sink }",
        ),
        pw_cat_app(
            "-r",
            "{ node.name=imp-obs application.name=OBS application.process.binary=obs target.object=cab_sink stream.capture.sink=true }",
        ),
    ];
    let item = |doc: &Value, id: &str| {
        doc["data"]
            .as_array()
            .and_then(|a| a.iter().find(|i| i["id"] == id).cloned())
    };
    let mut scan = Value::Null;
    wait(|| {
        scan = e.junc(&["--json", "import", "scan"]).json();
        item(&scan, "discord_mix")
            .is_some_and(|i| i["inputs"].as_array().is_some_and(|a| !a.is_empty()))
            && item(&scan, "mylb")
                .is_some_and(|i| !i["listens"].as_array().is_some_and(Vec::is_empty))
            && item(&scan, "cab_sink").is_some_and(|i| i["suggested"] == "cable")
    });
    let mix_item = item(&scan, "discord_mix").expect("discord_mix");
    assert_eq!(mix_item["kind"], "null_sink");
    assert_eq!(mix_item["origin"], "pactl load-module");
    assert_eq!(mix_item["name"], "Discord_Mix");
    assert_eq!(
        mix_item["inputs"],
        serde_json::json!([{ "binary": "firefox" }])
    );
    assert_eq!(mix_item["suggested"], "junction");
    let lb = item(&scan, "mylb").expect("mylb");
    assert_eq!(lb["origin"], "pw-loopback");
    assert_eq!(lb["inputs"], serde_json::json!([{ "device": "fake.hwin" }]));
    assert_eq!(lb["listens"], serde_json::json!(["fake.hp"]));
    let c = item(&scan, "cab_sink").expect("cab_sink");
    assert_eq!(
        c["cable"],
        serde_json::json!({ "from": { "binary": "spotify" }, "to": { "binary": "obs" } })
    );
    let human = e.ok(&["import", "scan"]);
    assert!(
        human.contains("discord_mix") && human.contains("suggested: cable"),
        "{human}"
    );

    // Not one app into another: no Cable.
    let refused = e
        .junc(&["--json", "import", "discord_mix", "--as", "cable"])
        .json();
    assert_eq!(refused["error"]["code"], "E_USAGE", "{refused}");
    let missing = e.junc(&["--json", "import", "nope"]).json();
    assert_eq!(missing["error"]["code"], "E_NOT_FOUND", "{missing}");

    let done = e
        .junc(&["--json", "import", "discord_mix", "mylb", "cab_sink"])
        .json();
    assert_eq!(done["ok"], true, "{done}");
    let ids: Vec<String> = done["data"]["ids"]
        .as_array()
        .expect("ids")
        .iter()
        .map(|v| v.as_str().expect("id").to_owned())
        .collect();
    assert_eq!(ids.len(), 3, "{ids:?}");
    eprintln!("imported as {ids:?}");
    wait(|| {
        let j = e.status(&ids[0]);
        j["inputs"][0]["status"] == "active" && j["channels"] == 2
    });
    wait(|| {
        let j = e.status(&ids[1]);
        j["inputs"][0]["status"] == "active" && j["listens"][0]["status"] == "active"
    });
    let cable = e.status(&ids[2]);
    assert_eq!(cable["kind"], "cable", "{cable}");
    // The old setups still run.
    assert!(pactl(&["list", "short", "modules"]).contains(&mix));

    drop(apps);
    drop(loopback);
    pactl(&["unload-module", &mix]);
    pactl(&["unload-module", &cab]);
    for id in &ids {
        e.ok(&["delete", id, "--yes"]);
    }
}

/// `pw-dump` of the live graph.
fn pw_dump() -> Vec<Value> {
    let out = Command::new("pw-dump").output().expect("pw-dump");
    serde_json::from_slice(&out.stdout).expect("pw-dump json")
}

fn junc_objects(dump: &[Value]) -> usize {
    dump.iter()
        .filter(|o| {
            o["info"]["props"]["junc.managed"] == true
                || o["info"]["props"]["junc.managed"] == "true"
        })
        .count()
}

fn default_key(dump: &[Value], key: &str) -> Option<String> {
    dump.iter()
        .filter(|o| o["props"]["metadata.name"] == "default")
        .flat_map(|o| o["metadata"].as_array().cloned().unwrap_or_default())
        .find(|m| m["key"] == key)
        .and_then(|m| m["value"]["name"].as_str().map(str::to_owned))
}

fn default_source(dump: &[Value]) -> Option<String> {
    default_key(dump, "default.audio.source")
}

/// F20 (B6): `junc teardown` stops and disables juncd, removes every
/// `junc.managed` object (they linger after the daemon stops) and gives the
/// default input back.
#[test]
fn teardown_leaves_no_junc_object() {
    if std::env::var("JUNC_LIVE_PIPEWIRE").as_deref() != Ok("1") {
        eprintln!("skipped: run scripts/ci/live-pipewire.sh");
        return;
    }
    let Some(e) = start("teardown", false, true) else {
        return;
    };
    let state = e.dir.join("state");
    std::fs::create_dir_all(state.join("junc")).expect("state dir");
    let before = default_source(&pw_dump());
    let cfg = e.dir.join("td.toml");
    std::fs::write(
        &cfg,
        "schema = 1\n[[junction]]\nid = \"td\"\nname = \"Teardown\"\ndefault = \"input\"\n\
         [[junction.input]]\nid = \"mic\"\nmatch = { device = \"fake.hwin\" }\n\
         [[junction.listen]]\nid = \"hp\"\ntarget = \"fake.hp\"\n",
    )
    .expect("config");
    let daemon = Kill(
        Command::new(bin("juncd"))
            .args(["--no-dbus", "--config"])
            .arg(&cfg)
            .arg("--state-dir")
            .arg(state.join("junc"))
            .stderr(Stdio::null())
            .spawn()
            .expect("juncd"),
    );
    wait(|| {
        let d = pw_dump();
        junc_objects(&d) >= 6 && default_source(&d).as_deref() == Some("junc.td")
    });
    drop(daemon);
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        junc_objects(&pw_dump()) > 0,
        "JUNC objects linger after juncd stops"
    );

    // A stand-in systemctl, so the test never touches a real user unit.
    let fake = e.dir.join("bin");
    std::fs::create_dir_all(&fake).expect("bin");
    std::fs::write(
        fake.join("systemctl"),
        "#!/bin/sh\necho \"$*\" >> \"$JUNC_TEST_DIR/calls\"\n",
    )
    .expect("fake systemctl");
    Command::new("chmod")
        .arg("755")
        .arg(fake.join("systemctl"))
        .status()
        .expect("chmod");
    let out = Command::new(env!("CARGO_BIN_EXE_junc"))
        .args(["--json", "teardown", "--yes"])
        .env(
            "PATH",
            format!(
                "{}:{}",
                fake.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("JUNC_TEST_DIR", &e.dir)
        .env("JUNC_JUNCD", bin("juncd"))
        .env("XDG_STATE_HOME", &state)
        .env("HOME", &e.dir)
        // Never the Omarchy settings of whoever runs the test.
        .env("XDG_CONFIG_HOME", e.dir.join("config"))
        .env_remove("JUNC_OMARCHY_PLUGIN")
        .env("DBUS_SESSION_BUS_ADDRESS", &e.address)
        .stdin(Stdio::null())
        .output()
        .expect("junc teardown");
    let doc: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(
        doc["ok"],
        true,
        "{doc} {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(doc["data"]["service"], "disabled");
    let calls = std::fs::read_to_string(e.dir.join("calls")).unwrap_or_default();
    assert!(calls.contains("disable --now juncd.service"), "{calls}");
    let after = pw_dump();
    assert_eq!(junc_objects(&after), 0, "JUNC objects left");
    assert_ne!(
        default_key(&after, "default.configured.audio.source").as_deref(),
        Some("junc.td"),
        "the configured default input is not the removed Junction"
    );
    // WirePlumber moves the default in use a moment later.
    wait(|| default_source(&pw_dump()) == before);
    // The config was not touched.
    assert!(
        std::fs::read_to_string(&cfg)
            .expect("cfg")
            .contains("Teardown")
    );

    // Without --yes and without a terminal, nothing happens.
    let refused = e.junc(&["--json", "teardown"]).json();
    assert_eq!(refused["error"]["code"], "E_NEEDS_CONFIRMATION");
}

/// F20 (B6): the package's `pre_remove` runs `junc teardown --yes` as
/// every logged-in user with a session bus, with that user's environment,
/// and never fails the removal.
#[test]
fn pre_remove_tears_down_every_logged_in_session() {
    let dir = std::env::temp_dir().join(format!("junc-cli-preremove-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).expect("dir");
    // Two users logged in; only alice has a session bus.
    for (uid, bus) in [("1000", true), ("1001", false)] {
        let run = dir.join("run").join(uid);
        std::fs::create_dir_all(&run).expect("run dir");
        if bus {
            std::os::unix::net::UnixListener::bind(run.join("bus")).expect("bus socket");
        }
    }
    let script = |name: &str, body: &str| {
        let p = bin.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).expect("fake");
        Command::new("chmod")
            .arg("755")
            .arg(&p)
            .status()
            .expect("chmod");
    };
    script(
        "loginctl",
        "printf '1000 alice no active\\n1001 bob no active\\n'",
    );
    script("getent", "echo \"$2:x:1000:1000::/home/$2:/bin/sh\"");
    // runuser -u USER -- CMD…: record who and what, then run the rest.
    script(
        "runuser",
        "user=$2; shift 3; echo \"$user $*\" >> \"$JUNC_TEST_DIR/calls\"; exit 1",
    );
    let out = Command::new("bash")
        .arg("-c")
        .arg(format!(
            "source {}; pre_remove",
            repo().join("packaging/arch/junc.install").display()
        ))
        .env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("JUNC_RUN_USER_DIR", dir.join("run"))
        .env("JUNC_TEST_DIR", &dir)
        .output()
        .expect("bash");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let calls = std::fs::read_to_string(dir.join("calls")).unwrap_or_default();
    let run = dir.join("run").join("1000");
    assert_eq!(
        calls.trim(),
        format!(
            "alice env HOME=/home/alice XDG_RUNTIME_DIR={r} DBUS_SESSION_BUS_ADDRESS=unix:path={r}/bus junc teardown --yes",
            r = run.display()
        ),
        "only alice, with her environment"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `junc --json watch` as a child, its lines on a channel.
struct Watch {
    _child: Kill,
    lines: std::sync::mpsc::Receiver<Value>,
    /// Every event read so far, in order.
    seen: std::cell::RefCell<Vec<Value>>,
}

impl Env {
    fn watch(&self) -> Watch {
        let mut child = Command::new(env!("CARGO_BIN_EXE_junc"))
            .args(["--json", "watch"])
            .env("DBUS_SESSION_BUS_ADDRESS", &self.address)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("junc watch");
        let out = child.stdout.take().expect("stdout");
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let v: Value =
                    serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"));
                if tx.send(v).is_err() {
                    return;
                }
            }
        });
        Watch {
            _child: Kill(child),
            lines,
            seen: std::cell::RefCell::new(Vec::new()),
        }
    }
}

impl Watch {
    /// The next event matching `want`, skipping others; panics after 10 s.
    fn expect(&self, what: &str, want: impl Fn(&Value) -> bool) -> Value {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            let left = until.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(v) => {
                    self.seen.borrow_mut().push(v.clone());
                    if want(&v) {
                        return v;
                    }
                }
                Err(_) => panic!("no {what} event in {:?}", self.seen.borrow()),
            }
        }
    }
}

/// F23 (B7): the stream starts with the current state and reports
/// Junctions turned off and on, created and removed.
#[test]
fn watch_streams_junction_changes() {
    let Some(e) = start("watch", true, false) else {
        return;
    };
    let w = e.watch();
    w.expect("initial capture", |v| {
        v["event"] == "capture" && v["apps"] == serde_json::json!([])
    });
    w.expect("initial junction", |v| {
        v["event"] == "junction"
            && v["id"] == "obs"
            && v["enabled"] == true
            && v["default"] == "none"
            && v["inputs"] == 0
    });
    e.ok(&["default", "obs", "--as", "input"]);
    w.expect("obs the default input", |v| {
        v["event"] == "junction" && v["id"] == "obs" && v["default"] == "input"
    });
    e.ok(&["default", "obs", "--clear"]);
    e.ok(&["disable", "obs"]);
    w.expect("obs off", |v| {
        v["event"] == "junction" && v["id"] == "obs" && v["enabled"] == false
    });
    e.ok(&["enable", "obs"]);
    w.expect("obs on", |v| {
        v["event"] == "junction" && v["id"] == "obs" && v["enabled"] == true
    });
    e.ok(&["create", "junction", "Second One"]);
    w.expect("new junction", |v| {
        v["event"] == "junction" && v["id"] == "second-one"
    });
    e.ok(&["delete", "second-one", "--yes"]);
    w.expect("removed", |v| {
        v["event"] == "junction" && v["id"] == "second-one" && v["removed"] == true
    });
}

/// F23 (B7), live: an app routed into an enabled Junction shows up in the
/// capture event, and leaves it when the Junction is turned off.
#[test]
fn watch_reports_captured_apps() {
    if std::env::var("JUNC_LIVE_PIPEWIRE").as_deref() != Ok("1") {
        eprintln!("skipped: run scripts/ci/live-pipewire.sh");
        return;
    }
    let Some(e) = start("watchlive", true, true) else {
        return;
    };
    e.ok(&["create", "junction", "Cap"]);
    e.ok(&["input", "add", "cap", "--app", "capplayer"]);
    let w = e.watch();
    w.expect("nothing captured", |v| {
        v["event"] == "capture" && v["apps"] == serde_json::json!([])
    });
    let _app = pw_cat_app(
        "-p",
        "{ node.name=capplayer application.name=CapPlayer application.process.binary=capplayer }",
    );
    w.expect("app captured", |v| {
        v["event"] == "capture" && v["apps"] == serde_json::json!(["CapPlayer"])
    });
    e.ok(&["disable", "cap"]);
    w.expect("capture ends", |v| {
        v["event"] == "capture" && v["apps"] == serde_json::json!([])
    });
    assert!(
        w.seen
            .borrow()
            .iter()
            .any(|v| v["event"] == "graph" && v["generation"].as_u64().is_some_and(|g| g > 0)),
        "graph events: {:?}",
        w.seen.borrow()
    );
    e.ok(&["delete", "cap", "--yes"]);
}

/// F22, F24: templates fill into Junctions; an exported preset imports
/// back as the same Junction.
#[test]
fn templates_and_presets_make_junctions() {
    // Offline: the built-in set.
    let list = offline(&["--json", "template", "list"]).json();
    let ids: Vec<&str> = list["data"]["templates"]
        .as_array()
        .expect("templates")
        .iter()
        .filter_map(|t| t["id"].as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "mic-plus-app",
            "cable",
            "combine-inputs",
            "separate-tracks",
            "stream-mix",
            "start-empty"
        ]
    );
    let show = offline(&["template", "show", "mic-plus-app"]);
    assert!(show.stdout.contains("[[preset.slot]]"), "{}", show.stdout);
    let typo = offline(&["--json", "template", "show", "cabel"]);
    assert_eq!(typo.code, 4);
    assert_eq!(typo.json()["error"]["hint"], "Did you mean 'cable'?");

    let Some(e) = start("presets", true, false) else {
        return;
    };
    let made = e.ok(&[
        "create",
        "junction",
        "Call Mix",
        "--template",
        "mic-plus-app",
        "--slot",
        "mic=alsa_input.usb-mic",
        "--slot",
        "app=spotify",
    ]);
    assert!(made.contains("(call-mix)"), "{made}");
    let j = e.status("call-mix");
    assert_eq!(j["inputs"][0]["match"]["device"], "alsa_input.usb-mic");
    assert_eq!(j["inputs"][1]["match"]["binary"], "spotify");

    let missing = e.junc(&[
        "--json",
        "create",
        "junction",
        "X",
        "--template",
        "cable",
        "--slot",
        "from=firefox",
    ]);
    assert_eq!(missing.code, 2);
    assert_eq!(missing.json()["error"]["code"], "E_PRESET_FILL");

    // A template file imports like a preset; its name comes from the slots.
    let cable = repo().join("templates/cable.toml");
    let out = e.ok(&[
        "preset",
        "import",
        &cable.to_string_lossy(),
        "--slot",
        "from=firefox",
        "--slot",
        "to=obs",
    ]);
    assert!(out.contains("firefox to obs"), "{out}");
    assert_eq!(e.status("firefox-to-obs")["kind"], "cable");

    // Export, then import: the same Junction under a new id. It is not
    // made a second default input.
    e.ok(&["input", "add", "call-mix", "--monitor", "alsa_output.hp"]);
    e.ok(&["volume", "call-mix", "app", "70"]);
    e.ok(&["switch", "call-mix"]);
    let file = e.dir.join("call.toml");
    let exported = e.ok(&[
        "preset",
        "export",
        "Call Mix",
        "-o",
        &file.to_string_lossy(),
    ]);
    assert!(
        exported.contains("input/mic"),
        "device-specific: {exported}"
    );
    let json = e.junc(&["--json", "preset", "export", "call-mix"]).json();
    assert_eq!(
        json["data"]["device_specific"],
        serde_json::json!(["input/mic", "input/alsa-output-hp"])
    );
    let imported = e.ok(&["preset", "import", &file.to_string_lossy()]);
    assert!(imported.contains("(call-mix-2)"), "{imported}");
    assert!(imported.contains("junc switch call-mix-2"), "{imported}");
    let (a, b) = (e.status("call-mix"), e.status("call-mix-2"));
    assert_eq!(a["inputs"], b["inputs"]);
    assert_eq!(a["listens"], b["listens"]);
    assert_eq!(b["default"], "none");

    // A broken preset says where.
    let bad = e.dir.join("bad.toml");
    std::fs::write(
        &bad,
        "schema = 1\n[preset]\nname = \"B\"\n[junction]\nname = \"B\"\n[[junction.input]]\nid = \"m\"\nmatch = { slot = \"mic\" }\n",
    )
    .expect("write");
    let out = e.junc(&["--json", "preset", "import", &bad.to_string_lossy()]);
    assert_eq!(out.code, 3);
    let err = &out.json()["error"];
    assert_eq!(err["code"], "E_PRESET_SLOT");
    assert_eq!(err["location"]["line"], 8);
}

/// F14: text for people is in the chosen language (--lang, Omarchy's
/// language file, the locale); --json stays English.
#[test]
fn messages_follow_the_language_and_json_stays_english() {
    let dir = std::env::temp_dir().join(format!("junc-cli-lang-{}", std::process::id()));
    let built = ["de", "ja"].iter().all(|lang| {
        let mo = dir.join(format!("locale/{lang}/LC_MESSAGES"));
        std::fs::create_dir_all(&mo).expect("dir");
        Command::new("msgfmt")
            .arg("-o")
            .arg(mo.join("junc.mo"))
            .arg(repo().join(format!("po/cli/{lang}.po")))
            .status()
            .is_ok_and(|s| s.success())
    });
    if !built {
        assert!(std::env::var_os("CI").is_none(), "msgfmt is missing in CI");
        eprintln!("skipped: no msgfmt");
        return;
    }
    let config = dir.join("ok.toml");
    std::fs::write(&config, "schema = 1\n").expect("config");
    let lang_file = dir.join("language");
    let junc = |args: &[&str], omarchy: Option<&str>| {
        if let Some(code) = omarchy {
            std::fs::write(&lang_file, format!("{code}\n")).expect("language");
        } else {
            let _ = std::fs::remove_file(&lang_file);
        }
        let out = Command::new(env!("CARGO_BIN_EXE_junc"))
            .args(args)
            .env("JUNC_LOCALEDIR", dir.join("locale"))
            .env("OMARCHY_LANGUAGE_FILE", &lang_file)
            .env("LANG", "C.UTF-8")
            .env_remove("LC_ALL")
            .env_remove("LC_MESSAGES")
            .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent")
            .output()
            .expect("junc");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let path = config.to_string_lossy().into_owned();
    let validate = ["config", "validate", path.as_str()];

    let (out, _) = junc(&validate, None);
    assert!(out.ends_with(": valid\n"), "{out}");
    let (out, _) = junc(&validate, Some("de"));
    assert!(out.ends_with(": gültig\n"), "Omarchy's language: {out}");
    let (out, _) = junc(&[&["--lang", "en"][..], &validate].concat(), Some("de"));
    assert!(out.ends_with(": valid\n"), "--lang wins: {out}");
    let (out, _) = junc(&[&["--lang", "en_XA"][..], &validate].concat(), None);
    assert!(out.contains("⟦"), "pseudo-locale: {out}");
    let (out, _) = junc(&validate, Some("ja"));
    assert!(out.ends_with(": 有効\n"), "Japanese: {out}");
    // A language without a catalog is English.
    let (out, _) = junc(&validate, Some("fi"));
    assert!(out.ends_with(": valid\n"), "{out}");
    // Right-to-left languages are English until RTL (PRD).
    let (out, _) = junc(&validate, Some("ar"));
    assert!(out.ends_with(": valid\n"), "Arabic shows English: {out}");

    let (_, err) = junc(&["schema", "--version", "9"], Some("de"));
    assert!(
        err.contains("Es gibt keine Konfigurationsschema-Version 9."),
        "{err}"
    );
    let (out, _) = junc(&["--json", "schema", "--version", "9"], Some("de"));
    let doc: Value = serde_json::from_str(&out).expect("json");
    assert_eq!(
        doc["error"]["message"], "There is no config schema version 9.",
        "--json is English"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
