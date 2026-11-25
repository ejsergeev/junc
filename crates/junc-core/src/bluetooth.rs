// SPDX-License-Identifier: GPL-3.0-or-later

//! Bluetooth headsets (F18, docs/architecture.md §3.9): which profile a
//! headset runs, whether using its microphone would switch it to the call
//! profile (and lower playback quality), and how good the call audio is.
//!
//! With WirePlumber 0.5 a headset in high-quality playback (A2DP) already
//! shows a microphone, `bluez_input.<address>`: recording from it is what
//! makes WirePlumber switch the headset to its call profile (HFP). JUNC
//! never switches the profile itself; it only warns.

use crate::graph::{Device, Graph, Node};

/// What a Bluetooth headset is doing now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// High-quality playback (A2DP); the microphone is not live.
    Playback,
    /// Call profile (HFP/HSP): microphone live, playback at call quality.
    Call,
    /// Turned off or another profile.
    Other,
}

impl Mode {
    /// Lowercase name used in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Playback => "playback",
            Self::Call => "call",
            Self::Other => "other",
        }
    }
}

/// Audio quality of a call codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallQuality {
    /// CVSD, 8 kHz.
    Narrowband,
    /// mSBC, 16 kHz.
    Wideband,
    /// LC3-SWB, 32 kHz.
    SuperWideband,
}

impl CallQuality {
    /// Lowercase name used in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Narrowband => "narrowband",
            Self::Wideband => "wideband",
            Self::SuperWideband => "super-wideband",
        }
    }
}

/// Bluetooth facts about a headset, for its card and its microphone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Headset {
    /// Active profile name, e.g. `a2dp-sink`, `headset-head-unit`.
    pub profile: Option<String>,
    /// What that profile means.
    pub mode: Mode,
    /// Codec the call profile uses (`cvsd`, `msbc`, `lc3-swb`), when known.
    pub call_codec: Option<String>,
    /// Its quality class.
    pub call_quality: Option<CallQuality>,
}

/// Mode of a profile name.
pub fn mode(profile: Option<&str>) -> Mode {
    match profile {
        Some(p) if p.starts_with("a2dp") => Mode::Playback,
        Some(p)
            if p.starts_with("headset-head-unit")
                || p.starts_with("hsp")
                || p.starts_with("hfp") =>
        {
            Mode::Call
        }
        _ => Mode::Other,
    }
}

/// Codec name normalised: `lc3_swb` and `LC3-SWB` both give `lc3-swb`.
fn normalise(codec: &str) -> String {
    codec.trim().to_ascii_lowercase().replace('_', "-")
}

/// Quality class of a call codec.
pub fn call_quality(codec: &str) -> Option<CallQuality> {
    match normalise(codec).as_str() {
        "cvsd" => Some(CallQuality::Narrowband),
        "msbc" => Some(CallQuality::Wideband),
        "lc3-swb" => Some(CallQuality::SuperWideband),
        _ => None,
    }
}

/// The codec the call profile would use. In a call, the nodes say
/// (`api.bluez5.codec`); otherwise the call profile WirePlumber picks
/// (`headset-head-unit`, the best one) names it in its description:
/// "Headset Head Unit (HSP/HFP, codec LC3-SWB)".
fn call_codec(graph: &Graph, device: &Device) -> Option<String> {
    let from_nodes = graph
        .nodes
        .values()
        .filter(|n| n.props.number("device.id") == Some(device.id))
        .filter(|n| mode(n.props.get("api.bluez5.profile")) == Mode::Call)
        .find_map(|n| n.props.get("api.bluez5.codec"));
    if let Some(c) = from_nodes {
        return Some(normalise(c));
    }
    let p = device
        .profiles
        .iter()
        .find(|p| p.name == "headset-head-unit")?;
    let (_, rest) = p.description.split_once("codec ")?;
    Some(normalise(rest.trim_end_matches(')')))
}

/// Facts about a Bluetooth card; `None` for other cards.
pub fn headset(graph: &Graph, device: &Device) -> Option<Headset> {
    if !device.is_bluetooth() {
        return None;
    }
    let call_codec = call_codec(graph, device);
    Some(Headset {
        profile: device.active_profile.clone(),
        mode: mode(device.active_profile.as_deref()),
        call_quality: call_codec.as_deref().and_then(call_quality),
        call_codec,
    })
}

/// Facts about the headset a microphone node belongs to; `None` for other
/// nodes.
pub fn mic(graph: &Graph, node: &Node) -> Option<Headset> {
    if !node.name().starts_with("bluez_input.") {
        return None;
    }
    let device = graph.devices.get(&node.props.number("device.id")?)?;
    headset(graph, device)
}

/// Using this microphone would switch its headset to the call profile and
/// lower playback quality (F18): the warning the picker and the CLI show.
pub fn mic_switches_profile(graph: &Graph, node: &Node) -> bool {
    mic(graph, node).is_some_and(|h| h.mode == Mode::Playback)
}
