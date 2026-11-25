// SPDX-License-Identifier: GPL-3.0-or-later

//! The GUI follows juncd live (architecture §7.1): a change made by another
//! client appears without a reload, and juncd going away is shown. juncd
//! runs on a private bus without PipeWire (the "mock daemon" of the CLI
//! tests); `cargo test --workspace` builds it.

use junc_gui::{daemon, state};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn juncd() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_BIN_EXE_junc-gui")).with_file_name("juncd");
    assert!(
        path.exists(),
        "{} is missing; run cargo test --workspace",
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

/// Waits for a snapshot that satisfies `want`.
fn until(
    rx: &Receiver<state::Snapshot>,
    want: impl Fn(&state::Snapshot) -> bool,
) -> state::Snapshot {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let left = end.saturating_duration_since(Instant::now());
        let s = rx
            .recv_timeout(left)
            .expect("no matching snapshot within 10 s");
        if want(&s) {
            return s;
        }
    }
}

#[test]
fn the_window_follows_juncd_live() {
    let Some((_bus, address)) = private_bus() else {
        assert!(
            std::env::var_os("CI").is_none(),
            "dbus-daemon is missing in CI"
        );
        eprintln!("skipped: no dbus-daemon");
        return;
    };
    let dir = std::env::temp_dir().join(format!("junc-gui-daemon-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("run")).expect("dir");
    let config = dir.join("junctions.toml");
    std::fs::write(
        &config,
        "schema = 1\n[[junction]]\nid = \"obs\"\nname = \"OBS Mix\"\n",
    )
    .expect("config");
    let start = || {
        Kill(
            Command::new(juncd())
                .arg("--config")
                .arg(&config)
                .arg("--state-dir")
                .arg(&dir)
                .env("DBUS_SESSION_BUS_ADDRESS", &address)
                .env("PIPEWIRE_RUNTIME_DIR", dir.join("run"))
                .env_remove("PIPEWIRE_REMOTE")
                .stderr(Stdio::null())
                .spawn()
                .expect("juncd"),
        )
    };
    let daemon = start();

    let (tx, rx) = channel();
    daemon::follow(Some(address.clone()), move |d| {
        let _ = tx.send(state::snapshot(d.as_ref()));
    })
    .expect("follow");
    let names = |s: &state::Snapshot| {
        s.junctions
            .iter()
            .map(|r| r.name.clone())
            .collect::<Vec<_>>()
    };
    let first = until(&rx, |s| s.daemon != "down");
    assert_eq!(names(&first), ["OBS Mix"]);

    // Another client (the CLI, an agent) creates a Junction.
    zbus::block_on(async {
        let conn = zbus::connection::Builder::address(address.as_str())
            .build()
            .await
            .expect("bus");
        let p = junc_ipc::DaemonProxy::new(&conn).await.expect("proxy");
        p.create_junction("Zoom Mic").await.expect("create");
    });
    let s = until(&rx, |s| s.junctions.len() == 2);
    assert_eq!(names(&s), ["OBS Mix", "Zoom Mic"]);

    // juncd stops: the window says so; it comes back: the list returns.
    drop(daemon);
    until(&rx, |s| s.daemon == "down");
    let _again = start();
    let back = until(&rx, |s| s.daemon != "down");
    assert_eq!(names(&back), ["OBS Mix", "Zoom Mic"]);
    let _ = std::fs::remove_dir_all(&dir);
}
