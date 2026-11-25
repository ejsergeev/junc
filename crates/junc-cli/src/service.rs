// SPDX-License-Identifier: GPL-3.0-or-later

//! `junc service enable|disable|status` (F9, "Start at login"): the
//! `juncd.service` user unit through `systemctl --user`. Without it the
//! daemon still starts on demand through D-Bus activation.

use crate::client::Client;
use crate::edit::Result;
use junc_ipc::ErrorBody;
use junc_l10n::{tr, trf};
use serde::Serialize;
use std::process::Command;

pub const UNIT: &str = "juncd.service";

/// State of the unit and the daemon.
#[derive(Debug, Serialize)]
pub struct Status {
    /// Starts at login.
    pub enabled: bool,
    /// `systemctl is-enabled` (`enabled`, `disabled`, `not-found`, …).
    pub unit_file: String,
    /// `systemctl is-active` (`active`, `inactive`, `failed`, …).
    pub active: String,
    /// The daemon's own state when it answers on D-Bus.
    pub daemon: Option<String>,
}

fn systemctl(args: &[&str]) -> Result<(bool, String)> {
    let out = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .map_err(|e| {
            ErrorBody::new(
                "E_NO_SYSTEMD",
                trf("cannot run systemctl: {e}", &[("e", &e)]),
                Some("Without systemd, start juncd from your session's autostart instead."),
            )
        })?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    Ok((
        out.status.success(),
        if text.is_empty() { err } else { text },
    ))
}

/// Current state.
pub fn status(client: Option<&Client>) -> Result<Status> {
    let (_, unit_file) = systemctl(&["is-enabled", UNIT])?;
    let (_, active) = systemctl(&["is-active", UNIT])?;
    Ok(Status {
        enabled: unit_file == "enabled",
        unit_file,
        active,
        daemon: client
            .and_then(Client::describe_if_running)
            .map(|d| d.state),
    })
}

/// Turns "Start at login" on (and starts the daemon) or off (and stops it;
/// D-Bus activation still starts it when something needs it).
pub fn set(on: bool) -> Result<()> {
    let args = if on {
        ["enable", "--now", UNIT]
    } else {
        ["disable", "--now", UNIT]
    };
    let (ok, message) = systemctl(&args)?;
    if ok {
        return Ok(());
    }
    if message.contains("not found") || message.contains("does not exist") {
        return Err(ErrorBody::new(
            "E_NOT_FOUND",
            trf("{unit} is not installed", &[("unit", &UNIT)]),
            Some("Install the junc package (it ships the user unit)."),
        ));
    }
    Err(ErrorBody::new(
        "E_SERVICE",
        trf("systemctl failed: {m}", &[("m", &message)]),
        Some("See `systemctl --user status juncd`."),
    ))
}

/// One line for people.
pub fn describe(s: &Status) -> String {
    let login = if s.enabled {
        tr("starts at login")
    } else {
        tr("does not start at login (it starts when needed)")
    };
    trf(
        "juncd {login}; service {active}{daemon}.",
        &[
            ("login", &login),
            ("active", &s.active),
            (
                "daemon",
                &s.daemon
                    .as_ref()
                    .map(|d| trf(", daemon {d}", &[("d", d)]))
                    .unwrap_or_default(),
            ),
        ],
    )
}
