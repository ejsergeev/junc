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
const CHURN: usize = 60;

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
            .arg("--no-dbus")
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
        .arg("--no-dbus")
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
        .arg("--no-dbus")
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

// --- F5: Cable ---------------------------------------------------------------

const CABLE: &str = r#"schema = 1
[[junction]]
id = "cab"
name = "Player to Recorder"
kind = "cable"
[[junction.input]]
id = "player"
match = { app = "junc-player" }
[junction.cable]
to = { app = "junc-recorder" }
"#;

const RATE: usize = 48_000;

/// Interleaved stereo s16 noise from a fixed LCG: every sample differs from
/// silence and from its neighbours, so a match proves the audio is intact.
fn noise(frames: usize) -> Vec<i16> {
    let mut x: u32 = 0x1234_5678;
    (0..frames * 2)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            i16::from_le_bytes([(x >> 16) as u8, (x >> 24) as u8]) | 1
        })
        .collect()
}

fn read_s16(path: &std::path::Path) -> Vec<i16> {
    let bytes = std::fs::read(path).unwrap_or_default();
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b))
        .collect()
}

fn pw_cat(mode: &str, app: &str, file: &std::path::Path) -> Kill {
    pw_cat_to(mode, app, file, None)
}

fn pw_cat_to(mode: &str, app: &str, file: &std::path::Path, target: Option<&str>) -> Kill {
    pw_cat_with(mode, app, file, target, "FL,FR")
}

/// pw-cat with raw s16 at 48 kHz and the channels of `map`.
fn pw_cat_with(
    mode: &str,
    app: &str,
    file: &std::path::Path,
    target: Option<&str>,
    map: &str,
) -> Kill {
    let mut c = Command::new("pw-cat");
    if let Some(t) = target {
        c.args(["--target", t]);
    }
    let channels = map.split(',').count().to_string();
    Kill(
        c.args([mode, "--raw", "--format", "s16", "--rate", "48000"])
            .args(["--channels", &channels, "--channel-map", map, "-P"])
            .arg(format!("{{ application.name={app} node.name={app} }}"))
            .arg(file)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("pw-cat"),
    )
}

/// Acceptance of A13: one app's audio reaches another app's capture
/// bit-exact, and the recording app hears only the Cable (not the default
/// mic WirePlumber would give it).
#[test]
fn cable_delivers_one_app_to_another_bit_exact() {
    if !enabled() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("juncd-cable-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let (_d, log) = run_juncd(&dir, "cable", CABLE);
    let (tx, rx) = channel();
    let _observer = PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    })
    .expect("observer");
    let mut g = Graph::default();

    let src = noise(2 * RATE);
    let src_file = dir.join("src.raw");
    let bytes: Vec<u8> = src.iter().flat_map(|s| s.to_le_bytes()).collect();
    std::fs::write(&src_file, bytes).expect("write source");
    let rec_file = dir.join("rec.raw");

    let recorder = pw_cat("-r", "junc-recorder", &rec_file);
    // juncd sends the recorder to the Cable; WirePlumber relinks it.
    let only_cable = |g: &Graph| {
        let Some(rec) = g.node_by_name("junc-recorder") else {
            return false;
        };
        let into: Vec<&str> = g
            .links
            .values()
            .filter(|l| l.input_node == rec.id)
            .filter_map(|l| g.nodes.get(&l.output_node))
            .map(|n| n.name())
            .collect();
        g.targets.get(&rec.id).map(String::as_str) == Some("junc.cab")
            && !into.is_empty()
            && into.iter().all(|n| *n == "junc.cab")
    };
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(10), only_cable).is_some(),
        "the recorder is not fed by the Cable alone\n{}",
        read(&log)
    );

    let player = pw_cat("-p", "junc-player", &src_file);
    let player_routed = |g: &Graph| {
        g.node_by_name("junc-player").is_some_and(|p| {
            g.links.values().any(|l| {
                l.output_node == p.id
                    && g.nodes
                        .get(&l.input_node)
                        .is_some_and(|n| n.name() == "junc.cab.in.player")
            })
        })
    };
    assert!(wait_for(&rx, &mut g, Duration::from_secs(10), player_routed).is_some());
    // Let the 2 s file play out, then stop the recorder.
    let mut player = player;
    let end = Instant::now() + Duration::from_secs(10);
    while player.0.try_wait().ok().flatten().is_none() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
    let _ = Command::new("kill")
        .arg("-INT")
        .arg(recorder.0.id().to_string())
        .status();
    std::thread::sleep(Duration::from_millis(300));
    drop(recorder);

    let n = bit_exact_frames(&src, &read_s16(&rec_file));
    eprintln!("cable: {n} frames bit-exact");
    // Leave the graph as found: later tests time re-links on it.
    drop(_d);
    clean(&dir, &rx, &mut g);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Frames of `src` found unchanged in `rec`, starting from the middle of
