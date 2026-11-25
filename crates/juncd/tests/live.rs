// SPDX-License-Identifier: GPL-3.0-or-later

//! juncd against a live isolated PipeWire (hardware layer 2): app churn
//! re-linking (F10), override detection without fighting (F17), foreign links
//! untouched. Runs only with `JUNC_LIVE_PIPEWIRE=1` via
//! `scripts/ci/live-pipewire.sh`; refuses a desktop session.

use junc_core::graph::Graph;
use junc_pw::{Event, PwBackend};
use std::collections::BTreeSet;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

const CONFIG: &str = r#"schema = 1
[[junction]]
id = "zoom"
name = "Zoom Mic"
[[junction.input]]
id = "spotify"
match = { binary = "spotify" }
[[junction.input]]
id = "mic"
match = { device = "fake.hwin" }
[[junction.listen]]
id = "hp"
target = "default"
"#;
const CHURN: usize = 20;

fn enabled() -> bool {
    if std::env::var("JUNC_LIVE_PIPEWIRE").as_deref() != Ok("1") {
        eprintln!("skipped: run scripts/ci/live-pipewire.sh");
        return false;
    }
    let uid = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1).map(str::to_owned))
        })
        .unwrap_or_default();
    assert_ne!(
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_default(),
        format!("/run/user/{uid}"),
        "refusing to run against the desktop session"
    );
    true
}

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spotify() -> Kill {
    Kill(
        Command::new("pw-cat")
            .args([
                "-p",
                "--raw",
                "-P",
                "{ application.name=Spotify application.process.binary=spotify node.name=spotify }",
                "/dev/zero",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("pw-cat"),
    )
}

/// Waits until `pred` holds for a snapshot; returns it and when it arrived.
fn wait_for(
    rx: &Receiver<Event>,
    g: &mut Graph,
    timeout: Duration,
    pred: impl Fn(&Graph) -> bool,
) -> Option<Instant> {
    if pred(g) {
        return Some(Instant::now());
    }
    let end = Instant::now() + timeout;
    while let Ok(ev) = rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
        if let Event::Graph(n) = ev {
            *g = *n;
            if pred(g) {
                return Some(Instant::now());
            }
        }
    }
    None
}

/// (serial, id) of the app's nodes. Registry ids are reused after an object
/// goes away; `object.serial` never is.
fn spotify_nodes(g: &Graph) -> BTreeSet<(u32, u32)> {
    g.nodes
        .values()
        .filter(|n| n.name() == "spotify")
        .map(|n| (n.props.number("object.serial").unwrap_or(0), n.id))
        .collect()
}

fn junc_links_from(g: &Graph, node: u32) -> usize {
    g.links
        .values()
        .filter(|l| l.is_junc() && l.output_node == node)
        .count()
}

#[test]
fn relinks_on_churn_and_does_not_fight_overrides() {
    if !enabled() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("juncd-live-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let cfg = dir.join("junctions.toml");
    std::fs::write(&cfg, CONFIG).expect("config");
    let log = dir.join("juncd.log");

    let mut app = spotify();
    let daemon = Kill(
        Command::new(env!("CARGO_BIN_EXE_juncd"))
            .arg("--config")
            .arg(&cfg)
            .stderr(std::fs::File::create(&log).expect("log"))
            .spawn()
            .expect("juncd"),
    );

    let (tx, rx) = channel();
    let _observer = PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    })
    .expect("observer");
    let mut g = Graph::default();
    let built = wait_for(&rx, &mut g, Duration::from_secs(10), |g| {
        g.links.values().filter(|l| l.is_junc()).count() >= 14
    });
    assert!(built.is_some(), "juncd did not build the Junction");
    let foreign: BTreeSet<u32> = g
        .links
        .values()
        .filter(|l| !l.is_junc() && !spotify_nodes(&g).iter().any(|(_, id)| *id == l.output_node))
        .map(|l| l.id)
        .collect();

    // App churn: every restart must be re-linked, quickly.
    let mut relink_ms = Vec::new();
    for i in 0..CHURN {
        let old = spotify_nodes(&g);
        drop(app);
        app = spotify();
        let appeared = wait_for(&rx, &mut g, Duration::from_secs(5), |g| {
            spotify_nodes(g).iter().any(|n| !old.contains(n))
        })
        .unwrap_or_else(|| panic!("restart {i}: app never appeared"));
        let (_, node) = *spotify_nodes(&g)
            .iter()
            .find(|n| !old.contains(n))
            .expect("new node");
        let linked = wait_for(&rx, &mut g, Duration::from_secs(2), |g| {
            junc_links_from(g, node) == 2
        })
        .unwrap_or_else(|| panic!("restart {i}: not re-linked within 2 s"));
        relink_ms.push(linked.duration_since(appeared).as_secs_f64() * 1000.0);
    }
    relink_ms.sort_by(f64::total_cmp);
    let p95 = relink_ms[(relink_ms.len() * 95 / 100).min(relink_ms.len() - 1)];
    eprintln!(
        "re-link over {CHURN} restarts: min {:.1} p50 {:.1} p95 {:.1} max {:.1} ms",
        relink_ms[0],
        relink_ms[relink_ms.len() / 2],
        p95,
        relink_ms[relink_ms.len() - 1]
    );
    assert!(p95 < 100.0, "re-link p95 {p95:.1} ms ≥ 100 ms");

    // Another tool removes a JUNC link: juncd must not recreate it, and must
    // say so.
    let victim = g
        .links
        .values()
        .find(|l| l.props.get("junc.ref") == Some("in/mic/1->2"))
        .map(|l| l.id)
        .expect("mic link");
    let status = Command::new("pw-cli")
        .args(["destroy", &victim.to_string()])
        .output()
        .expect("pw-cli");
    assert!(status.status.success());
    std::thread::sleep(Duration::from_millis(1500));
    while let Ok(Event::Graph(n)) = rx.try_recv() {
        g = *n;
    }
    assert!(
        !g.links
            .values()
            .any(|l| l.props.get("junc.ref") == Some("in/mic/1->2")),
        "juncd fought the other tool"
    );
    let mut text = String::new();
    std::fs::File::open(&log)
        .and_then(|mut f| f.read_to_string(&mut text))
        .expect("log");
    assert!(
        text.contains("notice link_overridden junction=zoom ref=in/mic/1->2"),
        "{text}"
    );

    // Links JUNC did not make are all still there.
    for id in &foreign {
        assert!(g.links.contains_key(id), "foreign link {id} disappeared");
    }
    drop(daemon);
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

// --- F16: defaults ----------------------------------------------------------

fn run_juncd(dir: &std::path::Path, tag: &str, config: &str) -> (Kill, std::path::PathBuf) {
    let cfg = dir.join(format!("{tag}.toml"));
    std::fs::write(&cfg, config).expect("config");
    let log = dir.join(format!("{tag}.log"));
    let child = Command::new(env!("CARGO_BIN_EXE_juncd"))
        .arg("--config")
        .arg(&cfg)
        .arg("--state-dir")
        .arg(dir)
        .stderr(std::fs::File::create(&log).expect("log"))
        .spawn()
        .expect("juncd");
    (Kill(child), log)
}

fn read(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn sh_ok(cmd: &str, args: &[&str]) -> String {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{cmd}: {e}"));
    assert!(
        out.status.success(),
        "{cmd} {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn node_id(g: &Graph, name: &str) -> u32 {
    g.node_by_name(name)
        .unwrap_or_else(|| panic!("no node {name}"))
        .id
}

/// Removes every JUNC object so tests do not see each other's leftovers.
fn clean(dir: &std::path::Path, rx: &Receiver<Event>, g: &mut Graph) {
    let (_d, _) = run_juncd(dir, "clean", "schema = 1\n");
    assert!(
        wait_for(rx, g, Duration::from_secs(5), |g| !g
            .nodes
            .values()
            .any(|n| n.is_junc()))
        .is_some(),
        "cleanup"
    );
}

const DEFAULT_IN: &str =
    "schema = 1\n[[junction]]\nid = \"zoom\"\nname = \"Zoom Mic\"\ndefault = \"input\"\n";
const OFF: &str = "schema = 1\n[[junction]]\nid = \"zoom\"\nname = \"Zoom Mic\"\nenabled = false\ndefault = \"input\"\n";

#[test]
fn default_input_set_restored_and_cleared() {
    if !enabled() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("juncd-defaults-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let (tx, rx) = channel();
    let _observer = PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    })
    .expect("observer");
    let mut g = Graph::default();
    let src = |g: &Graph| g.defaults.configured_source.clone();

    // The user's own mic is the configured default.
    sh_ok(
        "pw-cli",
        &[
            "create-node",
            "adapter",
            "{ factory.name=support.null-audio-sink node.name=tmp.mic media.class=Audio/Source/Virtual object.linger=true }",
        ],
    );
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(5), |g| g
            .node_by_name("tmp.mic")
            .is_some())
        .is_some()
    );
    sh_ok(
        "wpctl",
        &["set-default", &node_id(&g, "tmp.mic").to_string()],
    );
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(5), |g| src(g).as_deref()
            == Some("tmp.mic"))
        .is_some()
    );

    // Default = input: the Junction takes over.
    let (d, _) = run_juncd(&dir, "on", DEFAULT_IN);
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(5), |g| src(g).as_deref()
            == Some("junc.zoom"))
        .is_some(),
        "junction not made default: {:?}",
        src(&g)
    );
    drop(d);
    // Turned off (daemon restarted, the saved previous default survives).
    let (d, log) = run_juncd(&dir, "off", OFF);
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(5), |g| src(g).as_deref()
            == Some("tmp.mic"))
        .is_some(),
        "previous default not restored: {:?}\nstate: {}\non log: {}\noff log: {}",
        src(&g),
        read(&dir.join("state.toml")),
        read(&dir.join("on.log")),
        read(&dir.join("off.log"))
    );
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        read(&log)
            .contains("notice default_restored key=default.configured.audio.source to=tmp.mic"),
        "{}",
        read(&log)
    );
    drop(d);

    // Restore target missing: the key is cleared.
    let (d, _) = run_juncd(&dir, "on2", DEFAULT_IN);
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(5), |g| src(g).as_deref()
            == Some("junc.zoom"))
        .is_some()
    );
    drop(d);
    sh_ok("pw-cli", &["destroy", &node_id(&g, "tmp.mic").to_string()]);
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(5), |g| g
            .node_by_name("tmp.mic")
            .is_none())
        .is_some()
    );
    let (d, log) = run_juncd(&dir, "off2", OFF);
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(5), |g| src(g).is_none()).is_some(),
        "key not cleared: {:?}",
        src(&g)
    );
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        read(&log).contains("to=(WirePlumber's choice)"),
        "{}",
        read(&log)
    );
    drop(d);
    clean(&dir, &rx, &mut g);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn listen_default_follows_the_system_output() {
    if !enabled() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("juncd-follow-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let (tx, rx) = channel();
    let _observer = PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    })
    .expect("observer");
    let mut g = Graph::default();
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(5), |g| g
            .node_by_name("fake.obs")
            .is_some())
        .is_some()
    );
    sh_ok(
        "wpctl",
        &["set-default", &node_id(&g, "fake.hp").to_string()],
    );
    let listen_to = |g: &Graph, out: &str| {
        let Some(o) = g.node_by_name(out) else {
            return 0;
        };
        g.links
            .values()
            .filter(|l| l.is_junc() && l.input_node == o.id)
            .count()
    };
    let (_d, _) = run_juncd(
        &dir,
        "listen",
        "schema = 1\n[[junction]]\nid = \"mix\"\nname = \"Mix\"\n[[junction.listen]]\nid = \"out\"\ntarget = \"default\"\n",
    );
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(5), |g| listen_to(
            g, "fake.hp"
        ) == 2)
        .is_some(),
        "listen to fake.hp"
    );
    sh_ok(
        "wpctl",
        &["set-default", &node_id(&g, "fake.obs").to_string()],
    );
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(5), |g| listen_to(
            g, "fake.obs"
        ) == 2
            && listen_to(g, "fake.hp") == 0)
        .is_some(),
        "listen did not follow the default output"
    );
    drop(_d);
    clean(&dir, &rx, &mut g);
    let _ = std::fs::remove_dir_all(&dir);
}

