// SPDX-License-Identifier: GPL-3.0-or-later

//! Upgrade without an audio drop (F20, docs/architecture.md §3.13). The
//! daemon watches its own executable. When a package upgrade replaces it
//! (a new inode at the same path) and pacman has released its lock, the
//! daemon re-executes itself; JUNC's nodes and links linger in PipeWire, so
//! the new process adopts them and audio keeps running.
//!
//! The thread blocks in `read`, so it costs nothing while idle; it polls the
//! lock only while an upgrade is under way.

use inotify::{Inotify, WatchMask};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where pacman holds its lock during a transaction; tests point it
/// elsewhere with `JUNC_PACMAN_LOCK`.
fn lock_path() -> PathBuf {
    std::env::var_os("JUNC_PACMAN_LOCK")
        .map_or_else(|| PathBuf::from("/var/lib/pacman/db.lck"), PathBuf::from)
}

fn identity(path: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(path).ok()?;
    (m.is_file() && m.len() > 0).then(|| (m.dev(), m.ino()))
}

/// Starts watching `exe` (the path this process runs from); calls
/// `replaced` once when it has been replaced and the package manager is
/// done.
pub fn spawn(exe: &Path, replaced: impl Fn() + Send + 'static) -> std::io::Result<()> {
    let running = identity(exe).ok_or_else(|| std::io::Error::other("executable not found"))?;
    let dir = exe.parent().unwrap_or(Path::new("/")).to_owned();
    let name = exe
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    let exe = exe.to_owned();
    let mut ino = Inotify::init()?;
    ino.watches().add(
        &dir,
        WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO | WatchMask::CREATE,
    )?;
    std::thread::Builder::new()
        .name("junc-upgrade".into())
        .spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                let Ok(events) = ino.read_events_blocking(&mut buf) else {
                    return;
                };
                if !events
                    .into_iter()
                    .any(|e| e.name.is_some_and(|n| n == name))
                {
                    continue;
                }
                // Wait for the transaction to finish: other files of the
                // package (libraries, the CLI) may not be in place yet.
                while lock_path().exists() {
                    std::thread::sleep(Duration::from_millis(500));
                }
                match identity(&exe) {
                    Some(now) if now != running => {
                        replaced();
                        return;
                    }
                    _ => {}
                }
            }
        })?;
    Ok(())
}
