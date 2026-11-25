// SPDX-License-Identifier: GPL-3.0-or-later

//! The D-Bus contract between juncd, the CLI and the GUI
//! (docs/architecture.md §4, docs/dbus.md).
//!
//! Session bus only. Simple values are D-Bus types; structured values
//! (Junction contents, descriptions, diagnostics, plans) are JSON strings with
//! the same shapes `junc … --json` prints, so agents, the CLI and the GUI share
//! one schema.

#![forbid(unsafe_code)]

pub mod plan;
pub mod view;

use junc_core::diag::Diagnostic;
use serde::{Deserialize, Serialize};
use view::Diag;

/// Well-known bus name. The trailing digit is the major API version.
pub const BUS_NAME: &str = "io.github.ejsergeev.Junc1";
/// Path of the daemon object.
pub const ROOT_PATH: &str = "/io/github/ejsergeev/Junc1";
/// Daemon interface.
pub const DAEMON_IFACE: &str = "io.github.ejsergeev.Junc1.Daemon";
/// Junction interface.
pub const JUNCTION_IFACE: &str = "io.github.ejsergeev.Junc1.Junction";
/// API version (major, minor). Minor grows with additive changes.
pub const API_VERSION: (u32, u32) = (1, 1);

/// Object path of a Junction: ids are `[a-z0-9-]`, and `-` is not allowed in
/// object paths, so it becomes `_2d` (its hex code, D-Bus style).
pub fn junction_path(id: &str) -> String {
    let mut p = format!("{ROOT_PATH}/junction/");
    for c in id.chars() {
        if c.is_ascii_alphanumeric() {
            p.push(c);
        } else {
            p.push_str(&format!("_{:02x}", c as u32));
        }
    }
    p
}

/// The Junction id of an object path from [`junction_path`], if it is one.
pub fn junction_id(path: &str) -> Option<String> {
    let rest = path.strip_prefix(ROOT_PATH)?.strip_prefix("/junction/")?;
    let mut id = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        if c == '_' {
            let hex: String = chars.by_ref().take(2).collect();
            id.push(char::from(u8::from_str_radix(&hex, 16).ok()?));
        } else if c.is_ascii_alphanumeric() {
            id.push(c);
        } else {
            return None;
        }
    }
    (!id.is_empty()).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junction_paths_round_trip() {
        for id in ["zoom", "podcast-mix", "a-b-c-1"] {
            let path = junction_path(id);
            assert!(zbus::ObjectPath::try_from(path.as_str()).is_ok(), "{path}");
            assert_eq!(junction_id(&path).as_deref(), Some(id));
        }
        assert_eq!(
            junction_path("podcast-mix"),
            format!("{ROOT_PATH}/junction/podcast_2dmix")
        );
        assert_eq!(junction_id(ROOT_PATH), None);
        assert_eq!(junction_id(&format!("{ROOT_PATH}/junction/bad_zz")), None);
    }
}

/// The error body sent with every D-Bus error: what is wrong, where, and how
/// to fix it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorBody {
    /// Stable code, e.g. `E_CONFIG_UNKNOWN_KEY`.
    pub code: String,
    /// What is wrong.
    pub message: String,
    /// How to fix it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// All diagnostics when there are several (config validation).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diag>,
}

impl ErrorBody {
    /// A body from a code, message and hint.
    pub fn new(code: &str, message: impl Into<String>, hint: Option<&str>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            hint: hint.map(str::to_owned),
            diagnostics: Vec::new(),
        }
    }

    /// A body from config diagnostics (the first one names the error).
    pub fn from_diagnostics(d: &[Diagnostic]) -> Self {
        let first = d.first();
        Self {
            code: first
                .map_or("E_CONFIG_INVALID_VALUE", |x| x.code)
                .to_owned(),
            message: first.map_or_else(String::new, |x| x.message.clone()),
            hint: first.and_then(|x| x.hint.clone()),
            diagnostics: d.iter().map(Diag::from).collect(),
        }
    }

    fn json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| self.message.clone())
    }
}

/// D-Bus errors of the JUNC API; each carries an [`ErrorBody`] as JSON.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "io.github.ejsergeev.Junc1.Error")]
pub enum Error {
    /// zbus-level failure.
    #[zbus(error)]
    ZBus(zbus::Error),
    /// Unknown Junction, input, link or device.
    NotFound(String),
    /// The config does not validate.
    InvalidConfig(String),
    /// The change was based on an older config revision.
    Conflict(String),
    /// The daemon is waiting for PipeWire.
    PipeWireUnavailable(String),
    /// A destructive call without confirmation.
    NeedsConfirmation(String),
    /// Anything else.
    Failed(String),
}

impl Error {
    /// The matching D-Bus error for a body, by its code.
    pub fn from_body(b: &ErrorBody) -> Self {
        let json = b.json();
        match b.code.as_str() {
            "E_NOT_FOUND" => Self::NotFound(json),
            "E_CONFIG_CONFLICT" => Self::Conflict(json),
            "E_PIPEWIRE_UNAVAILABLE" => Self::PipeWireUnavailable(json),
            "E_NEEDS_CONFIRMATION" => Self::NeedsConfirmation(json),
            c if c.starts_with("E_CONFIG_") => Self::InvalidConfig(json),
            _ => Self::Failed(json),
        }
    }

