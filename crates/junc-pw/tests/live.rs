// SPDX-License-Identifier: GPL-3.0-or-later

//! Live PipeWire test (hardware layer 2). Runs only with
//! `JUNC_LIVE_PIPEWIRE=1`, inside an isolated PipeWire instance
//! (`scripts/ci/live-pipewire.sh`); it refuses to touch a desktop session.

use junc_core::config;
use junc_core::graph::Graph;
use junc_core::plan::{Held, plan};
use junc_pw::{Event, PwBackend};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

const CONFIG: &str = r#"
schema = 1
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

fn enabled() -> bool {
    if std::env::var("JUNC_LIVE_PIPEWIRE").as_deref() != Ok("1") {
        eprintln!(
            "skipped: set JUNC_LIVE_PIPEWIRE=1 inside an isolated PipeWire (scripts/ci/live-pipewire.sh)"
        );
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
    let live = format!("/run/user/{uid}");
    assert_ne!(
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_default(),
        live,
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
    let child = Command::new("pw-cat")
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
        .expect("pw-cat");
    Kill(child)
}

fn next_graph(rx: &Receiver<Event>, timeout: Duration) -> Graph {
    let end = Instant::now() + timeout;
    loop {
        let left = end.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(Event::Graph(g)) => return *g,
            Ok(Event::ActionFailed { action, error }) => panic!("{action}: {error}"),
            Ok(_) => {}
            Err(e) => panic!("no graph within {timeout:?}: {e}"),
        }
    }
}

/// Takes graph snapshots until none arrives for 200 ms.
fn settle(rx: &Receiver<Event>, mut g: Graph) -> Graph {
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Event::Graph(n)) => g = *n,
            Ok(Event::ActionFailed { action, error }) => panic!("{action}: {error}"),
            Ok(other) => eprintln!("  event {other:?}"),
            Err(_) => return g,
        }
    }
}

/// Plans and applies until nothing is left; returns the final graph and the
/// number of rounds that had actions.
fn converge(
    b: &PwBackend,
    rx: &Receiver<Event>,
    cfg: &junc_core::model::Config,
    mut g: Graph,
) -> (Graph, usize) {
    for round in 0..8 {
        let p = plan(cfg, &g, &Held::new());
        if p.is_empty() {
            return (g, round);
        }
        eprintln!(
            "round {round}: {}",
            p.actions
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" | ")
        );
        b.apply(p.actions);
        g = next_graph(rx, Duration::from_secs(5));
        // Created objects show up in later batches; wait until the graph
        // stops changing for this round.
        g = settle(rx, g);
    }
    panic!("did not converge");
}

fn sh(cmd: &str, args: &[&str]) -> String {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{cmd}: {e}"));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn builds_adopts_and_cleans_up_a_junction() {
    if !enabled() {
        return;
    }
    let _app = spotify();
    let cfg = config::load(CONFIG, "live.toml").expect("config").config;
    let (tx, rx) = channel();
    let backend = PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    })
    .expect("spawn");
    let g = next_graph(&rx, Duration::from_secs(5));
    let start = Instant::now();
    let (g, rounds) = converge(&backend, &rx, &cfg, g);
    eprintln!("built in {rounds} rounds, {:?}", start.elapsed());

    // Visible to PulseAudio as a source and a sink; gain nodes hidden.
    let sources = sh("pactl", &["list", "short", "sources"]);
    let sinks = sh("pactl", &["list", "short", "sinks"]);
    assert!(
        sources
            .lines()
            .any(|l| l.split('\t').nth(1) == Some("junc.zoom")),
        "{sources}"
    );
    assert!(
        sinks
            .lines()
            .any(|l| l.split('\t').nth(1) == Some("junc.zoom.sink")),
        "{sinks}"
    );
    assert!(
        !sources.contains("junc.zoom.in.") && !sinks.contains("junc.zoom.in."),
        "gain nodes must be hidden"
    );
    // Visible to JACK clients.
    let jack = sh("pw-jack", &["jack_lsp"]);
    assert!(jack.contains("Zoom Mic:"), "{jack}");

    let junc_links = |g: &Graph| g.links.values().filter(|l| l.is_junc()).count();
    // spotify 2 + mic 2 + 2 gains→bus 4 + sink→bus 2 + bus→listen 2 + listen→hp 2
    assert_eq!(
        junc_links(&g),
        14,
        "{:#?}",
        g.links
            .values()
            .filter(|l| l.is_junc())
            .map(|l| l.props.get("junc.ref"))
            .collect::<Vec<_>>()
    );

    // A new backend adopts the lingering objects: nothing to do.
    drop(backend);
    let (tx, rx) = channel();
    let backend = PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    })
    .expect("respawn");
    let g = settle(&rx, next_graph(&rx, Duration::from_secs(5)));
    let p = plan(&cfg, &g, &Held::new());
    assert!(
        p.is_empty(),
        "objects survived the client and were adopted: {:?}",
        p.actions
    );

    // An empty config removes every JUNC object and nothing else.
    let before: Vec<u32> = g
        .nodes
        .values()
        .filter(|n| !n.is_junc())
        .map(|n| n.id)
        .collect();
    let (g, _) = converge(&backend, &rx, &junc_core::model::Config::default(), g);
    assert_eq!(g.nodes.values().filter(|n| n.is_junc()).count(), 0);
    assert_eq!(junc_links(&g), 0);
    let after: Vec<u32> = g.nodes.values().map(|n| n.id).collect();
    assert_eq!(before, after);
}
