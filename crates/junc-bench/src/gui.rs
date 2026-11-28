// SPDX-License-Identifier: GPL-3.0-or-later

//! GUI performance (B26, architecture §11.4): the real `junc-gui`, its
//! timings turned on with `JUNC_GUI_PERF`, against the real juncd with ten Junctions on
//! the isolated PipeWire, its first Junction fed with noise so every
//! meter moves. Measured: cold start to the first frame with juncd's
//! state, PSS, CPU and wakeups with the meters running, frame times while
//! a link is dragged, and, under Hyprland, CPU while the window is on a
//! hidden special workspace.
//!
//! The window opens where the session puts it: under Xvfb in CI, or,
//! with `--hyprland WORKSPACE`, on that workspace through Hyprland's
//! `exec_cmd` (scripts/gui/perf.sh makes it a temporary headless
//! monitor's), without taking the user's focus.

use crate::gate::Metrics;
use crate::run::{
    Kill, log, private_bus_with, pw_cat, refs, rollup_kb, switches, top_mappings, unit_environment,
    wait_for,
};
use junc_core::graph::Graph;
use junc_pw::PwBackend;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How much to measure.
#[derive(Debug, Clone)]
pub struct Opts {
    /// Where juncd and junc-gui are.
    pub bins: PathBuf,
    /// Seconds to count CPU over, visible and hidden.
    pub secs: u64,
    /// Open the window on this Hyprland workspace.
    pub hyprland: Option<String>,
    /// The systemd unit whose environment juncd gets.
    pub unit: PathBuf,
}

const JUNCTIONS: usize = 10;

/// Ten Junctions; the first takes the noise app and the fake device and
/// is heard on the default output, so the Main screen shows all columns.
fn config() -> String {
    let mut s = String::from("schema = 1\n");
    for k in 0..JUNCTIONS {
        s.push_str(&format!(
            "[[junction]]\nid = \"g{k}\"\nname = \"Bench {k}\"\n"
        ));
        if k == 0 {
            s.push_str("[[junction.input]]\nid = \"noise\"\nmatch = { app = \"bench-gui\" }\n");
        }
        s.push_str(
            "[[junction.input]]\nid = \"mic\"\nmatch = { device = \"fake.hwin\" }\n\
             [[junction.listen]]\nid = \"hp\"\ntarget = \"default\"\n",
        );
    }
    s
}

fn routed(g: &Graph) -> bool {
    (0..JUNCTIONS).all(|k| {
        let r = refs(g, &format!("g{k}"));
        r.contains(&"sink/1") && r.contains(&"listen/hp/bus/1") && r.contains(&"in/mic/1->1")
    }) && refs(g, "g0").contains(&"in/noise/1->1")
}

/// A session bus with no service directories: nothing can be activated
/// on it (with the standard ones, the desktop portal of the user's
/// session was started once and crashed when the bus went away).
fn bus_config(dir: &Path) -> Result<PathBuf, String> {
    let file = dir.join("bus.conf");
    let text = format!(
        "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\"\n \
         \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n\
         <busconfig><type>session</type><listen>unix:dir={}</listen><auth>EXTERNAL</auth>\
         <policy context=\"default\"><allow send_destination=\"*\" eavesdrop=\"true\"/>\
         <allow eavesdrop=\"true\"/><allow own=\"*\"/></policy></busconfig>\n",
        dir.display()
    );
    std::fs::write(&file, text).map_err(|e| e.to_string())?;
    Ok(file)
}

/// User + system clock ticks of a process.
fn ticks(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The name in parentheses may hold spaces; fields count from after it.
    let rest = &stat[stat.rfind(')')? + 2..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    Some(f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?)
}

fn clock_ticks_per_s() -> f64 {
    Command::new("getconf")
        .arg("CLK_TCK")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(100.0)
}

/// Per thread: (tid, name, context switches, schedstat run time in ns).
fn threads(pid: u32) -> Vec<(String, String, u64, u64)> {
    let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) else {
        return Vec::new();
    };
    tasks
        .flatten()
        .map(|t| {
            let p = t.path();
            let read = |f: &str| std::fs::read_to_string(p.join(f)).unwrap_or_default();
            let switches = read("status")
                .lines()
                .filter(|l| l.contains("ctxt_switches"))
                .filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
                .sum();
            let run = read("schedstat")
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            (
                t.file_name().to_string_lossy().into_owned(),
                read("comm").trim().to_owned(),
                switches,
                run,
            )
        })
        .collect()
}

