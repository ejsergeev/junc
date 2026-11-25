// SPDX-License-Identifier: GPL-3.0-or-later

//! `junc doctor --bug-report` (F19): one text to paste into an issue, with
//! versions, the Junction graph, doctor's checks, the config and juncd's
//! recent log. Never audio; personal data (user and host name, home
//! paths, hardware addresses) is replaced before anything is shown. JUNC
//! sends nothing by itself.

use crate::doctor::{Check, Facts, metadata};
use crate::human;
use junc_core::graph::Graph;
use std::fmt::Write;

/// How many log lines the report carries.
pub const LOG_LINES: usize = 200;

/// What the scrubber replaces.
#[derive(Debug, Clone)]
pub struct Identity {
    pub home: String,
    pub user: String,
    pub host: String,
}

impl Identity {
    /// This user on this machine.
    pub fn current() -> Self {
        let env = |k: &str| std::env::var(k).unwrap_or_default();
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_default();
        let host = std::fs::read_to_string("/proc/sys/kernel/hostname")
            .map(|h| h.trim().to_owned())
            .unwrap_or_default();
        Self {
            home: env("HOME"),
            user,
            host,
        }
    }
}

fn boundary(c: Option<char>) -> bool {
    c.is_none_or(|c| !c.is_ascii_alphanumeric())
}