/// the source. pw-cat exits without playing its last partial buffer, so
/// the very end of the source may be missing: that tail must be silence
/// in the recording, shorter than two quanta, and everything before it
/// exact. At least one second must match.
fn bit_exact_frames(src: &[i16], rec: &[i16]) -> usize {
    let probe = RATE / 2 * 2; // frame 24 000, interleaved
    let window = &src[probe..probe + 960 * 2];
    let at = rec
        .windows(window.len())
        .step_by(2)
        .position(|w| w == window)
        .map(|f| f * 2)
        .unwrap_or_else(|| panic!("source audio not found in {} recorded samples", rec.len()));
    let n = (src.len() - probe).min(rec.len() - at);
    let same = src[probe..probe + n]
        .iter()
        .zip(&rec[at..at + n])
        .take_while(|(a, b)| a == b)
        .count();
    let tail = &rec[at + same..at + n];
    assert!(
        tail.iter().all(|s| *s == 0) && tail.len() <= 2 * 2048 * 2,
        "sample {same} of {n} differs and the rest is not a silent tail \
         (first values: source {:?}, recording {:?})",
        &src[probe + same..(probe + same + 4).min(src.len())],
        &rec[at + same..(at + same + 4).min(rec.len())]
    );
    assert!(same >= RATE * 2, "only {} frames match", same / 2);
    same / 2
}

/// Acceptance of A14: audio passes four levels of nesting (five Junctions,
/// each the only Input of the next) bit-exact.
#[test]
fn nesting_four_deep_is_bit_exact() {
    if !enabled() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("juncd-nest-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let mut config = String::from(
        "schema = 1\n[[junction]]\nid = \"n0\"\nname = \"N0\"\n\
         [[junction.input]]\nid = \"player\"\nmatch = { app = \"junc-player\" }\n",
    );
    for k in 1..=4 {
        config.push_str(&format!(
            "[[junction]]\nid = \"n{k}\"\nname = \"N{k}\"\n\
             [[junction.input]]\nid = \"below\"\nmatch = {{ junction = \"n{}\" }}\n",
            k - 1
        ));
    }
    let (_d, log) = run_juncd(&dir, "nest", &config);
    let (tx, rx) = channel();
    let _observer = PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    })
    .expect("observer");
    let mut g = Graph::default();
    let named = |g: &Graph, id: u32, name: &str| g.nodes.get(&id).is_some_and(|n| n.name() == name);
    let chained = |g: &Graph| {
        (1..=4).all(|k| {
            g.links.values().any(|l| {
                named(g, l.output_node, &format!("junc.n{}", k - 1))
                    && named(g, l.input_node, &format!("junc.n{k}.in.below"))
            })
        })
    };
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(10), chained).is_some(),
        "the chain was not built\n{}",
        read(&log)
    );

    let src = noise(2 * RATE);
    let src_file = dir.join("src.raw");
    let bytes: Vec<u8> = src.iter().flat_map(|s| s.to_le_bytes()).collect();
    std::fs::write(&src_file, bytes).expect("write source");
    let rec_file = dir.join("rec.raw");
    let recorder = pw_cat_to("-r", "junc-top", &rec_file, Some("junc.n4"));
    let fed = |g: &Graph| {
        g.node_by_name("junc-top").is_some_and(|r| {
            g.links
                .values()
                .any(|l| l.input_node == r.id && named(g, l.output_node, "junc.n4"))
        })
    };
    assert!(wait_for(&rx, &mut g, Duration::from_secs(10), fed).is_some());
    let mut player = pw_cat("-p", "junc-player", &src_file);
    let routed = |g: &Graph| {
        g.node_by_name("junc-player").is_some_and(|p| {
            g.links
                .values()
                .any(|l| l.output_node == p.id && named(g, l.input_node, "junc.n0.in.player"))
        })
    };
    assert!(wait_for(&rx, &mut g, Duration::from_secs(10), routed).is_some());
    let end = Instant::now() + Duration::from_secs(10);
    while player.0.try_wait().ok().flatten().is_none() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
    let _ = Command::new("kill")
        .arg("-INT")
        .arg(recorder.0.id().to_string())
        .status();
    std::thread::sleep(Duration::from_millis(300));
    drop(recorder);

    let n = bit_exact_frames(&src, &read_s16(&rec_file));
    eprintln!("nesting: {n} frames bit-exact through 4 levels");
    drop(_d);
    clean(&dir, &rx, &mut g);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Milliseconds from start to a converged graph, from juncd's log.
