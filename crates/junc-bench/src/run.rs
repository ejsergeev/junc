// SPDX-License-Identifier: GPL-3.0-or-later

//! Measurements of the real juncd on an isolated PipeWire
//! (scripts/ci/with-pipewire.sh), on a private D-Bus as in a session.

use crate::gate::Metrics;
use junc_core::graph::Graph;
use junc_pw::{Event, PwBackend};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

/// How much to measure.
#[derive(Debug, Clone)]
pub struct Opts {
    /// Where juncd and junc are.
    pub bins: PathBuf,
    /// Seconds of idle to count wakeups over.
    pub idle_secs: u64,
    /// App restarts for the re-link measure.
    pub churn: usize,
    /// PipeWire restarts for the recovery measure.
    pub recoveries: usize,
    /// `junc enable` calls for the CLI measure.
    pub cli_runs: usize,
    /// The systemd unit whose environment juncd gets.
    pub unit: PathBuf,
}

const JUNCTIONS: usize = 10;
const RATE: usize = 48_000;

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn log(msg: &str) {
    eprintln!("junc-bench: {msg}");
}

/// Ten Junctions, nine on the fake device and one on the bench app, each
/// heard on the default output.
fn config() -> String {
    let mut s = String::from("schema = 1\n");
    for k in 0..JUNCTIONS {
        let input = if k + 1 == JUNCTIONS {
            "id = \"app\"\nmatch = { app = \"bench-app\" }"
        } else {
            "id = \"mic\"\nmatch = { device = \"fake.hwin\" }"
        };
        s.push_str(&format!(
            "[[junction]]\nid = \"b{k}\"\nname = \"Bench {k}\"\n\
             [[junction.input]]\n{input}\n\
             [[junction.listen]]\nid = \"hp\"\ntarget = \"default\"\n"
        ));
    }
    s
}

fn refs<'a>(g: &'a Graph, junction: &str) -> Vec<&'a str> {
    g.links
        .values()
        .filter(|l| l.props.get("junc.junction") == Some(junction))
        .filter_map(|l| l.props.get("junc.ref"))
        .collect()
}

/// Every Junction exists and is wired; the device ones are routed.
fn all_routed(g: &Graph) -> bool {
    (0..JUNCTIONS).all(|k| {
        let r = refs(g, &format!("b{k}"));
        r.contains(&"sink/1")
            && r.contains(&"listen/hp/bus/1")
            && (k + 1 == JUNCTIONS || r.contains(&"in/mic/1->1"))
    })
}

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

fn private_bus() -> Result<(Kill, String), String> {
    let mut child = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--nopidfile", "--print-address=1"])
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("dbus-daemon: {e}"))?;
    let mut line = String::new();
    BufReader::new(child.stdout.take().ok_or("dbus-daemon stdout")?)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    Ok((Kill(child), line.trim().to_owned()))
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let i = ((v.len() as f64 * p).ceil() as usize).clamp(1, v.len()) - 1;
    v[i]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// `Environment=K=V` lines of a systemd unit.
fn unit_environment(unit: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(unit)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().strip_prefix("Environment="))
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

/// A field of `/proc/<pid>/smaps_rollup`, in kB.
fn rollup_kb(pid: u32, field: &str) -> Option<f64> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix(field)?.strip_prefix(':'))
        .and_then(|v| v.trim().trim_end_matches(" kB").trim().parse().ok())
}

/// Mappings backed by transparent huge pages: range, size, flags.
fn huge_mappings(pid: u32) -> String {
    let text = std::fs::read_to_string(format!("/proc/{pid}/smaps")).unwrap_or_default();
    let mut out = Vec::new();
    let (mut head, mut huge, mut size) = (String::new(), 0u64, String::new());
    for l in text.lines() {
        let first = l.split_whitespace().next().unwrap_or("");
        if first.contains('-') && first.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            head = l.to_owned();
            huge = 0;
        } else if let Some(v) = l.strip_prefix("Size:") {
            size = v.trim().to_owned();
        } else if let Some(v) = l.strip_prefix("AnonHugePages:") {
            huge = v.trim().trim_end_matches(" kB").trim().parse().unwrap_or(0);
        } else if let Some(flags) = l.strip_prefix("VmFlags:")
            && huge > 0
        {
            out.push(format!(
                "[{head}] size {size}, huge {huge} kB, flags{flags}"
            ));
        }
    }
    out.join("; ")
}

