// SPDX-License-Identifier: GPL-3.0-or-later

//! `junc doctor` (F19): is everything JUNC depends on installed and
//! running, and how is the audio clock set. Each check says what it found
//! and, when something is wrong, the command that fixes it.

use crate::client::Client;
use junc_l10n::{tr, trf};
use serde::Serialize;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Result of one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Missing,
    Down,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub status: Status,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

fn check(id: &'static str, status: Status, detail: String, fix: Option<&str>) -> Check {
    Check {
        id,
        status,
        detail,
        fix: fix.map(str::to_owned),
    }
}

/// Output of a command, if it ran and succeeded.
fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn installed(binary: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(binary).is_file()))
        .unwrap_or(false)
}

/// Whether a process of this user with this name runs.
fn running(comm: &str) -> bool {
    let uid = |p: &Path| std::fs::metadata(p).map(|m| m.uid()).ok();
    let me = uid(Path::new("/proc/self"));
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    entries.flatten().any(|e| {
        let p = e.path();
        std::fs::read_to_string(p.join("comm")).is_ok_and(|c| c.trim() == comm) && uid(&p) == me
    })
}

fn pipewire_socket() -> Option<PathBuf> {
    let dir =
        std::env::var_os("PIPEWIRE_RUNTIME_DIR").or_else(|| std::env::var_os("XDG_RUNTIME_DIR"))?;
    let name = std::env::var("PIPEWIRE_REMOTE").unwrap_or_else(|_| "pipewire-0".into());
    Some(Path::new(&dir).join(name))
}

/// `1.6.9` from `pipewire --version` output.
pub fn pipewire_version(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.split("libpipewire ").nth(1))
        .map(|v| v.trim().to_owned())
}

fn at_least_1_0(version: &str) -> bool {
    version
        .split('.')
        .next()
        .and_then(|m| m.parse::<u32>().ok())
        .is_some_and(|m| m >= 1)
}

/// `key → value` pairs from `pw-metadata` output lines like
/// `update: id:0 key:'clock.rate' value:'48000' type:''`.
pub fn metadata(text: &str) -> Vec<(String, String)> {
    let field = |l: &str, name: &str| {
        let start = l.find(&format!("{name}:'"))? + name.len() + 2;
        let len = l[start..].find('\'')?;
        Some(l[start..start + len].to_owned())
    };
    text.lines()
        .filter_map(|l| Some((field(l, "key")?, field(l, "value")?)))
        .collect()
}

fn juncd(client: Option<&Client>) -> Check {
    let fix = "systemctl --user start juncd";
    let Some(d) = client.and_then(|c| c.describe().ok()) else {
        return check(
            "juncd",
            Status::Down,
            tr("not running").into_owned(),
            Some(fix),
        );
    };
    if let Some(e) = d.config_error.first() {
        let at = e
            .location
            .as_ref()
            .map(|l| format!("{}:{}:{}: ", l.file, l.line, l.column))
            .unwrap_or_default();
        return check(
            "juncd",
            Status::Warn,
            trf(
                "the config file has an error ({at}{msg}); the last good config is in use",
                &[("at", &at), ("msg", &e.message)],
            ),
            Some("junc config validate"),
        );
    }
    match d.state.as_str() {
        "connected" => check(
            "juncd",
            Status::Ok,
            trf(
                "running {v}, connected to PipeWire, config revision {rev}",
                &[("v", &d.daemon_version), ("rev", &d.revision)],
            ),
            None,
        ),
        other => check(
            "juncd",
            Status::Warn,
            trf("running, but {state}", &[("state", &other)]),
            None,
        ),
    }
}

fn pipewire() -> Check {
    let version = run("pipewire", &["--version"]).and_then(|t| pipewire_version(&t));
    let socket = pipewire_socket().is_some_and(|s| s.exists());
    match (socket, version) {
        (true, Some(v)) if !at_least_1_0(&v) => check(
            "pipewire",
            Status::Warn,
            trf("running {v}; JUNC needs 1.0 or newer", &[("v", &v)]),
            Some("sudo pacman -Syu pipewire"),
        ),
        (true, v) => check(
            "pipewire",
            Status::Ok,
            trf("running {v}", &[("v", &v.unwrap_or_default())])
                .trim()
                .to_owned(),
            None,
        ),
        (false, Some(_)) => check(
            "pipewire",
            Status::Down,
            tr("installed but not running").into_owned(),
            Some("systemctl --user start pipewire"),
        ),
        (false, None) => check(
            "pipewire",
            Status::Missing,
            tr("not installed").into_owned(),
            Some("sudo pacman -S pipewire"),
        ),
    }
}

