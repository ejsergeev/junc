// SPDX-License-Identifier: GPL-3.0-or-later

//! What the window asks juncd to do (architecture §7.1: every action is a
//! D-Bus call). Calls run on their own thread so the window never waits
//! for juncd; their effect comes back as the next `Describe`.

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
    /// A config change to one Junction (`GetConfig` → `Apply`).
    Edit { junction: String, edit: Edit },
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