/// The mappings that weigh most, for the log: where memory goes.
fn top_mappings(pid: u32) -> String {
    let text = std::fs::read_to_string(format!("/proc/{pid}/smaps")).unwrap_or_default();
    let mut maps: Vec<(u64, String)> = Vec::new();
    let mut name = String::new();
    for l in text.lines() {
        let first = l.split_whitespace().next().unwrap_or("");
        if first.contains('-') && first.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            name = l.split_whitespace().nth(5).unwrap_or("[anon]").to_owned();
        } else if let Some(v) = l.strip_prefix("Pss:") {
            let kb: u64 = v.trim().trim_end_matches(" kB").trim().parse().unwrap_or(0);
            match maps.iter_mut().find(|(_, n)| *n == name) {
                Some((k, _)) => *k += kb,
                None => maps.push((kb, name.clone())),
            }
        }
    }
    maps.sort_by_key(|m| std::cmp::Reverse(m.0));
    maps.iter()
        .take(8)
        .map(|(k, n)| format!("{k} kB {n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Context switches of all threads of a process: each one is a wakeup.
fn switches(pid: u32) -> u64 {
    let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) else {
        return 0;
    };
    tasks
        .flatten()
        .filter_map(|t| std::fs::read_to_string(t.path().join("status")).ok())
        .flat_map(|s| {
            s.lines()
                .filter(|l| l.contains("ctxt_switches"))
                .filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
                .collect::<Vec<_>>()
        })
        .sum()
}

fn noise(frames: usize) -> Vec<i16> {
    let mut x: u32 = 0x9e37_79b9;
    (0..frames * 2)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            i16::from_le_bytes([(x >> 16) as u8, (x >> 24) as u8]) | 1
        })
        .collect()
}

fn pw_cat(args: &[&str], props: &str, file: &Path) -> Result<Kill, String> {
    Command::new("pw-cat")
        .args(args)
        .args([
            "--raw",
            "--format",
            "s16",
            "--rate",
            "48000",
            "--channels",
            "2",
        ])
        .args(["--channel-map", "FL,FR", "-P", props])
        .arg(file)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(Kill)
        .map_err(|e| format!("pw-cat: {e}"))
}

fn link(from: &str, to: &str) -> Result<(), String> {
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end {
        let ok = Command::new("pw-link")
            .args([from, to])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(format!("cannot link {from} → {to}"))
}

/// Whether audio through a Junction is unchanged: one recorder hears the
/// player directly on its left channel and through the Junction on its
/// right; the right channel must be the left one, shifted. (The shift
/// mixes in how the graph orders the two pw-cat clients, so the latency
/// comes from [`crate::latency`] instead.)
fn bit_exact(dir: &Path) -> Result<f64, String> {
    let src = noise(2 * RATE);
    let src_file = dir.join("src.raw");
    let bytes: Vec<u8> = src.iter().flat_map(|s| s.to_le_bytes()).collect();
    std::fs::write(&src_file, bytes).map_err(|e| e.to_string())?;
    let rec_file = dir.join("rec.raw");
    let recorder = pw_cat(
        &["-r"],
        "{ node.name=bench-lat application.name=bench-lat node.autoconnect=false }",
        &rec_file,
    )?;
    std::thread::sleep(Duration::from_millis(300));
    let mut player = pw_cat(
        &["-p"],
        "{ node.name=bench-play application.name=bench-app }",
        &src_file,
    )?;
    link("bench-play:output_FL", "bench-lat:input_FL")?;
    link("junc.b9:capture_FL", "bench-lat:input_FR")?;
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

    let raw = std::fs::read(&rec_file).map_err(|e| e.to_string())?;
    let samples: Vec<i16> = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b))
        .collect();
    let direct: Vec<i16> = samples.iter().step_by(2).copied().collect();
    let via: Vec<i16> = samples.iter().skip(1).step_by(2).copied().collect();
    // A stretch of the direct channel that is surely noise, found again in
    // the channel through the Junction.
    let start = direct
        .iter()
        .position(|s| *s != 0)
        .ok_or("the recorder heard nothing directly")?
        + RATE / 4;
    let window = direct
        .get(start..start + 960)
        .ok_or("recording too short")?;
    let at = via
        .windows(window.len())
        .position(|w| w == window)
        .ok_or("the audio through the Junction does not match the direct audio")?;
    #[allow(clippy::cast_precision_loss)]
    let shift = at as f64 - start as f64;
    let n = (direct.len() - start).min(via.len() - at);
    let same = direct[start..start + n]
        .iter()
        .zip(&via[at..at + n])
        .take_while(|(a, b)| a == b)
        .count();
    // The player stops before the recorder: the rest may be a silent tail.
    let tail_silent = via[at + same..at + n].iter().all(|s| *s == 0)
        || direct[start + same..start + n].iter().all(|s| *s == 0);
    let exact = same >= RATE / 2 && tail_silent;
    log(&format!(
        "bit-exact: {same} samples identical (pw-cat shift {shift}, not a latency)"
    ));
    Ok(if exact { 1.0 } else { 0.0 })
}