/// The threads that woke between two [`threads`] readings.
fn woke(before: &[(String, String, u64, u64)], after: &[(String, String, u64, u64)]) -> String {
    let mut out = Vec::new();
    for (tid, name, sw, run) in after {
        match before.iter().find(|b| &b.0 == tid) {
            Some(b) if b.2 != *sw => out.push(format!(
                "{name} ({tid}): {} switches, {} µs",
                sw - b.2,
                run.saturating_sub(b.3) / 1000
            )),
            Some(_) => {}
            None => out.push(format!("{name} ({tid}): new")),
        }
    }
    for b in before {
        if !after.iter().any(|a| a.0 == b.0) {
            out.push(format!("{} ({}): ended", b.1, b.0));
        }
    }
    if out.is_empty() {
        "none".to_owned()
    } else {
        out.join(", ")
    }
}

/// CPU of one core in percent and wakeups per second over `secs`.
fn cpu(pid: u32, secs: u64) -> Result<(f64, f64), String> {
    let (t0, s0) = (ticks(pid).ok_or("junc-gui exited")?, switches(pid));
    std::thread::sleep(Duration::from_secs(secs));
    let (t1, s1) = (ticks(pid).ok_or("junc-gui exited")?, switches(pid));
    #[allow(clippy::cast_precision_loss)]
    let pct = (t1 - t0) as f64 / clock_ticks_per_s() / secs as f64 * 100.0;
    #[allow(clippy::cast_precision_loss)]
    let wakeups = (s1 - s0) as f64 / secs as f64;
    Ok((pct, wakeups))
}

/// The `name=value` lines junc-gui wrote so far.
fn report(file: &Path) -> Metrics {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('='))
        .filter_map(|(k, v)| Some((k.to_owned(), v.trim().parse().ok()?)))
        .collect()
}

