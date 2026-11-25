// SPDX-License-Identifier: GPL-3.0-or-later

//! `junc doctor` (F19): is everything JUNC depends on installed and
//! running, and how is the audio clock set. Each check says what it found
//! and, when something is wrong, the command that fixes it.
//!
//! [`Facts::gather`] reads the system once (processes, packages,
//! `pw-metadata`, juncd); the checks are pure functions of those facts, so
//! their output is tested against golden files.

use crate::client::Client;
use junc_ipc::view::Describe;
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

/// Whether a process with this name runs, of this user only or of anyone
/// (system services).
fn running_as(comm: &str, mine: bool) -> bool {
    let uid = |p: &Path| std::fs::metadata(p).map(|m| m.uid()).ok();
    let me = uid(Path::new("/proc/self"));
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    entries.flatten().any(|e| {
        let p = e.path();
        std::fs::read_to_string(p.join("comm")).is_ok_and(|c| c.trim() == comm)
            && (!mine || uid(&p) == me)
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

/// Whether a program is running, installed, or neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proc {
    Running,
    Installed,
    Missing,
}

impl Proc {
    fn of(binary: &str) -> Self {
        Self::of_as(binary, true)
    }

    fn of_as(binary: &str, mine: bool) -> Self {
        if running_as(binary, mine) {
            Self::Running
        } else if installed(binary) {
            Self::Installed
        } else {
            Self::Missing
        }
    }
}

/// Everything the checks look at.
#[derive(Debug, Clone)]
pub struct Facts {
    /// `pipewire --version`, parsed.
    pub pipewire_version: Option<String>,
    /// The PipeWire socket exists.
    pub pipewire_socket: bool,
    pub wireplumber: Proc,
    pub pipewire_pulse: Proc,
    /// Installed packages (`None` without pacman).
    pub pipewire_jack: Option<bool>,
    pub jack2: Option<bool>,
    /// `pw-metadata -n settings 0` output.
    pub settings: Option<String>,
    /// `pw-dump` output.
    pub dump: Option<String>,
    /// juncd's snapshot, if it runs.
    pub daemon: Option<Describe>,
    /// An Apple Silicon Mac (Asahi Linux).
    pub apple: bool,
    /// The speaker protection daemon of Asahi (a system service).
    pub speakersafetyd: Proc,
}

impl Facts {
    /// Reads the system.
    pub fn gather(client: Option<&Client>) -> Self {
        Self {
            pipewire_version: run("pipewire", &["--version"]).and_then(|t| pipewire_version(&t)),
            pipewire_socket: pipewire_socket().is_some_and(|s| s.exists()),
            wireplumber: Proc::of("wireplumber"),
            pipewire_pulse: Proc::of("pipewire-pulse"),
            pipewire_jack: package("pipewire-jack"),
            jack2: package("jack2"),
            settings: run("pw-metadata", &["-n", "settings", "0"]),
            dump: run("pw-dump", &[]),
            daemon: client.and_then(Client::describe_if_running),
            apple: std::fs::read("/proc/device-tree/compatible")
                .is_ok_and(|c| c.windows(6).any(|w| w == b"apple,")),
            speakersafetyd: Proc::of_as("speakersafetyd", false),
        }
    }
}

fn juncd(daemon: Option<&Describe>) -> Check {
    let fix = "systemctl --user start juncd";
    let Some(d) = daemon else {
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

fn pipewire(f: &Facts) -> Check {
    match (f.pipewire_socket, f.pipewire_version.clone()) {
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

fn service(id: &'static str, binary: &str, state: Proc, why: &str) -> Check {
    match state {
        Proc::Running => check(id, Status::Ok, tr("running").into_owned(), None),
        Proc::Installed => check(
            id,
            Status::Down,
            trf("installed but not running; {why}", &[("why", &why)]),
            Some(&format!("systemctl --user start {binary}")),
        ),
        Proc::Missing => check(
            id,
            Status::Missing,
            trf("not installed; {why}", &[("why", &why)]),
            Some(&format!("sudo pacman -S {binary}")),
        ),
    }
}

fn package(name: &str) -> Option<bool> {
    let out = Command::new("pacman").args(["-Q", name]).output().ok()?;
    Some(out.status.success())
}

fn jack(f: &Facts) -> Check {
    match (f.pipewire_jack, f.jack2) {
        (Some(true), _) => check(
            "jack",
            Status::Ok,
            tr("pipewire-jack: JACK apps go through PipeWire").into_owned(),
            None,
        ),
        (_, Some(true)) => check(
            "jack",
            Status::Warn,
            tr("jack2 is installed: JACK apps bypass PipeWire and cannot be routed; replace it with pipewire-jack (pacman offers to remove jack2)").into_owned(),
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

fn clock(f: &Facts) -> Check {
    let Some(text) = &f.settings else {
        return check(
            "clock",
            Status::Warn,
            tr("cannot read the PipeWire settings").into_owned(),
            None,
        );
    };
    let m = metadata(text);
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
            ("ms", &junc_l10n::numbers::format(ms, 1)),
        ],
    );
    if get("clock.force-rate").is_some() || get("clock.force-quantum").is_some() {
        detail.push_str(&trf("; forced with pw-metadata", &[]));
    }
    check("clock", Status::Ok, detail, None)
}

/// `44.1 kHz`, `48 kHz`, in the language's number format.
fn khz(rate: u32) -> String {
    let k = f64::from(rate) / 1000.0;
    let decimals = if rate.is_multiple_of(1000) { 0 } else { 1 };
    format!("{} kHz", junc_l10n::numbers::format(k, decimals))
}

/// The rates a device's `EnumFormat` offers.
#[derive(Debug, Clone, PartialEq)]
enum Rates {
    One(u32),
    Range { default: u32, min: u32, max: u32 },
    List { default: u32, list: Vec<u32> },
}

impl Rates {
    fn parse(v: &serde_json::Value) -> Option<Self> {
        let n = |v: &serde_json::Value| v.as_u64().and_then(|n| u32::try_from(n).ok());
        if let Some(r) = n(v) {
            return Some(Self::One(r));
        }
        let default = n(v.get("default")?)?;
        if let Some(list) = v.get("alternatives").and_then(|a| a.as_array()) {
            return Some(Self::List {
                default,
                list: list.iter().filter_map(n).collect(),
            });
        }
        match (v.get("min").and_then(n), v.get("max").and_then(n)) {
            (Some(min), Some(max)) => Some(Self::Range { default, min, max }),
            _ => Some(Self::One(default)),
        }
    }

    fn supports(&self, rate: u32) -> bool {
        match self {
            Self::One(r) => *r == rate,
            Self::Range { min, max, .. } => (*min..=*max).contains(&rate),
            Self::List { list, .. } => list.contains(&rate),
        }
    }

    /// The rate the device prefers.
    fn own(&self) -> u32 {
        match self {
            Self::One(r) | Self::Range { default: r, .. } | Self::List { default: r, .. } => *r,
        }
    }
}

/// Hardware (ALSA) inputs and outputs with their name and rates.
fn device_rates(dump: &str) -> Vec<(String, Rates)> {
    let Ok(serde_json::Value::Array(objects)) = serde_json::from_str(dump) else {
        return Vec::new();
    };
    objects
        .iter()
        .filter(|o| o["type"] == "PipeWire:Interface:Node")
        .filter_map(|o| {
            let info = &o["info"];
            let props = &info["props"];
            let class = props["media.class"].as_str()?;
            if props["device.api"] != "alsa" || !matches!(class, "Audio/Sink" | "Audio/Source") {
                return None;
            }
            let rates = info["params"]["EnumFormat"]
                .as_array()?
                .iter()
                .find_map(|f| Rates::parse(&f["rate"]))?;
            let name = props["node.description"]
                .as_str()
                .or(props["node.name"].as_str())?;
            Some((name.to_owned(), rates))
        })
        .collect()
}

/// `[ 44100 48000 ]` → the rates.
fn rate_list(text: &str) -> Vec<u32> {
    text.split(|c: char| !c.is_ascii_digit())
        .filter_map(|n| n.parse().ok())
        .collect()
}

/// Devices that cannot run at the graph's rate are resampled. Listing
/// their rate in `clock.allowed-rates` lets PipeWire switch to it.
fn rates(f: &Facts) -> Check {
    let (Some(settings), Some(dump)) = (&f.settings, &f.dump) else {
        return check(
            "rates",
            Status::Warn,
            tr("cannot read the devices' sample rates").into_owned(),
            None,
        );
    };
    let m = metadata(settings);
    let get = |k: &str| m.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    let Some(rate) = get("clock.force-rate")
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|r| *r > 0)
        .or_else(|| get("clock.rate").and_then(|v| v.parse().ok()))
    else {
        return check(
            "rates",
            Status::Warn,
            tr("no clock rate found").into_owned(),
            None,
        );
    };
    let mut allowed = get("clock.allowed-rates")
        .map(rate_list)
        .unwrap_or_default();
    if allowed.is_empty() {
        allowed.push(rate);
    }
    let devices = device_rates(dump);
    let off: Vec<&(String, Rates)> = devices
        .iter()
        .filter(|(_, r)| !r.supports(rate) && !allowed.contains(&r.own()))
        .collect();
    if off.is_empty() {
        let detail = if devices.is_empty() {
            trf("{rate}; no hardware devices", &[("rate", &khz(rate))])
        } else {
            trf(
                "every hardware device can run at {rate}",
                &[("rate", &khz(rate))],
            )
        };
        return check("rates", Status::Ok, detail, None);
    }
    let mut want = allowed.clone();
    want.extend(off.iter().map(|(_, r)| r.own()));
    want.sort_unstable();
    want.dedup();
    let detail = off
        .iter()
        .map(|(name, r)| {
            trf(
                "{device} runs at {own}, the graph at {rate}: audio is resampled",
                &[
                    ("device", name),
                    ("own", &khz(r.own())),
                    ("rate", &khz(rate)),
                ],
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let list = want
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    check(
        "rates",
        Status::Warn,
        detail,
        Some(&format!(
            "pw-metadata -n settings 0 clock.allowed-rates \"[ {list} ]\""
        )),
    )
}

/// The Asahi DSP sink (`audio_effect.*`) in front of the speakers.
fn dsp_sink(dump: &str) -> Option<String> {
    let serde_json::Value::Array(objects) = serde_json::from_str(dump).ok()? else {
        return None;
    };
    objects.iter().find_map(|o| {
        let props = &o["info"]["props"];
        let name = props["node.name"].as_str()?;
        (name.starts_with("audio_effect.") && props["media.class"] == "Audio/Sink")
            .then(|| name.to_owned())
    })
}

/// The Mac's built-in sound card (ALSA path `hw:Apple…`) is in the graph.
fn builtin_card(dump: &str) -> bool {
    let Ok(serde_json::Value::Array(objects)) = serde_json::from_str(dump) else {
        return false;
    };
    objects.iter().any(|o| {
        o["info"]["props"]["api.alsa.path"]
            .as_str()
            .is_some_and(|p| p.starts_with("hw:Apple"))
    })
}

/// On Apple Silicon the speakers need the Asahi DSP (or they sound thin)
/// and speakersafetyd (or the kernel keeps them quiet). JUNC routes to the
/// DSP, never to the raw speakers. `None` on other machines, and when the
/// session does not see the built-in card.
fn asahi(f: &Facts) -> Option<Check> {
    let dump = f.dump.as_deref()?;
    if !f.apple || !builtin_card(dump) {
        return None;
    }
    let dsp = dsp_sink(dump);
    Some(match (dsp, f.speakersafetyd) {
        (None, _) => check(
            "asahi",
            Status::Warn,
            tr("no Asahi DSP in front of the speakers: they sound thin and quiet").into_owned(),
            Some("sudo pacman -S --needed asahi-audio"),
        ),
        (Some(_), Proc::Installed) => check(
            "asahi",
            Status::Down,
            tr("speakersafetyd is not running: the speakers stay quiet to protect them")
                .into_owned(),
            Some("sudo systemctl enable --now speakersafetyd"),
        ),
        (Some(_), Proc::Missing) => check(
            "asahi",
            Status::Missing,
            tr("speakersafetyd is not installed: the speakers stay quiet to protect them")
                .into_owned(),
            Some("sudo pacman -S speakersafetyd"),
        ),
        (Some(d), Proc::Running) => check(
            "asahi",
            Status::Ok,
            trf(
                "the speakers play through the Asahi DSP ({dsp}); speakersafetyd running",
                &[("dsp", &d)],
            ),
            None,
        ),
    })
}

/// All checks, in the order a person would fix them.
pub fn checks(f: &Facts) -> Vec<Check> {
    let mut out = vec![
        pipewire(f),
        service(
            "wireplumber",
            "wireplumber",
            f.wireplumber,
            &tr("nothing gets connected without it"),
        ),
        service(
            "pipewire-pulse",
            "pipewire-pulse",
            f.pipewire_pulse,
            &tr("apps that use PulseAudio cannot be routed"),
        ),
        jack(f),
        clock(f),
        rates(f),
    ];
    out.extend(asahi(f));
    out.push(juncd(f.daemon.as_ref()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTINGS: &str = "Found \"settings\" metadata 31\n\
        update: id:0 key:'clock.rate' value:'48000' type:''\n\
        update: id:0 key:'clock.allowed-rates' value:'[ 48000 ]' type:''\n\
        update: id:0 key:'clock.quantum' value:'1024' type:''\n";

    /// A healthy session without juncd.
    fn healthy() -> Facts {
        Facts {
            pipewire_version: Some("1.6.9".into()),
            pipewire_socket: true,
            wireplumber: Proc::Running,
            pipewire_pulse: Proc::Running,
            pipewire_jack: Some(true),
            jack2: Some(false),
            settings: Some(SETTINGS.into()),
            dump: Some(fixture("komplete-audio-2-stereo.json")),
            daemon: None,
            apple: false,
            speakersafetyd: Proc::Missing,
        }
    }

    #[test]
    fn an_asahi_mac() {
        let mac = Facts {
            dump: Some(fixture("asahi-m1-j293.json")),
            apple: true,
            speakersafetyd: Proc::Running,
            ..healthy()
        };
        golden("asahi", &mac);
        let no_dsp = Facts {
            dump: Some(fixture("asahi-m1-j293.json").replace("audio_effect.", "effect.")),
            ..mac.clone()
        };
        // A session that does not see the built-in card (an isolated
        // PipeWire): nothing to say.
        let blind = Facts {
            dump: Some(fixture("komplete-audio-2-stereo.json")),
            ..mac.clone()
        };
        assert!(asahi(&blind).is_none());
        let quiet = Facts {
            speakersafetyd: Proc::Installed,
            ..mac
        };
        let got: Vec<Check> = [no_dsp, quiet].iter().filter_map(asahi).collect();
        golden_value("asahi-problems", &got);
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/graphs")
                .join(name),
        )
        .expect("fixture")
    }

    /// An HDMI output that only does 44.1 kHz next to a graph at 48 kHz.
    const HDMI: &str = r#"[{"id": 50, "type": "PipeWire:Interface:Node", "info": {
        "props": {"node.name": "alsa_output.hdmi", "node.description": "HDMI Output",
                  "media.class": "Audio/Sink", "device.api": "alsa"},
        "params": {"EnumFormat": [{"rate": {"default": 44100, "alternatives": [44100, 32000]}}]}}}]"#;

    #[test]
    fn a_device_that_cannot_follow_the_graph_rate() {
        golden(
            "rate-mismatch",
            &Facts {
                dump: Some(HDMI.into()),
                ..healthy()
            },
        );
        // Once its rate is allowed, PipeWire can switch: no issue.
        let allowed = SETTINGS.replace("[ 48000 ]", "[ 44100 48000 ]");
        let f = Facts {
            dump: Some(HDMI.into()),
            settings: Some(allowed),
            ..healthy()
        };
        assert_eq!(rates(&f).status, Status::Ok);
    }

    #[test]
    fn rates_parse_every_shape() {
        use serde_json::json;
        assert_eq!(Rates::parse(&json!(48000)), Some(Rates::One(48000)));
        let r =
            Rates::parse(&json!({"default": 48000, "min": 44100, "max": 96000})).expect("range");
        assert!(r.supports(88200) && !r.supports(32000));
        let l =
            Rates::parse(&json!({"default": 44100, "alternatives": [44100, 32000]})).expect("list");
        assert!(l.supports(32000) && !l.supports(48000) && l.own() == 44100);
        assert_eq!(rate_list("[ 44100 48000 ]"), [44100, 48000]);
        assert_eq!(
            (khz(44100), khz(48000)),
            ("44.1 kHz".into(), "48 kHz".into())
        );
    }

    /// Compares the checks' JSON with tests/golden/doctor/<name>.json;
    /// `JUNC_BLESS=1` rewrites it.
    fn golden(name: &str, f: &Facts) {
        golden_value(name, &checks(f));
    }

    fn golden_value(name: &str, checks: &[Check]) {
        let json = serde_json::to_string_pretty(checks).expect("json") + "\n";
        let file = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/golden/doctor")
            .join(format!("{name}.json"));
        if std::env::var_os("JUNC_BLESS").is_some() {
            std::fs::create_dir_all(file.parent().expect("dir")).expect("mkdir");
            std::fs::write(&file, &json).expect("write golden");
        }
        let want = std::fs::read_to_string(&file).unwrap_or_default();
        assert!(
            want == json,
            "{} differs (JUNC_BLESS=1 cargo test -p junc-cli --bin junc doctor rewrites it):\n{json}",
            file.display()
        );
    }

    #[test]
    fn a_healthy_session() {
        golden("healthy", &healthy());
    }

    #[test]
    fn a_broken_session() {
        let f = Facts {
            pipewire_version: Some("0.3.80".into()),
            wireplumber: Proc::Installed,
            pipewire_pulse: Proc::Missing,
            pipewire_jack: Some(false),
            jack2: Some(true),
            settings: None,
            dump: None,
            ..healthy()
        };
        golden("broken", &f);
    }

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