    /// The structured body, if the error carries one.
    pub fn body(&self) -> Option<ErrorBody> {
        let text = match self {
            Self::NotFound(t)
            | Self::InvalidConfig(t)
            | Self::Conflict(t)
            | Self::PipeWireUnavailable(t)
            | Self::NeedsConfirmation(t)
            | Self::Failed(t) => t,
            Self::ZBus(_) => return None,
        };
        serde_json::from_str(text).ok()
    }
}

/// The D-Bus error name and body of a failed call, for clients. Errors that
/// do not come from JUNC (no reply, access denied) get a body with the D-Bus
/// error name as code.
pub fn error_body(e: &zbus::Error) -> ErrorBody {
    match e {
        zbus::Error::MethodError(name, msg, _) => msg
            .as_deref()
            .and_then(|m| serde_json::from_str(m).ok())
            .unwrap_or_else(|| {
                ErrorBody::new(name.as_str(), msg.clone().unwrap_or_default(), None)
            }),
        other => ErrorBody::new(
            "E_DBUS",
            other.to_string(),
            Some("Is juncd running? Try `systemctl --user status juncd`."),
        ),
    }
}

impl From<ErrorBody> for Error {
    fn from(b: ErrorBody) -> Self {
        Self::from_body(&b)
    }
}

/// Client proxy for the daemon object.
#[zbus::proxy(
    interface = "io.github.ejsergeev.Junc1.Daemon",
    default_service = "io.github.ejsergeev.Junc1",
    default_path = "/io/github/ejsergeev/Junc1"
)]
pub trait Daemon {
    /// JSON snapshot for agents (`junc describe --json`).
    fn describe(&self) -> zbus::Result<String>;
    /// The config file text and its revision.
    fn get_config(&self) -> zbus::Result<(String, u64)>;
    /// Validates config text; returns diagnostics as JSON (empty list = valid).
    fn validate(&self, toml: &str) -> zbus::Result<String>;
    /// Dry run: the graph changes applying this config would make, as JSON.
    fn plan(&self, toml: &str) -> zbus::Result<String>;
    /// Validates, writes and reconciles; returns the new revision and the
    /// plan. `expected_revision` 0 skips the conflict check.
    fn apply(&self, toml: &str, expected_revision: u64) -> zbus::Result<(u64, String)>;
    /// Creates an empty Junction named `name`; returns its object path and
    /// the revision. Cables need their apps up front and go through `Apply`.
    fn create_junction(&self, name: &str) -> zbus::Result<(zbus::wire::OwnedObjectPath, u64)>;
    /// Answers a notice: `restore` or `leave` for an overridden link.
    fn resolve_link(&self, junction: &str, reference: &str, action: &str) -> zbus::Result<u64>;
    /// Graph status changed.
    #[zbus(signal)]
    fn graph_changed(&self, generation: u64) -> zbus::Result<()>;
    /// A notice for the user: kind and a JSON body.
    #[zbus(signal)]
    fn notice(&self, kind: &str, body: &str) -> zbus::Result<()>;
    /// (major, minor).
    #[zbus(property)]
    fn api_version(&self) -> zbus::Result<(u32, u32)>;
    /// Package version.
    #[zbus(property)]
    fn daemon_version(&self) -> zbus::Result<String>;
    /// `starting`, `connected` or `waiting_for_pipewire`.
    #[zbus(property)]
    fn state(&self) -> zbus::Result<String>;
    /// Current config revision.
    #[zbus(property)]
    fn revision(&self) -> zbus::Result<u64>;
    /// Diagnostics of an invalid config file as JSON, `""` when valid.
    #[zbus(property)]
    fn config_error(&self) -> zbus::Result<String>;
    /// Junction object paths in config order.
    #[zbus(property)]
    fn junctions(&self) -> zbus::Result<Vec<zbus::wire::OwnedObjectPath>>;
    /// Apps whose audio is being captured into an enabled Junction (F23).
    #[zbus(property)]
    fn capture_apps(&self) -> zbus::Result<Vec<String>>;
}

/// Client proxy for one Junction object.
#[zbus::proxy(
    interface = "io.github.ejsergeev.Junc1.Junction",
    default_service = "io.github.ejsergeev.Junc1"
)]
pub trait Junction {
    /// Renames it; returns the new revision.
    fn rename(&self, name: &str) -> zbus::Result<u64>;
    /// Turns it on or off.
    fn set_enabled(&self, enabled: bool) -> zbus::Result<u64>;
    /// Master volume in percent.
    fn set_volume(&self, percent: u32) -> zbus::Result<u64>;
    /// `none`, `input` or `output`.
    fn set_default(&self, role: &str) -> zbus::Result<u64>;
    /// Deletes it; `confirm` must be true.
    fn delete(&self, confirm: bool) -> zbus::Result<u64>;
    /// The Junction as JSON (the same shape as one entry of `describe`).
    #[zbus(property)]
    fn view(&self) -> zbus::Result<String>;
    /// Id.
    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;
    /// Name.
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;
    /// On or off.
    #[zbus(property)]
    fn enabled(&self) -> zbus::Result<bool>;
}