fn wait_report(file: &Path, key: &str, pid: u32, timeout: Duration) -> Result<Metrics, String> {
    let end = Instant::now() + timeout;
    loop {
        let r = report(file);
        if r.contains_key(key) {
            return Ok(r);
        }
        if ticks(pid).is_none() {
            return Err(format!("junc-gui exited before writing {key}"));
        }
        if Instant::now() > end {
            return Err(format!(
                "junc-gui wrote no {key} in {} s (a renderer without frame \
                 notifications, such as the software one?)",
                timeout.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `hyprctl` against the user's Hyprland: its runtime directory is in
/// `JUNC_BENCH_HYPR_RUNTIME` (ours is the isolated PipeWire's).
fn hyprctl(args: &[&str]) -> Result<String, String> {
    let mut c = Command::new("hyprctl");
    if let Ok(dir) = std::env::var("JUNC_BENCH_HYPR_RUNTIME") {
        c.env("XDG_RUNTIME_DIR", dir);
    }
    let out = c
        .args(args)
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("hyprctl: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A field of the first JSON object in `json` where `key` is `value`.
fn json_field(json: &str, key: &str, value: &serde_json::Value, field: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let found = match &v {
        serde_json::Value::Array(a) => a.iter().find(|o| o.get(key) == Some(value))?,
        o if o.get(key) == Some(value) => o,
        _ => return None,
    };
    found.get(field).map(|f| match f {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

fn window_address(pid: u32) -> Option<String> {
    json_field(
        &hyprctl(&["-j", "clients"]).ok()?,
        "pid",
        &serde_json::json!(pid),
        "address",
    )
}

fn active_address() -> Option<String> {
    let v: serde_json::Value =
        serde_json::from_str(&hyprctl(&["-j", "activewindow"]).ok()?).ok()?;
    v.get("address")?.as_str().map(str::to_owned)
}

fn move_window(address: &str, workspace: &str) -> Result<(), String> {
    hyprctl(&[
        "dispatch",
        &format!(
            "hl.dsp.window.move({{ workspace = \"{workspace}\", follow = false, \
             window = \"address:{address}\" }})"
        ),
    ])
    .map(drop)
}

fn newest_gui_after(since: SystemTime) -> Option<u32> {
    let out = Command::new("pgrep")
        .args(["-n", "-x", "junc-gui"])
        .output()
        .ok()?;
    let pid: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    let started = std::fs::metadata(format!("/proc/{pid}"))
        .ok()?
        .modified()
        .ok()?;
    (started + Duration::from_secs(1) >= since).then_some(pid)
}

/// Starts junc-gui; returns its pid and, when it was spawned here, the
/// child to stop.
fn launch(o: &Opts, env: &[(String, String)], log: &Path) -> Result<(u32, Option<Kill>), String> {
    let bin = o.bins.join("junc-gui");
    let Some(ws) = &o.hyprland else {
        let child = Command::new(&bin)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(log).map_err(|e| e.to_string())?)
            .spawn()
            .map_err(|e| format!("{}: {e}", bin.display()))?;
        return Ok((child.id(), Some(Kill(child))));
    };
    let before = active_address();
    let since = SystemTime::now();
    let vars: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    hyprctl(&[
        "dispatch",
        &format!(
            "hl.dsp.exec_cmd(\"env {} {}\", {{ workspace = \"{ws} silent\", \
             no_initial_focus = true }})",
            vars.join(" "),
            bin.display()
        ),
    ])?;
    let end = Instant::now() + Duration::from_secs(5);
    let pid = loop {
        if let Some(p) = newest_gui_after(since) {
            break p;
        }
        if Instant::now() > end {
            return Err("junc-gui did not start under Hyprland".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // Never leave the user's focus on the test window.
    let end = Instant::now() + Duration::from_secs(3);
    while window_address(pid).is_none() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    if let Some(b) = before
        && active_address().as_ref() != Some(&b)
    {
        let _ = hyprctl(&[
            "dispatch",
            &format!("hl.dsp.focus({{ window = \"address:{b}\" }})"),
        ]);
    }
    Ok((pid, None))
}

/// Runs everything and returns the metrics.
pub fn run(o: &Opts) -> Result<Metrics, String> {
    let mut m = Metrics::new();
    let dir = std::env::temp_dir().join(format!("junc-bench-gui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let cfg = dir.join("junctions.toml");
    std::fs::write(&cfg, config()).map_err(|e| e.to_string())?;
    let (_bus, address) = private_bus_with(&bus_config(&dir)?)?;

    let (tx, rx) = channel();
    let _observer = PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    })
    .map_err(|e| e.to_string())?;
    let mut g = Graph::default();
    wait_for(&rx, &mut g, Duration::from_secs(5), |g| !g.nodes.is_empty())
        .ok_or("no PipeWire graph (run under scripts/ci/with-pipewire.sh)")?;

    let juncd = o.bins.join("juncd");
    let _daemon = Kill(
        Command::new(&juncd)
            .envs(unit_environment(&o.unit))
            .arg("--config")
            .arg(&cfg)
            .arg("--state-dir")
            .arg(&dir)
            .env("DBUS_SESSION_BUS_ADDRESS", &address)
            .stderr(std::fs::File::create(dir.join("juncd.log")).map_err(|e| e.to_string())?)
            .spawn()
            .map_err(|e| format!("{}: {e}", juncd.display()))?,
    );
    let _noise = pw_cat(
        &["-p"],
        "{ node.name=bench-gui application.name=bench-gui }",
        Path::new("/dev/urandom"),
    )?;
    wait_for(&rx, &mut g, Duration::from_secs(10), routed)
        .ok_or("juncd did not route the Junctions")?;

    // The GUI's own files stay in the bench's folder: never the user's
    // state, config or language.
    let perf = dir.join("perf.txt");
    let pipewire = std::env::var("PIPEWIRE_RUNTIME_DIR")
        .or_else(|_| std::env::var("XDG_RUNTIME_DIR"))
        .map_err(|_| "no PipeWire runtime directory")?;
    let warmup = 3;
    let t0 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?;
    let env = vec![
        ("DBUS_SESSION_BUS_ADDRESS".into(), address.clone()),
        ("PIPEWIRE_RUNTIME_DIR".into(), pipewire),
        (
            "XDG_STATE_HOME".into(),
            dir.join("state").display().to_string(),
        ),
        (
            "XDG_CONFIG_HOME".into(),
            dir.join("config").display().to_string(),
        ),
        ("JUNC_GUI_PERF".into(), perf.display().to_string()),
        ("JUNC_GUI_PERF_T0".into(), t0.as_nanos().to_string()),
        (
            "JUNC_GUI_PERF_DRAG_MS".into(),
            ((warmup + o.secs + 1) * 1000).to_string(),
        ),
    ];
    log("phase: cold start");
    let gui_log = dir.join("junc-gui.log");
    let (pid, _gui) = launch(o, &env, &gui_log)?;
    let r = wait_report(&perf, "first_frame_ms", pid, Duration::from_secs(20)).map_err(|e| {
        let text = std::fs::read_to_string(&gui_log).unwrap_or_default();
        format!("{e}\njunc-gui's log:\n{text}")
    })?;
    m.insert("gui_first_frame_ms".into(), r["first_frame_ms"]);
    let mut steps: Vec<(&String, &f64)> = r.iter().filter(|(k, _)| k.ends_with("_ms")).collect();
    steps.sort_by(|a, b| a.1.total_cmp(b.1));
    log(&format!(
        "cold start steps (ms since launch): {}",
        steps
            .iter()
            .map(|(k, v)| format!("{} {v:.0}", k.trim_end_matches("_ms")))
            .collect::<Vec<_>>()
            .join(", ")
    ));

    std::thread::sleep(Duration::from_secs(warmup));
    m.insert(
        "gui_pss_kb".into(),
        rollup_kb(pid, "Pss").ok_or("cannot read the GUI's PSS")?,
    );
    log(&format!("largest GUI mappings: {}", top_mappings(pid)));

    log("phase: meters visible");
    let (pct, wakeups) = cpu(pid, o.secs)?;
    m.insert("gui_cpu_meters_pct".into(), pct);
    m.insert("gui_wakeups_meters_per_s".into(), wakeups);

    log("phase: drag");
    let r = wait_report(&perf, "done", pid, Duration::from_secs(30))?;
    if r.contains_key("drag_error") {
        return Err("junc-gui found no port to drag a link from".into());
    }
    for k in [
        "drag_frames",
        "drag_render_p95_ms",
        "drag_render_max_ms",
        "drag_interval_p95_ms",
    ] {
        if let Some(v) = r.get(k) {
            m.insert(format!("gui_{k}"), *v);
        }
    }

    if o.hyprland.is_some() {
        log("phase: hidden");
        let addr = window_address(pid).ok_or("the window is not in Hyprland's list")?;
        move_window(&addr, "special:junc-bench")?;
        std::thread::sleep(Duration::from_millis(1500));
        let before = threads(pid);
        let (pct, wakeups) = cpu(pid, o.secs)?;
        log(&format!(
            "threads that woke while hidden: {}",
            woke(&before, &threads(pid))
        ));
        m.insert("gui_cpu_hidden_pct".into(), pct);
        m.insert("gui_wakeups_hidden_per_s".into(), wakeups);
        if let Some(ws) = &o.hyprland {
            move_window(&addr, ws)?;
        }
    }
    if o.hyprland.is_some() {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_is_valid() {
        let doc = junc_core::config::load(&config(), "bench.toml");
        assert!(doc.is_ok(), "{doc:?}");
    }

    #[test]
    fn reports_are_read_line_by_line() {
        let dir = std::env::temp_dir().join(format!("junc-bench-report-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let f = dir.join("perf.txt");
        std::fs::write(&f, "first_frame_ms=123.4\nnote=text\ndone=1\n").expect("write");
        let r = report(&f);
        assert_eq!(r.get("first_frame_ms"), Some(&123.4));
        assert_eq!(r.get("done"), Some(&1.0));
        assert!(!r.contains_key("note"), "only numbers");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
