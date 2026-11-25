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
/// regressions.
#[test]
fn enable_stays_fast() {
    let Some(e) = start("time", true, false) else {
        return;
    };
    // Forty runs, so p95 does not hinge on one slow start.
    let mut times: Vec<Duration> = (0..40)
        .map(|i| {
            let t = Instant::now();
            e.ok(&[if i % 2 == 0 { "disable" } else { "enable" }, "obs"]);
            t.elapsed()
        })
        .collect();
    times.sort();
    let p95 = times[(times.len() * 95).div_ceil(100) - 1];
    eprintln!("junc enable: p50 {:?}, p95 {p95:?}", times[times.len() / 2]);
    assert!(p95 < Duration::from_millis(250), "p95 {p95:?}");
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