fn service(id: &'static str, binary: &str, why: &str) -> Check {
    if running(binary) {
        check(id, Status::Ok, tr("running").into_owned(), None)
    } else if installed(binary) {
        check(
            id,
            Status::Down,
            trf("installed but not running; {why}", &[("why", &why)]),
            Some(&format!("systemctl --user start {binary}")),
        )
    } else {
        check(
            id,
            Status::Missing,
            trf("not installed; {why}", &[("why", &why)]),
            Some(&format!("sudo pacman -S {binary}")),
        )
    }
}

fn package(name: &str) -> Option<bool> {
    let out = Command::new("pacman").args(["-Q", name]).output().ok()?;
    Some(out.status.success())
}

fn jack() -> Check {
    match (package("pipewire-jack"), package("jack2")) {
        (Some(true), _) => check(
            "jack",
            Status::Ok,
            tr("pipewire-jack: JACK apps go through PipeWire").into_owned(),
            None,
        ),
        (_, Some(true)) => check(
            "jack",
            Status::Warn,
            tr("jack2 is installed: JACK apps bypass PipeWire and cannot be routed").into_owned(),
            Some("sudo pacman -S pipewire-jack"),
        ),
        (Some(false), Some(false)) => check(
            "jack",
            Status::Ok,
            tr("no JACK library; only JACK apps need one").into_owned(),
            Some("sudo pacman -S pipewire-jack"),
        ),
        _ => check(
            "jack",
            Status::Warn,
            tr("cannot tell which JACK library is installed (no pacman)").into_owned(),
            None,
        ),
    }
}

fn clock() -> Check {
    let Some(text) = run("pw-metadata", &["-n", "settings", "0"]) else {
        return check(
            "clock",
            Status::Warn,
            tr("cannot read the PipeWire settings").into_owned(),
            None,
        );
    };
    let m = metadata(&text);
    let get = |k: &str| {
        m.iter()
            .find(|(key, _)| key == k)
            .and_then(|(_, v)| v.parse::<u32>().ok())
            .filter(|v| *v > 0)
    };
    let (Some(rate), Some(quantum)) = (
        get("clock.force-rate").or(get("clock.rate")),
        get("clock.force-quantum").or(get("clock.quantum")),
    ) else {
        return check(
            "clock",
            Status::Warn,
            tr("no clock settings found").into_owned(),
            None,
        );
    };
    let ms = f64::from(quantum) * 1000.0 / f64::from(rate);
    let mut detail = trf(
        "{rate} Hz, quantum {q} ({ms} ms)",
        &[
            ("rate", &rate),
            ("q", &quantum),
            ("ms", &format!("{ms:.1}")),
        ],
    );
    if get("clock.force-rate").is_some() || get("clock.force-quantum").is_some() {
        detail.push_str(&trf("; forced with pw-metadata", &[]));
    }
    check("clock", Status::Ok, detail, None)
}

/// All checks, in the order a person would fix them.
pub fn checks(client: Option<&Client>) -> Vec<Check> {
    vec![
        pipewire(),
        service(
            "wireplumber",
            "wireplumber",
            &tr("nothing gets connected without it"),
        ),
        service(
            "pipewire-pulse",
            "pipewire-pulse",
            &tr("apps that use PulseAudio cannot be routed"),
        ),
        jack(),
        clock(),
        juncd(client),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_metadata_parse() {
        let v = "pipewire\nCompiled with libpipewire 1.6.9\nLinked with libpipewire 1.6.9\n";
        assert_eq!(pipewire_version(v).as_deref(), Some("1.6.9"));
        assert!(at_least_1_0("1.6.9") && !at_least_1_0("0.3.80"));
        let m = metadata(
            "Found \"settings\" metadata 31\n\
             update: id:0 key:'log.level' value:'2' type:''\n\
             update: id:0 key:'clock.rate' value:'48000' type:''\n\
             update: id:0 key:'clock.quantum' value:'1024' type:''\n",
        );
        assert_eq!(m.len(), 3);
        assert_eq!(m[1], ("clock.rate".into(), "48000".into()));
    }
}