fn graph_complete_ms(log: &std::path::Path) -> f64 {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(ms) = read(log)
            .lines()
            .find_map(|l| l.strip_prefix("juncd: graph complete after "))
            .and_then(|r| r.trim_end_matches(" ms").parse::<f64>().ok())
        {
            return ms;
        }
        assert!(Instant::now() < end, "never converged:\n{}", read(log));
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Acceptance of A15 (F9): ten Junctions are restored in under a second,
/// on a fresh graph (login) and when the daemon restarts over the
/// Junctions it left lingering (crash, upgrade).
#[test]
fn ten_junctions_restore_in_under_a_second() {
    if !enabled() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("juncd-restore-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let config: String = std::iter::once("schema = 1\n".to_owned())
        .chain((0..10).map(|k| {
            format!(
                "[[junction]]\nid = \"r{k}\"\nname = \"R{k}\"\n\
                 [[junction.input]]\nid = \"mic\"\nmatch = {{ device = \"fake.hwin\" }}\n\
                 [[junction.listen]]\nid = \"hp\"\ntarget = \"default\"\n"
            )
        }))
        .collect();
    let (tx, rx) = channel();
    let _observer = PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    })
    .expect("observer");
    let mut g = Graph::default();
    let all_routed = |g: &Graph| {
        (0..10).all(|k| {
            let refs: Vec<&str> = g
                .links
                .values()
                .filter(|l| l.props.get("junc.junction") == Some(&format!("r{k}")))
                .filter_map(|l| l.props.get("junc.ref"))
                .collect();
            [
                "sink/2",
                "gain/mic/2",
                "in/mic/1->2",
                "listen/hp/bus/2",
                "listen/hp/2->2",
            ]
            .iter()
            .all(|r| refs.contains(r))
        })
    };

    // From process start to every Junction routed, seen from outside.
    let restore = |tag: &str, g: &mut Graph| {
        let t = Instant::now();
        let (d, log) = run_juncd(&dir, tag, &config);
        let done = wait_for(&rx, g, Duration::from_secs(5), all_routed)
            .unwrap_or_else(|| panic!("{tag}: not restored\n{}", read(&log)));
        (
            d,
            done.duration_since(t).as_secs_f64() * 1000.0,
            graph_complete_ms(&log),
        )
    };
    let (first, cold, cold_inner) = restore("restore", &mut g);
    // A restart finds the Junctions still there (they linger): it must
    // adopt them as they are, recreating nothing (no audio drop).
    let serials = |g: &Graph| {
        let mut v: Vec<(String, Option<u32>)> = g
            .nodes
            .values()
            .filter(|n| n.is_junc())
            .map(|n| (n.name().to_owned(), n.props.number("object.serial")))
            .collect();
        v.sort();
        v
    };
    let before = serials(&g);
    drop(first);
    let (second, _, warm) = restore("restore2", &mut g);
    assert_eq!(serials(&g), before, "a restart recreated Junction nodes");
    eprintln!(
        "restore 10 Junctions: fresh {cold:.0} ms from process start \
         ({cold_inner:.0} ms after connecting); restart adopted them in {warm:.0} ms"
    );
    assert!(cold < 1000.0, "fresh restore took {cold:.0} ms");
    assert!(warm < 1000.0, "restart took {warm:.0} ms to converge");
    drop(second);
    clean(&dir, &rx, &mut g);
    let _ = std::fs::remove_dir_all(&dir);
}

/// F3/F4: a four-channel Bus with an Input linked to channels 3–4 only.
/// The app's stereo must arrive bit-exact on AUX2/AUX3 of the Junction,
/// and channels 1–2 must stay silent.
#[test]
fn custom_links_place_an_input_on_chosen_bus_channels() {
    if !enabled() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("juncd-bus-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let config = "schema = 1\n[[junction]]\nid = \"wide\"\nname = \"Wide\"\n\
                  bus = { channels = 4 }\n[[junction.input]]\nid = \"player\"\n\
                  match = { app = \"junc-player\" }\nlinks = [[1, 3], [2, 4]]\n";
    let (d, log) = run_juncd(&dir, "bus", config);
    let (tx, rx) = channel();
    let _observer = PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    })
    .expect("observer");
    let mut g = Graph::default();
    let named = |g: &Graph, id: u32, name: &str| g.nodes.get(&id).is_some_and(|n| n.name() == name);
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(10), |g| {
            g.node_by_name("junc.wide")
                .is_some_and(|n| g.ports_of(n.id, junc_core::graph::Direction::Out).count() == 4)
        })
        .is_some(),
        "no 4-channel Junction\n{}",
        read(&log)
    );

    let src = noise(2 * RATE);
    let src_file = dir.join("src.raw");
    let bytes: Vec<u8> = src.iter().flat_map(|s| s.to_le_bytes()).collect();
    std::fs::write(&src_file, bytes).expect("write source");
    let rec_file = dir.join("rec.raw");
    let recorder = pw_cat_with(
        "-r",
        "junc-wide-rec",
        &rec_file,
        Some("junc.wide"),
        "AUX0,AUX1,AUX2,AUX3",
    );
    let fed = |g: &Graph| {
        g.node_by_name("junc-wide-rec").is_some_and(|r| {
            g.links
                .values()
                .filter(|l| l.input_node == r.id && named(g, l.output_node, "junc.wide"))
                .count()
                == 4
        })
    };
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(10), fed).is_some(),
        "recorder not linked to all 4 channels"
    );
    let mut player = pw_cat("-p", "junc-player", &src_file);
    let routed = |g: &Graph| {
        let refs: Vec<&str> = g
            .links
            .values()
            .filter(|l| l.props.get("junc.junction") == Some("wide"))
            .filter_map(|l| l.props.get("junc.ref"))
            .collect();
        refs.contains(&"in/player/1->3")
            && refs.contains(&"in/player/2->4")
            && !refs.contains(&"in/player/1->1")
    };
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(10), routed).is_some(),
        "player not routed as configured"
    );
    let end = Instant::now() + Duration::from_secs(10);
    while player.0.try_wait().ok().flatten().is_none() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
    let _ = Command::new("kill")
        .arg("-INT")
        .arg(recorder.0.id().to_string())
        .status();
    std::thread::sleep(Duration::from_millis(300));
    drop(recorder);

    // Split the 4-channel recording: channels 3–4 against the source,
    // channels 1–2 must be silent over the same stretch.
    let rec = read_s16(&rec_file);
    let frames = rec.len() / 4;
    let upper: Vec<i16> = (0..frames)
        .flat_map(|f| [rec[f * 4 + 2], rec[f * 4 + 3]])
        .collect();
    let lower_loud = (0..frames)
        .filter(|f| rec[f * 4] != 0 || rec[f * 4 + 1] != 0)
        .count();
    let n = bit_exact_frames(&src, &upper);
    assert_eq!(
        lower_loud, 0,
        "{lower_loud} frames on Bus channels 1–2 are not silent"
    );
    eprintln!("bus channels 3–4: {n} frames bit-exact, channels 1–2 silent");
    drop(d);
    clean(&dir, &rx, &mut g);
    let _ = std::fs::remove_dir_all(&dir);
}

