// SPDX-License-Identifier: GPL-3.0-or-later

//! What the window asks juncd (and the desktop) to do (architecture §7.1:
//! every config change is a D-Bus call). Calls run on their own thread so
//! the window never waits for juncd; their effect comes back as the next
//! `Describe`, or for the system check through [`crate::app::checks_current`].

use crate::edits::{self, Edit};
use junc_core::store::{CONFLICT, Store};
use junc_ipc::{DaemonProxy, JunctionProxy, error_body};
use std::sync::mpsc;

/// One request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Junction on or off (`Junction.SetEnabled`).
    Enabled { path: String, on: bool },
    /// Master volume (`Junction.SetVolume`).
    Volume { path: String, percent: u16 },
    /// Remove it (`Junction.Delete(true)`, after the window confirmed).
    Delete { path: String },
    /// A new display name (`Junction.Rename`; the id stays).
    Rename { path: String, name: String },
    /// A config change to one Junction (`GetConfig` → `Apply`).
    Edit { junction: String, edit: Edit },
    /// A new empty Junction (`Daemon.CreateJunction`).
    Create { name: String },
    /// Run the onboarding system check (`junc doctor --json`).
    Check,
    /// Put text on the clipboard (`wl-copy`).
    Copy(String),
    /// juncd's journal in a terminal (board 20).
    OpenLogs,
}

/// Where the window sends requests: juncd, or a scenario's mock.
pub trait Actions {
    fn send(&self, command: Command);
}

/// juncd over D-Bus, from a worker thread.
#[derive(Debug)]
pub struct Dbus {
    tx: mpsc::Sender<Command>,
}

impl Dbus {
    /// Starts the worker; `address` is a bus address (tests), `None` the
    /// session bus.
    pub fn spawn(address: Option<String>) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Command>();
        std::thread::Builder::new()
            .name("junc-actions".into())
            .spawn(move || {
                let conn = zbus::block_on(async {
                    match &address {
                        Some(a) => zbus::connection::Builder::address(a.as_str()).build().await,
                        None => zbus::Connection::session().await,
                    }
                });
                let Ok(conn) = conn else { return };
                for command in rx {
                    if local(&command) {
                        continue;
                    }
                    if let Err(e) = zbus::block_on(run(&conn, &command)) {
                        eprintln!("junc-gui: {command:?} failed: {e}");
                    }
                }
            })?;
        Ok(Self { tx })
    }
}

impl Actions for Dbus {
    fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }
}

/// Runs what needs no D-Bus; returns whether it did.
fn local(command: &Command) -> bool {
    match command {
        Command::Check => {
            // Doctor asks pacman and PipeWire: never hold up D-Bus calls.
            let _ = std::thread::Builder::new()
                .name("junc-doctor".into())
                .spawn(|| {
                    let rows = crate::checks::run().unwrap_or_else(|e| {
                        eprintln!("junc-gui: {e}");
                        Vec::new()
                    });
                    let _ = slint::invoke_from_event_loop(move || {
                        crate::app::checks_current(rows);
                    });
                });
            true
        }
        Command::Copy(text) => {
            let copied = std::process::Command::new("wl-copy")
                .arg("--")
                .arg(text)
                .status()
                .is_ok_and(|s| s.success());
            if !copied {
                eprintln!("junc-gui: wl-copy is not available to copy {text:?}");
            }
            true
        }
        Command::OpenLogs => {
            // The desktop's terminal (xdg-terminal-exec, as Omarchy's
            // own menus use); it is not waited for.
            let opened = std::process::Command::new("xdg-terminal-exec")
                .args(["journalctl", "--user", "-u", "juncd", "-e"])
                .spawn()
                .is_ok();
            if !opened {
                eprintln!("junc-gui: xdg-terminal-exec is not available to show juncd's journal");
            }
            true
        }
        _ => false,
    }
}

async fn junction<'a>(
    conn: &'a zbus::Connection,
    path: &'a str,
) -> zbus::Result<JunctionProxy<'a>> {
    JunctionProxy::builder(conn)
        .path(path)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
}

async fn run(conn: &zbus::Connection, command: &Command) -> Result<(), String> {
    let text = |e: zbus::Error| error_body(&e).message;
    match command {
        Command::Enabled { path, on } => {
            junction(conn, path)
                .await
                .map_err(text)?
                .set_enabled(*on)
                .await
                .map_err(text)?;
        }
        Command::Volume { path, percent } => {
            junction(conn, path)
                .await
                .map_err(text)?
                .set_volume(u32::from(*percent))
                .await
                .map_err(text)?;
        }
        Command::Delete { path } => {
            junction(conn, path)
                .await
                .map_err(text)?
                .delete(true)
                .await
                .map_err(text)?;
        }
        Command::Rename { path, name } => {
            junction(conn, path)
                .await
                .map_err(text)?
                .rename(name)
                .await
                .map_err(text)?;
        }
        Command::Create { name } => {
            DaemonProxy::builder(conn)
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()
                .await
                .map_err(text)?
                .create_junction(name)
                .await
                .map_err(text)?;
        }
        Command::Check | Command::Copy(_) | Command::OpenLogs => {}
        Command::Edit { junction: id, edit } => {
            let daemon = DaemonProxy::builder(conn)
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()
                .await
                .map_err(text)?;
            // Someone else may change the config between reading and
            // writing it: read again and retry, as the CLI does.
            for _ in 0..3 {
                let (current, revision) = daemon.get_config().await.map_err(text)?;
                let mut store = Store::open(&current, "junctions.toml")
                    .map_err(|d| format!("{} diagnostics", d.len()))?;
                let mut config = store.config().clone();
                if !edits::apply(&mut config, id, edit) {
                    return Err(format!("{id} changed meanwhile"));
                }
                let (next, _) = store
                    .update(None, config)
                    .map_err(|d| format!("{} diagnostics", d.len()))?;
                match daemon.apply(&next, revision).await {
                    Ok(_) => return Ok(()),
                    Err(e) if error_body(&e).code == CONFLICT => {}
                    Err(e) => return Err(text(e)),
                }
            }
            return Err("the config kept changing".to_owned());
        }
    }
    Ok(())
}
