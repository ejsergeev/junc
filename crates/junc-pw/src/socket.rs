// SPDX-License-Identifier: GPL-3.0-or-later

//! Waiting for the PipeWire socket while disconnected (§3.6).
//!
//! The socket path follows libpipewire: `$PIPEWIRE_RUNTIME_DIR` or
//! `$XDG_RUNTIME_DIR`, then `$PIPEWIRE_REMOTE` or `pipewire-0`. An inotify
//! watch on the directory wakes us when the socket appears; without inotify
//! we fall back to the validated poll (20 ms for 2 s, then 500 ms).

use inotify::{Inotify, WatchMask};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Path of the PipeWire socket this process would connect to.
pub fn socket_path() -> Option<PathBuf> {
    let dir =
        std::env::var_os("PIPEWIRE_RUNTIME_DIR").or_else(|| std::env::var_os("XDG_RUNTIME_DIR"))?;
    let name = std::env::var("PIPEWIRE_REMOTE").unwrap_or_else(|_| "pipewire-0".into());
    Some(PathBuf::from(dir).join(name))
}

/// Blocks until the socket exists or `stop` is set. Returns false on stop.
pub fn wait_for_socket(stop: &AtomicBool) -> bool {
    let Some(path) = socket_path() else {
        return poll_wait(stop, None);
    };
    if path.exists() {
        return !stop.load(Ordering::SeqCst);
    }
    let watch = path.parent().and_then(|dir| {
        let ino = Inotify::init().ok()?;
        ino.watches()
            .add(dir, WatchMask::CREATE | WatchMask::MOVED_TO)
            .ok()?;
        Some(ino)
    });
    let Some(mut ino) = watch else {
        return poll_wait(stop, Some(&path));
    };
    let mut buf = [0u8; 1024];
    loop {
        if stop.load(Ordering::SeqCst) {
            return false;
        }
        if path.exists() {
            return true;
        }
        // Wake at least once a second so a shutdown request is noticed.
        let mut fds = libc::pollfd {
            fd: ino.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd for the duration of the call.
        let ready = unsafe { libc::poll(&raw mut fds, 1, 1000) };
        if ready > 0 {
            let _ = ino.read_events(&mut buf);
        }
    }
}

fn poll_wait(stop: &AtomicBool, path: Option<&PathBuf>) -> bool {
    for attempt in 0u32.. {
        if stop.load(Ordering::SeqCst) {
            return false;
        }
        if path.is_none_or(|p| p.exists()) && attempt > 0 {
            return true;
        }
        let ms = if attempt < 100 { 20 } else { 500 };
        std::thread::sleep(Duration::from_millis(ms));
    }
    false
}