/// §3.6 recovery: PipeWire restarts (crash, update). Its objects die with
/// it; juncd must wait for the new instance and rebuild every Junction.
#[test]
fn junctions_come_back_after_pipewire_restarts() {
    if !enabled() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("juncd-pwrestart-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let config = "schema = 1\n[[junction]]\nid = \"back\"\nname = \"Back\"\n\
                  [[junction.input]]\nid = \"mic\"\nmatch = { device = \"fake.hwin\" }\n\
                  [[junction.listen]]\nid = \"hp\"\ntarget = \"default\"\n";
    let (d, log) = run_juncd(&dir, "pwrestart", config);
    let (tx, rx) = channel();
    let _observer = PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    })
    .expect("observer");
    let mut g = Graph::default();
    let routed = |g: &Graph| {
        let refs: Vec<&str> = g
            .links
            .values()
            .filter(|l| l.props.get("junc.junction") == Some("back"))
            .filter_map(|l| l.props.get("junc.ref"))
            .collect();
        [
            "in/mic/1->1",
            "in/mic/1->2",
            "listen/hp/1->1",
            "listen/hp/2->2",
        ]
        .iter()
        .all(|r| refs.contains(r))
    };
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(10), routed).is_some(),
        "{}",
        read(&log)
    );

    let session = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../bench/validation/bench/pw-session.sh");
    let ok = |arg: &str| {
        let s = Command::new(&session)
            .arg(arg)
            .status()
            .expect("pw-session.sh");
        assert!(s.success(), "pw-session.sh {arg}");
    };
    ok("stop");
    std::thread::sleep(Duration::from_millis(300));
    let t = Instant::now();
    ok("start");
    let back = wait_for(&rx, &mut Graph::default(), Duration::from_secs(10), routed)
        .unwrap_or_else(|| panic!("not rebuilt after the restart\n{}", read(&log)));
    let ms = back.duration_since(t).as_secs_f64() * 1000.0;
    eprintln!("PipeWire restart: Junction rebuilt and routed {ms:.0} ms after PipeWire started");
    let text = read(&log);
    assert!(
        text.contains("lost PipeWire") && text.matches("connected to PipeWire").count() >= 2,
        "{text}"
    );
    // Let the session settle before the next test (pulse socket).
    std::thread::sleep(Duration::from_millis(1000));
    drop(d);
    let mut g = Graph::default();
    clean(&dir, &rx, &mut g);
    let _ = std::fs::remove_dir_all(&dir);
}

/// F1/S1–S2: a Junction is a device for every audio API: PulseAudio apps
/// (`pactl`), JACK apps (`jack_lsp` through pipewire-jack) and native ones
/// (`pw-cli`) all list it.
#[test]
fn every_audio_api_sees_a_junction() {
    if !enabled() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("juncd-apis-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let (d, log) = run_juncd(
        &dir,
        "apis",
        "schema = 1\n[[junction]]\nid = \"seen\"\nname = \"Seen Everywhere\"\n",
    );
    let (tx, rx) = channel();
    let _observer = PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    })
    .expect("observer");
    let mut g = Graph::default();
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(10), |g| g
            .node_by_name("junc.seen")
            .is_some())
        .is_some(),
        "{}",
        read(&log)
    );
    std::thread::sleep(Duration::from_millis(300));
    let pulse = sh_ok("pactl", &["list", "short", "sources"]);
    assert!(pulse.contains("junc.seen"), "PulseAudio sources:\n{pulse}");
    let pulse = sh_ok("pactl", &["list", "short", "sinks"]);
    assert!(
        pulse.contains("junc.seen.sink"),
        "PulseAudio sinks:\n{pulse}"
    );
    let jack = sh_ok("pw-jack", &["jack_lsp"]);
    assert!(jack.contains("Seen Everywhere"), "JACK ports:\n{jack}");
    let native = sh_ok("pw-cli", &["ls", "Node"]);
    assert!(native.contains("junc.seen"), "{native}");
    drop(d);
    clean(&dir, &rx, &mut g);
    let _ = std::fs::remove_dir_all(&dir);
}

