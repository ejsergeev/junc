// SPDX-License-Identifier: GPL-3.0-or-later

//! The daemon over D-Bus, as blocking calls with [`ErrorBody`] errors.

use crate::edit::Result;
use junc_core::model::Config;
use junc_core::store::{CONFLICT, Store};
use junc_ipc::view::Describe;
use junc_ipc::{DaemonProxy, ErrorBody, JunctionProxy, error_body, junction_path};
use zbus::Connection;
use zbus::proxy::CacheProperties;

/// An error from zbus, with "daemon not running" recognised.
fn failed(e: &zbus::Error) -> ErrorBody {
    let unavailable = match e {
        zbus::Error::MethodError(name, _, _) => matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
                | "org.freedesktop.DBus.Error.NoReply"
                | "org.freedesktop.DBus.Error.Spawn.ServiceNotFound"
        ),
        _ => true,
    };
    if unavailable {
        return ErrorBody::new(
            "E_DAEMON_UNAVAILABLE",
            "the JUNC daemon is not running",
            Some("Start it: systemctl --user start juncd"),
        );
    }
    error_body(e)
}

/// A session bus connection to juncd.
#[derive(Debug)]
pub struct Client {
    conn: Connection,
    daemon: DaemonProxy<'static>,
}

impl Client {
    /// Connects to the session bus. Does not check that juncd runs; the
    /// first call does.
    pub fn connect() -> Result<Self> {
        zbus::block_on(async {
            let conn = Connection::session().await.map_err(|e| failed(&e))?;
            let daemon = DaemonProxy::builder(&conn)
                .cache_properties(CacheProperties::No)
                .build()
                .await
                .map_err(|e| failed(&e))?;
            Ok(Self { conn, daemon })
        })
    }

    /// Whether juncd runs now. Unlike a method call, asking does not
    /// start it through D-Bus activation.
    pub fn running(&self) -> bool {
        zbus::block_on(async {
            let dbus = zbus::fdo::DBusProxy::new(&self.conn).await?;
            let name = zbus::names::BusName::try_from(junc_ipc::BUS_NAME)?;
            dbus.name_has_owner(name).await.map_err(zbus::Error::from)
        })
        .unwrap_or(false)
    }

    /// Describe, but only from a daemon that already runs.
    pub fn describe_if_running(&self) -> Option<Describe> {
        self.running().then(|| self.describe().ok()).flatten()
    }

    pub fn describe(&self) -> Result<Describe> {
        let text = zbus::block_on(self.daemon.describe()).map_err(|e| failed(&e))?;
        serde_json::from_str(&text).map_err(|e| {
            ErrorBody::new(
                "E_INTERNAL",
                format!("unexpected answer from juncd: {e}"),
                Some("Are junc and juncd the same version?"),
            )
        })
    }

    /// Setups that can be imported (F13).
    pub fn scan_importable(&self) -> Result<Vec<junc_core::import::Importable>> {
        let text = zbus::block_on(self.daemon.scan_importable()).map_err(|e| failed(&e))?;
        serde_json::from_str(&text).map_err(|e| {
            ErrorBody::new(
                "E_INTERNAL",
                format!("unexpected answer from juncd: {e}"),
                Some("Are junc and juncd the same version?"),
            )
        })
    }

    pub fn get_config(&self) -> Result<(String, u64)> {
        zbus::block_on(self.daemon.get_config()).map_err(|e| failed(&e))
    }

    /// The config in use, parsed, with its revision.
    pub fn config(&self) -> Result<(Config, u64)> {
        let (text, rev) = self.get_config()?;
        let store =
            Store::open(&text, "junctions.toml").map_err(|d| ErrorBody::from_diagnostics(&d))?;
        Ok((store.config().clone(), rev))
    }

    pub fn apply(&self, text: &str, expected: u64) -> Result<(u64, String)> {
        zbus::block_on(self.daemon.apply(text, expected)).map_err(|e| failed(&e))
    }

    /// Dry run of a config text against the live graph.
    pub fn plan(&self, text: &str) -> Result<junc_ipc::plan::PlanView> {
        let json = zbus::block_on(self.daemon.plan(text)).map_err(|e| failed(&e))?;
        serde_json::from_str(&json).map_err(|e| {
            ErrorBody::new(
                "E_INTERNAL",
                format!("unexpected answer from juncd: {e}"),
                Some("Are junc and juncd the same version?"),
            )
        })
    }

    pub fn create_junction(&self, name: &str) -> Result<(String, u64)> {
        let (path, rev) =
            zbus::block_on(self.daemon.create_junction(name)).map_err(|e| failed(&e))?;
        Ok((path.as_str().to_owned(), rev))
    }

    /// Calls `f` on the Junction object of `id`.
    pub fn junction<T>(
        &self,
        id: &str,
        f: impl AsyncFnOnce(&JunctionProxy<'_>) -> zbus::Result<T>,
    ) -> Result<T> {
        zbus::block_on(async {
            let proxy = JunctionProxy::builder(&self.conn)
                .path(junction_path(id))
                .cache_properties(CacheProperties::No)
                .build()
                .await?;
            f(&proxy).await
        })
        .map_err(|e| failed(&e))
    }

    /// Reads the config, lets `f` change it, and applies the result with
    /// the user's comments kept. A change that raced with another one is
    /// redone on the new config (up to three times), since `f` describes
    /// the edit, not the whole file.
    pub fn edit<T>(&self, mut f: impl FnMut(&mut Config) -> Result<T>) -> Result<(T, u64)> {
        let mut last = None;
        for _ in 0..3 {
            let (text, rev) = self.get_config()?;
            let mut store = Store::open(&text, "junctions.toml")
                .map_err(|d| ErrorBody::from_diagnostics(&d))?;
            let mut config = store.config().clone();
            let out = f(&mut config)?;
            let (new_text, _) = store
                .update(None, config)
                .map_err(|d| ErrorBody::from_diagnostics(&d))?;
            match self.apply(&new_text, rev) {
                Ok((rev, _plan)) => return Ok((out, rev)),
                Err(e) if e.code == CONFLICT => last = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| ErrorBody::new(CONFLICT, "config kept changing", None)))
    }
}
