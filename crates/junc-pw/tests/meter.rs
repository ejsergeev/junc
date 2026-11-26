// SPDX-License-Identifier: GPL-3.0-or-later

//! The GUI meter against a live isolated PipeWire (hardware layer 2):
//! levels per channel, and a paused meter leaving nothing behind (no
//! stream, no links, no RT callbacks, no CPU) — the "CPU hidden 0 %" gate
//! of architecture §7.6. Runs only with `JUNC_LIVE_PIPEWIRE=1` via
//! `scripts/ci/live-pipewire.sh`; refuses a desktop session.

use junc_pw::meter::{Meter, NODE_NAME};
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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

/// A stereo playback stream named `name`: left at 0.5, right at 0.25 of
/// full scale, fed until it is killed.
fn tone(name: &str) -> Kill {
    let mut child = Command::new("pw-cat")
        .args([
            "-p",
            "--raw",
            "--format",
            "f32",
            "--channels",
            "2",
            "--rate",
            "48000",
            "-P",
            &format!("{{ node.name={name} }}"),
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("pw-cat");
    let mut stdin = child.stdin.take().expect("stdin");
    std::thread::spawn(move || {
        let frame: Vec<u8> = [0.5f32, -0.25f32]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let block = frame.repeat(4800);
        while stdin.write_all(&block).is_ok() {}
    });
    Kill(child)
}

/// Lines of `pw-dump` objects of a type whose props mention `needle`.
fn count(kind: &str, needle: &str) -> usize {
    let out = Command::new("pw-dump").output().expect("pw-dump");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    v.as_array()
        .map(|a| {
            a.iter()
                .filter(|o| o["type"].as_str().is_some_and(|t| t.ends_with(kind)))
                .filter(|o| o["info"]["props"].to_string().contains(needle))
                .count()
        })
        .unwrap_or(0)
}

/// Links into the meter stream, whoever made them.
fn links_into_meter() -> usize {
    let out = Command::new("pw-dump").output().expect("pw-dump");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    let all = v.as_array().cloned().unwrap_or_default();
    let Some(id) = all.iter().find_map(|o| {
        (o["info"]["props"]["node.name"].as_str() == Some(NODE_NAME)).then(|| o["id"].as_u64())?
    }) else {
        return 0;
    };
    all.iter()
        .filter(|o| o["info"]["input-node-id"].as_u64() == Some(id))
        .count()
}

/// CPU ticks (user + system) of this process's thread named `comm`.
fn thread_ticks(comm: &str) -> Option<u64> {
    for t in std::fs::read_dir("/proc/self/task").ok()?.flatten() {
        let p = t.path();
        if std::fs::read_to_string(p.join("comm")).ok()?.trim() != comm {
            continue;
        }
        let stat = std::fs::read_to_string(p.join("stat")).ok()?;
        let rest = stat.rsplit_once(')')?.1;
        let f: Vec<&str> = rest.split_whitespace().collect();
        // Fields 14 and 15 (utime, stime); `rest` starts at field 3.
        return Some(f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?);
    }
    None
}

/// Polls `read` until both channels of `name` show the tone, or fails.
fn wait_for_levels(meter: &Meter, name: &str, timeout: Duration) -> Vec<f32> {
    let start = Instant::now();
    let mut last = Vec::new();
    while start.elapsed() < timeout {
        std::thread::sleep(Duration::from_millis(150));
        if let Some((_, peaks)) = meter.read().into_iter().find(|(n, _)| n == name) {
            if peaks.len() == 2 && peaks[0] > 0.4 && peaks[1] > 0.2 {
                return peaks;
            }
            last = peaks;
        }
    }
    panic!("no levels for {name} after {timeout:?}: {last:?}");
}

fn wait_until(what: &str, timeout: Duration, f: impl Fn() -> bool) -> Duration {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < timeout, "{what} after {timeout:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
    start.elapsed()
}

#[test]
fn meters_levels_and_leaves_nothing_behind_when_paused() {
    if !enabled() {
        return;
    }
    let meter = Meter::spawn().expect("meter");
    // Watching before the node exists: it is linked once it appears.
    meter.watch(&["junc-meter-src".to_owned()]);
    let _src = tone("junc-meter-src");
    let peaks = wait_for_levels(&meter, "junc-meter-src", Duration::from_secs(5));
    assert!(
        (peaks[0] - 0.5).abs() < 0.01 && (peaks[1] - 0.25).abs() < 0.01,
        "left 0.5 and right 0.25 in their own channels: {peaks:?}"
    );
    assert_eq!(count("Node", NODE_NAME), 1, "one meter stream");
    assert_eq!(
        count("Link", "\"junc.role\":\"meter\""),
        2,
        "one tagged link per channel"
    );
    assert_eq!(
        links_into_meter(),
        2,
        "the session manager links nothing to it"
    );

    // Paused: the stream and its links go, callbacks and CPU stop.
    meter.watch(&[]);
    let gone = wait_until("meter stream still there", Duration::from_secs(1), || {
        count("Node", NODE_NAME) == 0
    });
    eprintln!("meter stream gone {gone:?} after the pause");
    assert_eq!(
        count("Link", "\"junc.role\":\"meter\""),
        0,
        "no meter links"
    );
    let (calls, ticks) = (meter.callbacks(), thread_ticks("junc-meter"));
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(meter.callbacks(), calls, "no RT callbacks while paused");
    assert_eq!(
        thread_ticks("junc-meter"),
        ticks,
        "the meter thread takes no CPU while paused"
    );
    assert!(meter.read().is_empty(), "nothing metered while paused");

    // Shown again: back without restarting anything.
    meter.watch(&["junc-meter-src".to_owned()]);
    wait_for_levels(&meter, "junc-meter-src", Duration::from_secs(2));
}

#[test]
fn a_missing_node_costs_nothing() {
    if !enabled() {
        return;
    }
    let meter = Meter::spawn().expect("meter");
    meter.watch(&["junc-meter-nobody".to_owned()]);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        count("Node", NODE_NAME),
        0,
        "no stream for nothing to meter"
    );
    assert!(meter.read().is_empty());
    assert_eq!(meter.callbacks(), 0);
}