/// F10: a device Input whose device is unplugged and plugged back in ten
/// times is re-linked every time.
#[test]
fn a_device_that_comes_and_goes_is_relinked_every_time() {
    if !enabled() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("juncd-replug-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let config = "schema = 1\n[[junction]]\nid = \"usb\"\nname = \"USB\"\n\
                  [[junction.input]]\nid = \"dev\"\nmatch = { device = \"fake.usb\" }\n";
    let (d, log) = run_juncd(&dir, "replug", config);
    let (tx, rx) = channel();
    let _observer = PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    })
    .expect("observer");
    let mut g = Graph::default();
    let device_linked = |g: &Graph| {
        g.node_by_name("fake.usb").is_some_and(|dev| {
            g.links
                .values()
                .filter(|l| {
                    l.output_node == dev.id
                        && l.props
                            .get("junc.ref")
                            .is_some_and(|r| r.starts_with("in/dev/"))
                })
                .count()
                == 2
        })
    };
    let mut relink_ms = Vec::new();
    for round in 0..10 {
        // A stand-in for a USB input: a foreign null-audio source.
        let plug = Kill(
            Command::new("pw-cli")
                .args([
                    "-m",
                    "create-node",
                    "adapter",
                    "{ factory.name=support.null-audio-sink node.name=fake.usb \
                     media.class=Audio/Source audio.position=[FL FR] }",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("pw-cli"),
        );
        let t = Instant::now();
        let linked = wait_for(&rx, &mut g, Duration::from_secs(5), device_linked)
            .unwrap_or_else(|| panic!("round {round}: not re-linked\n{}", read(&log)));
        relink_ms.push(linked.duration_since(t).as_secs_f64() * 1000.0);
        // Unplug: the device goes when its client goes.
        drop(plug);
        assert!(
            wait_for(&rx, &mut g, Duration::from_secs(5), |g| g
                .node_by_name("fake.usb")
                .is_none())
            .is_some(),
            "round {round}: device did not go away"
        );
    }
    relink_ms.sort_by(f64::total_cmp);
    eprintln!(
        "device re-plug ×10: all re-linked; p50 {:.0} ms, max {:.0} ms (from pw-cli start)",
        relink_ms[5], relink_ms[9]
    );
    drop(d);
    clean(&dir, &rx, &mut g);
    let _ = std::fs::remove_dir_all(&dir);
}

/// RMS of the loud middle of a recording, per sample.
fn rms_middle(samples: &[i16]) -> f64 {
    let first = samples.iter().position(|s| *s != 0).unwrap_or(0);
    let last = samples
        .iter()
        .rposition(|s| *s != 0)
        .unwrap_or(samples.len());
    let n = last.saturating_sub(first);
    let mid = &samples[first + n / 3..first + 2 * n / 3];
    #[allow(clippy::cast_precision_loss)]
    let sum: f64 = mid
        .iter()
        .map(|s| f64::from(*s) * f64::from(*s))
        .sum::<f64>()
        / mid.len().max(1) as f64;
    sum.sqrt()
}

/// RMS ratio of the recording to the source.
fn level_through(
    dir: &std::path::Path,
    src: &[i16],
    app: &str,
    target: &str,
    monitor: bool,
) -> f64 {
    rms_middle(&record_through(dir, src, app, target, monitor)) / rms_middle(src)
}

/// One channel of interleaved stereo.
fn stereo_channel(samples: &[i16], c: usize) -> Vec<i16> {
    samples.iter().skip(c).step_by(2).copied().collect()
}

/// Plays `src` as `app` (not auto-connected, so only JUNC routes it) and
/// records `target` (a node, or a sink's monitor), stereo interleaved.
fn record_through(
    dir: &std::path::Path,
    src: &[i16],
    app: &str,
    target: &str,
    monitor: bool,
) -> Vec<i16> {
    let src_file = dir.join("vol-src.raw");
    let bytes: Vec<u8> = src.iter().flat_map(|s| s.to_le_bytes()).collect();
    std::fs::write(&src_file, bytes).expect("write source");
    let rec_file = dir.join("vol-rec.raw");
    let _ = std::fs::remove_file(&rec_file);
    let props = if monitor {
        "{ node.name=vol-rec stream.capture.sink=true }"
    } else {
        "{ node.name=vol-rec }"
    };
    let recorder = Kill(
        Command::new("pw-cat")
            .args([
                "-r", "--target", target, "--raw", "--format", "s16", "--rate", "48000",
            ])
            .args(["--channels", "2", "--channel-map", "FL,FR", "-P", props])
            .arg(&rec_file)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("pw-cat"),
    );
    std::thread::sleep(Duration::from_millis(500));
    let mut player = Kill(
        Command::new("pw-cat")
            .args([
                "-p",
                "--raw",
                "--format",
                "s16",
                "--rate",
                "48000",
                "--channels",
                "2",
            ])
            .args(["--channel-map", "FL,FR", "-P"])
            .arg(format!(
                "{{ node.name={app} application.name={app} node.autoconnect=false }}"
            ))
            .arg(&src_file)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("pw-cat"),
    );
    let end = Instant::now() + Duration::from_secs(10);
    while player.0.try_wait().ok().flatten().is_none() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
    let _ = Command::new("kill")
        .arg("-INT")
        .arg(recorder.0.id().to_string())
        .status();
    std::thread::sleep(Duration::from_millis(300));
    drop(recorder);
    read_s16(&rec_file)
}

/// F7 (B1): Input, Junction (master) and Listen volumes scale the audio by
/// the cubic gain of their percentage; a hand edit changes them live.
#[test]
fn volumes_scale_the_audio() {
    if !enabled() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("juncd-vol-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let config = |master: u16, listen: u16| {
        format!(
            "schema = 1\n[[junction]]\nid = \"vol\"\nname = \"Vol\"\nvolume = {master}\n\
             [[junction.input]]\nid = \"player\"\nmatch = {{ app = \"vol-player\" }}\nvolume = 50\n\
             [[junction.listen]]\nid = \"hp\"\ntarget = \"fake.hp\"\nvolume = {listen}\n"
        )
    };
    let (d, log) = run_juncd(&dir, "vol", &config(100, 100));
    let (tx, rx) = channel();
    let _observer = PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    })
    .expect("observer");
    let mut g = Graph::default();
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(10), |g| g
            .node_by_name("junc.vol.in.player")
            .is_some())
        .is_some(),
        "{}",
        read(&log)
    );
    std::thread::sleep(Duration::from_millis(300));
    let src = noise(RATE);
    let close = |got: f64, want: f64| (got / want - 1.0).abs() < 0.02;

    let r = level_through(&dir, &src, "vol-player", "junc.vol", false);
    eprintln!("Input 50 %: level {r:.4} (want 0.125)");
    assert!(close(r, 0.125), "Input 50 %: {r}");

    std::fs::write(dir.join("vol.toml"), config(50, 70)).expect("hand edit");
    std::thread::sleep(Duration::from_millis(500));
    let r = level_through(&dir, &src, "vol-player", "junc.vol", false);
    eprintln!("Input 50 % × master 50 %: level {r:.5} (want 0.015625)");
    assert!(close(r, 0.015_625), "master: {r}");
    let r = level_through(&dir, &src, "vol-player", "fake.hp", true);
    eprintln!("… × Listen 70 %: level {r:.6} (want 0.005359)");
    assert!(close(r, 0.015_625 * 0.343), "listen: {r}");
    drop(d);
    clean(&dir, &rx, &mut g);
    let _ = std::fs::remove_dir_all(&dir);
}

