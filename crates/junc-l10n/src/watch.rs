// SPDX-License-Identifier: GPL-3.0-or-later

//! Following Omarchy's language live (F14): `omarchy-language` rewrites
//! `~/.config/omarchy/language` when the user picks a language under
//! Setup › Language. The directory is watched (files may be replaced by a
//! rename), and the thread blocks in `read`, so it costs nothing while
//! idle. Omarchy's own directory is never created.

use crate::{Sources, resolve};
use inotify::{Inotify, WatchMask};

/// Starts a thread that calls `changed` with the newly resolved language
/// whenever the Omarchy language file is written, replaced or removed and
/// the result differs from the last one. The thread lives as long as the
/// process. Fails if there is no Omarchy file location or its directory
/// does not exist (not an Omarchy system).
pub fn spawn(sources: Sources, changed: impl Fn(String) + Send + 'static) -> std::io::Result<()> {
    let not_found = || std::io::Error::from(std::io::ErrorKind::NotFound);
    let file = sources.omarchy_file.clone().ok_or_else(not_found)?;
    let dir = file.parent().ok_or_else(not_found)?.to_owned();
    let name = file.file_name().ok_or_else(not_found)?.to_os_string();
    let mut ino = Inotify::init()?;
    ino.watches().add(
        &dir,
        WatchMask::CLOSE_WRITE
            | WatchMask::MOVED_TO
            | WatchMask::CREATE
            | WatchMask::DELETE
            | WatchMask::MOVED_FROM,
    )?;
    let mut last = resolve(&sources);
    std::thread::Builder::new()
        .name("junc-language".into())
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
                let now = resolve(&sources);
                if now != last {
                    last.clone_from(&now);
                    changed(now);
                }
            }
        })?;
    Ok(())
}
