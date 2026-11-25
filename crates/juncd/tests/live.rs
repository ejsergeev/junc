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