/// Runs everything and returns the metrics.
pub fn run(o: &Opts) -> Result<Metrics, String> {
    let mut m = Metrics::new();
    let juncd = o.bins.join("juncd");
    let junc = o.bins.join("junc");
    let size = std::fs::metadata(&juncd)
        .map_err(|e| format!("{}: {e}", juncd.display()))?
        .len();
    #[allow(clippy::cast_precision_loss)]
    m.insert("binary_size_bytes".into(), size as f64);

    let dir = std::env::temp_dir().join(format!("junc-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let cfg = dir.join("junctions.toml");
    std::fs::write(&cfg, config()).map_err(|e| e.to_string())?;
    let (_bus, address) = private_bus()?;

    let (tx, rx) = channel();
    let _observer = PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    })
    .map_err(|e| e.to_string())?;
    let mut g = Graph::default();
    wait_for(&rx, &mut g, Duration::from_secs(5), |g| !g.nodes.is_empty())
        .ok_or("no PipeWire graph (run under scripts/ci/with-pipewire.sh)")?;

    // Restore: process start to every Junction routed. juncd runs with the
    // environment its systemd unit gives it, as shipped.
    let unit_env = unit_environment(&o.unit);
    log(&format!(
        "juncd environment from {}: {unit_env:?}",
        o.unit.display()
    ));
    let t = Instant::now();
    let daemon = Kill(
        Command::new(&juncd)
            .envs(unit_env.iter().map(|(k, v)| (k, v)))
            .arg("--config")
            .arg(&cfg)
            .arg("--state-dir")
            .arg(&dir)
            .env("DBUS_SESSION_BUS_ADDRESS", &address)
            .stderr(std::fs::File::create(dir.join("juncd.log")).map_err(|e| e.to_string())?)
            .spawn()
            .map_err(|e| format!("juncd: {e}"))?,
    );
    let pid = daemon.0.id();
    let done = wait_for(&rx, &mut g, Duration::from_secs(10), all_routed)
        .ok_or("juncd did not restore the Junctions")?;
    m.insert("restore_ms".into(), ms(done - t));

    // Memory and idle cost with ten Junctions.
    std::thread::sleep(Duration::from_secs(1));
    m.insert(
        "pss_kb".into(),
        rollup_kb(pid, "Pss").ok_or("cannot read PSS")?,
    );
    // Tracked, not gated: what the PSS is made of.
    for (metric, field) in [
        ("rss_kb", "Rss"),
        ("pss_anon_kb", "Pss_Anon"),
        ("pss_file_kb", "Pss_File"),
        ("pss_shmem_kb", "Pss_Shmem"),
        ("anon_huge_kb", "AnonHugePages"),
    ] {
        if let Some(v) = rollup_kb(pid, field) {
            m.insert(metric.into(), v);
        }
    }
    log(&format!("largest mappings: {}", top_mappings(pid)));
    log(&format!("huge-page mappings: {}", huge_mappings(pid)));
    let read = |p: &str| {
        std::fs::read_to_string(p)
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    let threads = std::fs::read_dir(format!("/proc/{pid}/task")).map_or(0, Iterator::count);
    let page = Command::new("getconf")
        .arg("PAGESIZE")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    log(&format!(
        "memory context: {threads} threads, page size {page}, THP {}, cpus {}",
        read("/sys/kernel/mm/transparent_hugepage/enabled"),
        std::thread::available_parallelism().map_or(0, std::num::NonZero::get)
    ));
    let before = switches(pid);
    std::thread::sleep(Duration::from_secs(o.idle_secs));
    #[allow(clippy::cast_precision_loss)]
    let rate = (switches(pid) - before) as f64 / o.idle_secs as f64;
    m.insert("idle_wakeups_per_s".into(), rate);

    // CLI round trip.
    let mut cli = Vec::new();
    for i in 0..o.cli_runs {
        let t = Instant::now();
        let ok = Command::new(&junc)
            .args([if i % 2 == 0 { "disable" } else { "enable" }, "b0"])
            .env("DBUS_SESSION_BUS_ADDRESS", &address)
            .stdout(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            return Err("junc enable/disable failed".into());
        }
        cli.push(ms(t.elapsed()));
    }
    m.insert("cli_enable_p95_ms".into(), percentile(&mut cli, 0.95));
    let routed_again = wait_for(&rx, &mut g, Duration::from_secs(5), all_routed);
    routed_again.ok_or("not routed after the CLI calls")?;

    // Re-link: an app that restarts over and over.
    let serials = |g: &Graph| -> Vec<u32> {
        g.nodes
            .values()
            .filter(|n| n.name() == "bench-app")
            .filter_map(|n| n.props.number("object.serial"))
            .collect()
    };
    let mut relink = Vec::new();
    for i in 0..o.churn {
        let old = serials(&g);
        let app = pw_cat(
            &["-p"],
            "{ node.name=bench-app application.name=bench-app }",
            Path::new("/dev/zero"),
        )?;
        let seen = wait_for(&rx, &mut g, Duration::from_secs(5), |g| {
            serials(g).iter().any(|s| !old.contains(s))
        })
        .ok_or(format!("restart {i}: app never appeared"))?;
        let linked = wait_for(&rx, &mut g, Duration::from_secs(2), |g| {
            refs(g, "b9")
                .iter()
                .filter(|r| r.starts_with("in/app/"))
                .count()
                == 2
        })
        .ok_or(format!("restart {i}: not re-linked within 2 s"))?;
        relink.push(ms(linked - seen));
        drop(app);
    }
    m.insert("relink_p95_ms".into(), percentile(&mut relink, 0.95));

    // Recovery: PipeWire restarts under the daemon.
    let session = std::env::var("JUNC_PW_SESSION")
        .map_err(|_| "JUNC_PW_SESSION is not set (run under scripts/ci/with-pipewire.sh)")?;
    let (mut recovery, mut total) = (Vec::new(), Vec::new());
    for i in 0..o.recoveries {
        let ok = |arg: &str| {
            Command::new(&session)
                .arg(arg)
                .stdout(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        };
        if !ok("stop") {
            return Err("pw-session.sh stop".into());
        }
        // Snapshots of the old instance may still be queued; only what
        // arrives after the observer lost PipeWire counts.
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            match rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
                Ok(Event::Disconnected) => break,
                Ok(_) => {}
                Err(_) => return Err(format!("recovery {i}: the observer never lost PipeWire")),
            }
        }
        std::thread::sleep(Duration::from_millis(300));
        let t = Instant::now();
        if !ok("start") {
            return Err("pw-session.sh start".into());
        }
        // Two clocks: from PipeWire's start (includes WirePlumber bringing
        // the devices and the default output back), and from the moment
        // everything juncd needs is there again (juncd's own share).
        let mut fresh = Graph::default();
        let ready = wait_for(&rx, &mut fresh, Duration::from_secs(10), |g| {
            g.node_by_name("fake.hwin").is_some() && g.defaults.sink.is_some()
        })
        .ok_or(format!("recovery {i}: the devices never came back"))?;
        let back = wait_for(&rx, &mut fresh, Duration::from_secs(10), all_routed)
            .ok_or(format!("recovery {i}: not rebuilt"))?;
        total.push(ms(back - t));
        recovery.push(ms(back.saturating_duration_since(ready)));
        std::thread::sleep(Duration::from_millis(1000));
    }
    m.insert("recovery_max_ms".into(), percentile(&mut recovery, 1.0));
    m.insert("recovery_total_max_ms".into(), percentile(&mut total, 1.0));
    if daemon.0.id() != pid || std::fs::metadata(format!("/proc/{pid}")).is_err() {
        return Err("juncd did not survive the PipeWire restarts".into());
    }

    let (added, quantum) =
        crate::latency::added_frames("junc.b9.in.app:playback_FL", "junc.b9:capture_FL", 4)?;
    log(&format!(
        "latency: the Junction adds {added} frames (quantum {quantum})"
    ));
    m.insert("added_latency_frames".into(), added);
    m.insert("bit_exact".into(), bit_exact(&dir)?);

    drop(daemon);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(m)
}