/// F6, F8 (B1): a Listen with chosen links plays only those Bus channels;
/// switching an Input off silences it, switching it on brings it back.
#[test]
fn on_off_and_listen_channels() {
    if !enabled() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("juncd-onoff-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let config = |input_on: bool| {
        format!(
            "schema = 1\n[[junction]]\nid = \"oo\"\nname = \"On Off\"\n\
             [[junction.input]]\nid = \"player\"\nmatch = {{ app = \"oo-player\" }}\nenabled = {input_on}\n\
             [[junction.listen]]\nid = \"hp\"\ntarget = \"fake.hp\"\nlinks = [[1, 1]]\n"
        )
    };
    let (d, log) = run_juncd(&dir, "oo", &config(true));
    let (tx, rx) = channel();
    let _observer = PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    })
    .expect("observer");
    let mut g = Graph::default();
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(10), |g| g
            .node_by_name("junc.oo.listen.hp")
            .is_some())
        .is_some(),
        "{}",
        read(&log)
    );
    std::thread::sleep(Duration::from_millis(300));
    let src = noise(RATE);
    let ratio = |rec: &[i16], c: usize| {
        rms_middle(&stereo_channel(rec, c)) / rms_middle(&stereo_channel(&src, c))
    };

    let rec = record_through(&dir, &src, "oo-player", "fake.hp", true);
    let (l, r) = (ratio(&rec, 0), ratio(&rec, 1));
    eprintln!("Listen links [[1, 1]]: left {l:.4}, right {r:.4}");
    assert!((l - 1.0).abs() < 0.02, "Bus channel 1 is heard: {l}");
    assert!(r < 1e-3, "Bus channel 2 is not: {r}");

    std::fs::write(dir.join("oo.toml"), config(false)).expect("hand edit");
    std::thread::sleep(Duration::from_millis(500));
    let off = level_through(&dir, &src, "oo-player", "junc.oo", false);
    eprintln!("Input off: level {off:.5}");
    assert!(off < 1e-3, "off is silent: {off}");

    std::fs::write(dir.join("oo.toml"), config(true)).expect("hand edit");
    std::thread::sleep(Duration::from_millis(500));
    let on = level_through(&dir, &src, "oo-player", "junc.oo", false);
    eprintln!("Input on again: level {on:.4}");
    assert!((on - 1.0).abs() < 0.02, "on again: {on}");
    drop(d);
    clean(&dir, &rx, &mut g);
    let _ = std::fs::remove_dir_all(&dir);
}

