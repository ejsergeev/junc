// SPDX-License-Identifier: GPL-3.0-or-later

//! systemd readiness and status (`Type=notify`, sd_notify(3)) without
//! libsystemd: one datagram per message to `$NOTIFY_SOCKET`. Outside systemd
//! the variable is unset and every call does nothing.

use std::os::unix::net::{SocketAddr, UnixDatagram};

/// Where systemd listens, if it does.
#[derive(Debug)]
pub struct Notifier {
    socket: Option<(UnixDatagram, SocketAddr)>,
}

impl Notifier {
    /// Reads `$NOTIFY_SOCKET` (a path, or `@name` for the abstract namespace).
    pub fn from_env() -> Self {
        let socket = std::env::var("NOTIFY_SOCKET").ok().and_then(|path| {
            let addr = match path.strip_prefix('@') {
                Some(name) => {
                    use std::os::linux::net::SocketAddrExt;
                    SocketAddr::from_abstract_name(name.as_bytes()).ok()?
                }
                None => SocketAddr::from_pathname(&path).ok()?,
            };
            Some((UnixDatagram::unbound().ok()?, addr))
        });
        Self { socket }
    }

    fn send(&self, msg: &str) {
        if let Some((sock, addr)) = &self.socket
            && let Err(e) = sock.send_to_addr(msg.as_bytes(), addr)
        {
            eprintln!("juncd: cannot notify systemd: {e}");
        }
    }

    /// The daemon serves D-Bus and has a config: callers can talk to it.
    pub fn ready(&self, status: &str) {
        self.send(&format!("READY=1\nSTATUS={status}"));
    }

    /// One line for `systemctl status`.
    pub fn status(&self, status: &str) {
        self.send(&format!("STATUS={status}"));
    }
}