/// Replaces `word` where it stands alone (not inside a longer name).
fn replace_word(text: &str, word: &str, with: &str) -> String {
    if word.len() < 2 {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(word) {
        let before = if i > 0 {
            rest[..i].chars().last()
        } else {
            out.chars().last()
        };
        let after = rest[i + word.len()..].chars().next();
        out.push_str(&rest[..i]);
        if boundary(before) && boundary(after) {
            out.push_str(with);
        } else {
            out.push_str(word);
        }
        rest = &rest[i + word.len()..];
    }
    out.push_str(rest);
    out
}

/// `AA:BB:CC:DD:EE:FF` and `AA_BB_CC_DD_EE_FF` (Bluetooth node names)
/// become `XX:XX:XX:XX:XX:XX` with the same separator.
fn hide_addresses(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        let at = |k: usize| b.get(i + k).copied();
        let sep = at(2);
        let is_mac = matches!(sep, Some(b':' | b'_'))
            && (0..6).all(|n| {
                at(3 * n).is_some_and(|c| c.is_ascii_hexdigit())
                    && at(3 * n + 1).is_some_and(|c| c.is_ascii_hexdigit())
                    && (n == 5 || at(3 * n + 2) == sep)
            })
            && !at(17).is_some_and(|c| c.is_ascii_hexdigit())
            && !(i > 0 && b[i - 1].is_ascii_hexdigit());
        if is_mac {
            let s = char::from(sep.unwrap_or(b':')).to_string();
            out.push_str(&["XX"; 6].join(&s));
            i += 17;
        } else {
            let c = text[i..].chars().next().unwrap_or_default();
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// Removes personal data: the home path becomes `~`, other homes
/// `/home/<user>`, the user and host names `<user>` and `<host>`, and
/// hardware addresses `XX:…`.
pub fn scrub(text: &str, id: &Identity) -> String {
    let mut t = text.to_owned();
    let home = id.home.trim_end_matches('/');
    if home.len() > 1 {
        t = t.replace(home, "~");
    }
    // Any other home directory.
    let mut out = String::with_capacity(t.len());
    let mut rest = t.as_str();
    while let Some(i) = rest.find("/home/") {
        out.push_str(&rest[..i]);
        out.push_str("/home/<user>");
        let after = &rest[i + 6..];
        let end = after
            .find(|c: char| c == '/' || c.is_whitespace() || c == '"' || c == '\'')
            .unwrap_or(after.len());
        rest = &after[end..];
    }
    out.push_str(rest);
    let mut t = replace_word(&out, &id.user, "<user>");
    t = replace_word(&t, &id.host, "<host>");
    hide_addresses(&t)
}

/// What the report is made of (gathered by [`gather`]).
#[derive(Debug, Clone)]
pub struct Inputs {
    pub junc_version: String,
    pub wireplumber_version: Option<String>,
    pub kernel: String,
    pub arch: String,
    pub facts: Facts,
    pub checks: Vec<Check>,
    /// The config file text.
    pub config: Option<String>,
    /// Where it is.
    pub config_path: String,
    /// juncd's recent log.
    pub log: Option<String>,
}

/// `0.5.18` from `wireplumber --version`.
pub fn wireplumber_version(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.split("libwireplumber ").nth(1))
        .map(|v| v.trim().to_owned())
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The one-line graph summary: Junctions, Cables, JUNC links, rate,
/// quantum.
fn graph_line(i: &Inputs) -> String {
    let f = &i.facts;
    let mut parts = Vec::new();
    if let Some(d) = &f.daemon {
        let cables = d
            .junctions
            .iter()
            .filter(|j| j.kind == junc_core::model::Kind::Cable)
            .count();
        parts.push(format!(
            "{}, {}",
            plural(d.junctions.len() - cables, "Junction", "Junctions"),
            plural(cables, "Cable", "Cables")
        ));
    } else {
        parts.push("juncd not running".to_owned());
    }
    if let Some(g) = f.dump.as_deref().and_then(|d| Graph::from_pw_dump(d).ok()) {
        let links = g.links.values().filter(|l| l.is_junc()).count();
        parts.push(plural(links, "link", "links"));
    }
    let m = f.settings.as_deref().map(metadata).unwrap_or_default();
    let get = |k: &str| {
        m.iter()
            .find(|(key, v)| key == k && v != "0")
            .map(|(_, v)| v.clone())
    };
    if let Some(r) = get("clock.force-rate").or_else(|| get("clock.rate")) {
        parts.push(format!("{r} Hz"));
    }
    if let Some(q) = get("clock.force-quantum").or_else(|| get("clock.quantum")) {
        parts.push(format!("quantum {q}"));
    }
    format!("graph: {}", parts.join(" · "))
}

/// The report, before scrubbing.
pub fn render(i: &Inputs) -> String {
    let f = &i.facts;
    let mut out = String::from("## JUNC bug report\n\n");
    let juncd = f
        .daemon
        .as_ref()
        .map_or("not running".to_owned(), |d| d.daemon_version.clone());
    let _ = writeln!(
        out,
        "junc {} · juncd {juncd} · PipeWire {} · WirePlumber {} · kernel {} ({})",
        i.junc_version,
        f.pipewire_version.as_deref().unwrap_or("?"),
        i.wireplumber_version.as_deref().unwrap_or("?"),
        i.kernel,
        i.arch
    );
    let _ = writeln!(out, "{}", graph_line(i));
    let state = match &f.daemon {
        Some(d) if !d.config_error.is_empty() => "has errors; the last good config is in use",
        Some(_) => "valid",
        None if i.config.is_some() => "not checked: juncd not running",
        None => "none",
    };
    let _ = writeln!(out, "config: {} ({state})", i.config_path);

    let _ = write!(
        out,
        "\n### Checks\n\n```\n{}```\n",
        human::doctor(&i.checks)
    );

    if let Some(d) = &f.daemon {
        out.push_str("\n### Junctions\n\n```\n");
        if d.junctions.is_empty() {
            out.push_str("none\n");
        }
        for j in &d.junctions {
            out.push_str(&human::junction(j));
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
        out.push_str("```\n");
        if !d.devices.is_empty() {
            out.push_str("\n### Sound cards\n\n");
            for dev in &d.devices {
                let _ = writeln!(
                    out,
                    "- {} ({}, {}): profile {}, {} input channels",
                    dev.description,
                    dev.name,
                    dev.api,
                    dev.profile.as_deref().unwrap_or("none"),
                    dev.input_channels
                );
            }
        }
    }

    if let Some(c) = &i.config {
        let _ = write!(out, "\n### Config\n\n```toml\n{}\n```\n", c.trim_end());
    }

    let _ = write!(
        out,
        "\n### Log (last {LOG_LINES} lines of `journalctl --user -u juncd`)\n\n```\n{}\n```\n",
        i.log
            .as_deref()
            .map(str::trim_end)
            .filter(|l| !l.is_empty())
            .unwrap_or("no journal entries for juncd.service")
    );
    out.push_str(
        "\nNo audio. User and host names, home paths and hardware addresses are replaced.\n",
    );
    out
}

/// Gathers everything and returns the scrubbed report.
pub fn gather(client: Option<&crate::client::Client>, facts: Facts, checks: Vec<Check>) -> String {
    let run = |cmd: &str, args: &[&str]| {
        std::process::Command::new(cmd)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    // Ask juncd only if it runs: a report must not start it.
    let running = client.filter(|_| facts.daemon.is_some());
    let config_path = crate::config_path(running);
    let config = running
        .and_then(|c| c.get_config().ok())
        .map(|(t, _)| t)
        .or_else(|| std::fs::read_to_string(&config_path).ok());
    let lines = LOG_LINES.to_string();
    let inputs = Inputs {
        junc_version: env!("CARGO_PKG_VERSION").to_owned(),
        wireplumber_version: run("wireplumber", &["--version"])
            .and_then(|t| wireplumber_version(&t)),
        kernel: std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|k| k.trim().to_owned())
            .unwrap_or_default(),
        arch: std::env::consts::ARCH.to_owned(),
        facts,
        checks,
        config,
        config_path: config_path.display().to_string(),
        log: run(
            "journalctl",
            &[
                "--user",
                "-u",
                "juncd",
                "-n",
                &lines,
                "--no-pager",
                "-o",
                "short-iso",
            ],
        ),
    };
    scrub(&render(&inputs), &Identity::current())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::{Proc, checks};
    use std::path::Path;

    fn alice() -> Identity {
        Identity {
            home: "/home/alice".into(),
            user: "alice".into(),
            host: "macbook".into(),
        }
    }

    #[test]
    fn scrub_removes_personal_data() {
        let text = "\
config: /home/alice/.config/junc/junctions.toml
2026-11-25T10:00:00+0100 macbook juncd[812]: alice's file /home/bob/x and /home/alice
bluez_output.00_1A_7D_DA_71_13.1 and bluez_input.00:1A:7D:DA:71:13 at 12:34:56
user alicex, malice, macbookpro stay; alice@macbook goes";
        let got = scrub(text, &alice());
        assert_eq!(
            got,
            "\
config: ~/.config/junc/junctions.toml
2026-11-25T10:00:00+0100 <host> juncd[812]: <user>'s file /home/<user>/x and ~
bluez_output.XX_XX_XX_XX_XX_XX.1 and bluez_input.XX:XX:XX:XX:XX:XX at 12:34:56
user alicex, malice, macbookpro stay; <user>@<host> goes"
        );
        for secret in ["alice/", "bob", "macbook ", "1A", "7D"] {
            assert!(!got.contains(secret), "{secret} left in:\n{got}");
        }
    }

    #[test]
    fn the_report_matches_the_golden_file() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let read = |p: &str| std::fs::read_to_string(root.join(p)).expect(p);
        let describe: serde_json::Value =
            serde_json::from_str(&read("tests/golden/describe/full.json")).expect("json");
        let daemon = serde_json::from_value(describe["data"].clone()).expect("describe");
        let facts = Facts {
            pipewire_version: Some("1.6.9".into()),
            pipewire_socket: true,
            wireplumber: Proc::Running,
            pipewire_pulse: Proc::Running,
            pipewire_jack: Some(true),
            jack2: Some(false),
            settings: Some(
                "update: id:0 key:'clock.rate' value:'48000' type:''\n\
                 update: id:0 key:'clock.quantum' value:'1024' type:''\n"
                    .into(),
            ),
            dump: Some(read("tests/fixtures/graphs/asahi-m1-j293.json")),
            daemon: Some(daemon),
            apple: true,
            speakersafetyd: Proc::Running,
        };
        let inputs = Inputs {
            junc_version: "0.1.0".into(),
            wireplumber_version: Some("0.5.18".into()),
            kernel: "7.2.11-14-1-ARCH".into(),
            arch: "aarch64".into(),
            checks: checks(&facts),
            facts,
            config: Some(read("tests/fixtures/configs/valid/full.toml")),
            config_path: "/home/alice/.config/junc/junctions.toml".into(),
            log: Some(
                "2026-11-25T10:00:00+0100 macbook juncd[812]: juncd: connected to PipeWire\n\
                 2026-11-25T10:00:00+0100 macbook juncd[812]: juncd: config /home/alice/.config/junc/junctions.toml\n\
                 2026-11-25T10:00:01+0100 macbook juncd[812]: juncd: notice link_overridden junction=zoom ref=in/bluez_input.00:1A:7D:DA:71:13/1->1\n"
                    .into(),
            ),
        };
        let text = scrub(&render(&inputs), &alice());
        let file = root.join("tests/golden/doctor/bug-report.md");
        if std::env::var_os("JUNC_BLESS").is_some() {
            std::fs::write(&file, &text).expect("write golden");
        }
        let want = std::fs::read_to_string(&file).unwrap_or_default();
        assert!(
            want == text,
            "{} differs (JUNC_BLESS=1 cargo test -p junc-cli --bin junc report rewrites it):\n{text}",
            file.display()
        );
        assert!(
            wireplumber_version("Linked with libwireplumber 0.5.18\n").as_deref() == Some("0.5.18")
        );
    }
}