/// F11 (B2), with WirePlumber: an Exclusive Input takes the app off the
/// speakers while JUNC captures it, and turning Exclusive off brings it
/// back; JUNC's capture link stays throughout.
#[test]
fn exclusive_takes_the_app_off_the_speakers_and_back() {
    if !enabled() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("juncd-excl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let config = |exclusive: bool| {
        format!(
            "schema = 1\n[[junction]]\nid = \"ex\"\nname = \"Exclusive\"\n\
             [[junction.input]]\nid = \"player\"\nmatch = {{ app = \"ex-player\" }}\n\
             exclusive = {exclusive}\n"
        )
    };
    let (tx, rx) = channel();
    let _observer = PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    })
    .expect("observer");
    let mut g = Graph::default();
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(5), |g| g
            .node_by_name("fake.hp")
            .is_some())
        .is_some()
    );
    sh_ok(
        "wpctl",
        &["set-default", &node_id(&g, "fake.hp").to_string()],
    );
    // Where the app's audio goes: names of the nodes it is linked into.
    let into = |g: &Graph| -> BTreeSet<String> {
        let Some(p) = g.node_by_name("ex-player") else {
            return BTreeSet::new();
        };
        g.links
            .values()
            .filter(|l| l.output_node == p.id)
            .filter_map(|l| g.nodes.get(&l.input_node))
            .map(|n| n.name().to_owned())
            .collect()
    };
    let gain = "junc.ex.in.player".to_owned();
    let speakers_and_junc = BTreeSet::from(["fake.hp".to_owned(), gain.clone()]);
    let junc_only = BTreeSet::from([gain.clone()]);

    let player = pw_cat("-p", "ex-player", std::path::Path::new("/dev/zero"));
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(10), |g| into(g)
            .contains("fake.hp"))
        .is_some(),
        "WirePlumber did not route the player to the speakers: {:?}",
        into(&g)
    );

    let (d, log) = run_juncd(&dir, "ex", &config(true));
    let t0 = Instant::now();
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(10), |g| into(g)
            == junc_only)
        .is_some(),
        "Exclusive on: {:?}\n{}",
        into(&g),
        read(&log)
    );
    eprintln!("exclusive on: off the speakers after {:?}", t0.elapsed());

    std::fs::write(dir.join("ex.toml"), config(false)).expect("hand edit");
    let t0 = Instant::now();
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(10), |g| into(g)
            == speakers_and_junc)
        .is_some(),
        "Exclusive off: {:?}\n{}",
        into(&g),
        read(&log)
    );
    eprintln!(
        "exclusive off: back on the speakers after {:?}",
        t0.elapsed()
    );

    std::fs::write(dir.join("ex.toml"), config(true)).expect("hand edit");
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(10), |g| into(g)
            == junc_only)
        .is_some(),
        "Exclusive on again: {:?}",
        into(&g)
    );
    drop(player);
    drop(d);
    clean(&dir, &rx, &mut g);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A counter signal: frame k carries `k % 30000 + 1` on both channels, so
/// a recording shows any lost or repeated frame.
fn counter(frames: usize) -> Vec<i16> {
    (0..frames)
        .flat_map(|k| {
            let v = i16::try_from(k % 30_000 + 1).expect("fits");
            [v, v]
        })
        .collect()
}

/// The longest run of consecutive counter frames in a recording, and the
/// counter value it starts at.
fn longest_count(rec: &[i16]) -> (usize, i16) {
    let (mut best, mut best_first) = (0, 0i16);
    let (mut run, mut first, mut prev) = (0, 0i16, 0i16);
    for &[v, w] in rec.as_chunks::<2>().0 {
        if v == 0 || w != v {
            run = 0;
        } else if run > 0 && v == prev % 30_000 + 1 {
            run += 1;
        } else {
            run = 1;
            first = v;
        }
        prev = v;
        if run > best {
            best = run;
            best_first = first;
        }
    }
    (best, best_first)
}