// --- F22: hand edits ----------------------------------------------------------

#[test]
fn hand_edits_apply_live_and_bad_edits_keep_the_last_good_config() {
    if !enabled() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("juncd-edits-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let (tx, rx) = channel();
    let _observer = PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    })
    .expect("observer");
    let mut g = Graph::default();
    let has = |g: &Graph, name: &str| g.node_by_name(name).is_some_and(|n| n.is_junc());

    let (_d, log) = run_juncd(
        &dir,
        "edits",
        "schema = 1\n# my notes\n[[junction]]\nid = \"a\"\nname = \"A\"\n",
    );
    assert!(wait_for(&rx, &mut g, Duration::from_secs(5), |g| has(g, "junc.a")).is_some());
    let cfg = dir.join("edits.toml");

    // A valid edit, written the way editors do (temp file + rename).
    let tmp = dir.join(".edits.toml.swp");
    std::fs::write(&tmp, "schema = 1\n# my notes\n[[junction]]\nid = \"a\"\nname = \"A\"\n[[junction]]\nid = \"b\"\nname = \"B\"\n")
        .expect("write");
    std::fs::rename(&tmp, &cfg).expect("rename");
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(5), |g| has(g, "junc.b")).is_some(),
        "edit not applied"
    );

    // An invalid edit: reported with a position, nothing changes.
    std::fs::write(
        &cfg,
        "schema = 1\n[[junction]]\nid = \"a\"\nname = \"A\"\nvolum = 3\n",
    )
    .expect("write");
    std::thread::sleep(Duration::from_millis(500));
    while let Ok(Event::Graph(n)) = rx.try_recv() {
        g = *n;
    }
    assert!(
        has(&g, "junc.a") && has(&g, "junc.b"),
        "last good config must stay in use"
    );
    let text = read(&log);
    assert!(
        text.contains("notice config_error")
            && text.contains("edits.toml:5:1")
            && text.contains("Did you mean 'volume'"),
        "{text}"
    );

    // Restarting with the broken file uses the last good config.
    drop(_d);
    let child = Command::new(env!("CARGO_BIN_EXE_juncd"))
        .arg("--config")
        .arg(&cfg)
        .arg("--state-dir")
        .arg(&dir)
        .stderr(std::fs::File::create(dir.join("restart.log")).expect("log"))
        .spawn()
        .expect("juncd");
    let restarted = Kill(child);
    std::thread::sleep(Duration::from_millis(800));
    let text = read(&dir.join("restart.log"));
    assert!(text.contains("using the last good config"), "{text}");
    drop(restarted);
    clean(&dir, &rx, &mut g);
    let _ = std::fs::remove_dir_all(&dir);
}
