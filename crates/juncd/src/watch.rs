// SPDX-License-Identifier: GPL-3.0-or-later

//! Watching `junctions.toml` for hand edits (F22). Editors replace files
//! (write a temporary file, rename it over), so the directory is watched,
//! not the file. The thread blocks in `read`, so it costs nothing while idle.

use inotify::{Inotify, WatchMask};
use std::path::Path;

/// Starts a thread that calls `changed` whenever the file is written,
/// replaced or removed.
pub fn spawn(file: &Path, changed: impl Fn() + Send + 'static) -> std::io::Result<()> {
    let dir = file
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = file
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    let mut ino = Inotify::init()?;
    ino.watches().add(
        dir,
        WatchMask::CLOSE_WRITE
            | WatchMask::MOVED_TO
            | WatchMask::CREATE
            | WatchMask::DELETE
            | WatchMask::MOVED_FROM,
    )?;
    std::thread::Builder::new()
        .name("junc-watch".into())
        .spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                let Ok(events) = ino.read_events_blocking(&mut buf) else {
                    return;
                };
                // One notification per batch is enough: the handler re-reads.
                if events
                    .into_iter()
                    .any(|e| e.name.is_some_and(|n| n == name))
                {
                    changed();
                }
            }
        })?;
    Ok(())
}