/// F20 (B6): pacman replaces juncd while audio plays through a Junction.
/// juncd waits for the pacman lock, re-executes itself and adopts its
/// lingering nodes and links: the counter arrives without a gap, and no
/// JUNC object is recreated.
#[test]
fn an_upgrade_reexecs_without_an_audio_gap() {
    if !enabled() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("juncd-upgrade-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("bin")).expect("tmp dir");
    let exe = dir.join("bin/juncd");
    std::fs::copy(env!("CARGO_BIN_EXE_juncd"), &exe).expect("copy juncd");
    let lock = dir.join("db.lck");
    let cfg = dir.join("up.toml");
    std::fs::write(
        &cfg,
        "schema = 1\n[[junction]]\nid = \"up\"\nname = \"Upgrade\"\n\
         [[junction.input]]\nid = \"player\"\nmatch = { app = \"up-player\" }\n",
    )
    .expect("config");
    let log = dir.join("up.log");
    let daemon = Kill(
        Command::new(&exe)
            .args(["--no-dbus", "--config"])
            .arg(&cfg)
            .arg("--state-dir")
            .arg(&dir)
            .env("JUNC_PACMAN_LOCK", &lock)
            .stderr(std::fs::File::create(&log).expect("log"))
            .spawn()
            .expect("juncd"),
    );
    let pid = daemon.0.id();
    let (tx, rx) = channel();
    let _observer = PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    })
    .expect("observer");
    let mut g = Graph::default();
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(10), |g| g
            .node_by_name("junc.up.in.player")
            .is_some())
        .is_some(),
        "{}",
        read(&log)
    );
    let junc_objects = |g: &Graph| -> BTreeSet<u32> {
        g.nodes
            .values()
            .filter(|n| n.is_junc())
            .map(|n| n.id)
            .chain(g.links.values().filter(|l| l.is_junc()).map(|l| l.id))
            .collect()
    };

    let secs = 6;
    let src = counter(secs * RATE);
    let src_file = dir.join("counter.raw");
    let bytes: Vec<u8> = src.iter().flat_map(|s| s.to_le_bytes()).collect();
    std::fs::write(&src_file, bytes).expect("write source");
    let rec_file = dir.join("rec.raw");
    let recorder = pw_cat_to("-r", "up-rec", &rec_file, Some("junc.up"));
    std::thread::sleep(Duration::from_millis(500));
    let mut player = pw_cat_to("-p", "up-player", &src_file, None);
    assert!(
        wait_for(&rx, &mut g, Duration::from_secs(5), |g| g
            .node_by_name("up-player")
            .is_some_and(|p| g
                .links
                .values()
                .any(|l| l.output_node == p.id && l.is_junc())))
        .is_some()
    );
    let before = junc_objects(&g);

    // pacman takes its lock and puts a new juncd in place (write, rename).
    std::thread::sleep(Duration::from_secs(1));
    std::fs::write(&lock, "").expect("lock");
    let fresh = dir.join("bin/juncd.new");
    std::fs::copy(env!("CARGO_BIN_EXE_juncd"), &fresh).expect("copy");
    std::fs::rename(&fresh, &exe).expect("replace");
    std::thread::sleep(Duration::from_millis(800));
    assert!(
        !read(&log).contains("re-executing"),
        "must wait for the pacman lock:\n{}",
        read(&log)
    );
    std::fs::remove_file(&lock).expect("unlock");
    let until = Instant::now() + Duration::from_secs(5);
    while !read(&log).contains("re-executed after an upgrade") {
        assert!(Instant::now() < until, "no re-exec:\n{}", read(&log));
        std::thread::sleep(Duration::from_millis(50));
    }
    eprintln!("re-executed in place (pid {pid})");
    // Same objects while the player still plays: nothing was recreated.
    let _ = wait_for(&rx, &mut g, Duration::from_millis(700), |_| false);
    assert_eq!(junc_objects(&g), before, "{}", read(&log));

    let end = Instant::now() + Duration::from_secs(10);
    while player.0.try_wait().ok().flatten().is_none() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
    let _ = Command::new("kill")
        .arg("-INT")
        .arg(recorder.0.id().to_string())
        .status();
    std::thread::sleep(Duration::from_millis(300));
    drop(recorder);

    assert!(
        std::fs::metadata(format!("/proc/{pid}")).is_ok(),
        "juncd died:\n{}",
        read(&log)
    );
    let (run, first) = longest_count(&read_s16(&rec_file));
    let skipped = usize::try_from(first).expect("positive") - 1;
    eprintln!(
        "counter: {run} consecutive frames from frame {skipped} of {}",
        secs * RATE
    );
    // The player sounds on its default output for a moment before JUNC
    // links it, and pw-cat may drop its last partial buffer; everything in
    // between, across the re-exec at about 1.5 s, arrives unbroken.
    assert!(
        skipped < RATE / 2,
        "the Junction got the player late: {skipped}"
    );
    assert!(
        skipped + run + 2048 >= secs * RATE,
        "gap in the audio: {run} frames from {skipped}"
    );

    drop(daemon);
    clean(&dir, &rx, &mut g);
    let _ = std::fs::remove_dir_all(&dir);
}
