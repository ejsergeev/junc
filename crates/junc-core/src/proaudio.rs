// SPDX-License-Identifier: GPL-3.0-or-later

//! Pro Audio (F15, docs/architecture.md §3.8): which card a device Input
//! belongs to, how many channels its Pro Audio profile would give, and the
//! groups of eight the channels are shown in.
//!
//! PipeWire reports channel counts only for the nodes of the active
//! profile. For USB cards the full count comes from ALSA's own stream
//! description (`/proc/asound/card<N>/stream*`), read without switching
//! profiles; the backend parses it with [`parse_usb_stream`] and stores it
//! on the [`Device`].

use crate::graph::{Device, Graph, Node, NodeKind};

/// The profile that exposes every channel of a card.
pub const PRO_AUDIO: &str = "pro-audio";

/// Channel counts of a USB card from ALSA's stream description: the widest
/// playback and capture format of any interface.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsbChannels {
    /// Output channels.
    pub playback: u32,
    /// Input channels.
    pub capture: u32,
}

/// Parses `/proc/asound/card<N>/stream<M>` (one file per USB stream
/// interface; merge several with [`UsbChannels::merge`]). Returns `None`
/// when the text has no channel counts.
pub fn parse_usb_stream(text: &str) -> Option<UsbChannels> {
    let mut out = UsbChannels::default();
    let mut section = None;
    for line in text.lines() {
        let t = line.trim();
        match t {
            "Playback:" => section = Some(true),
            "Capture:" => section = Some(false),
            _ => {}
        }
        if let (Some(playback), Some(n)) = (section, t.strip_prefix("Channels:")) {
            let Ok(n) = n.trim().parse::<u32>() else {
                continue;
            };
            let slot = if playback {
                &mut out.playback
            } else {
                &mut out.capture
            };
            *slot = (*slot).max(n);
        }
    }
    (out != UsbChannels::default()).then_some(out)
}

impl UsbChannels {
    /// The wider of two descriptions, per direction.
    #[must_use]
    pub fn merge(self, other: Self) -> Self {
        Self {
            playback: self.playback.max(other.playback),
            capture: self.capture.max(other.capture),
        }
    }
}

/// What a card's Pro Audio profile would add for recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProAudioOffer {
    /// Input channels in the active profile.
    pub now: u32,
    /// Input channels in Pro Audio, when ALSA tells (USB cards); `None`
    /// means "Pro Audio available" without a number.
    pub all: Option<u32>,
}

/// The card's widest capture node in its active profile.
pub fn capture_channels(graph: &Graph, device: &Device) -> u32 {
    card_sources(graph, device)
        .filter_map(|n| n.props.number("audio.channels"))
        .max()
        .unwrap_or(0)
}

/// Whether to offer "Use all channels" for a card, and with what numbers.
/// Nothing when Pro Audio is already active or not offered, or when ALSA
/// says it gives no more inputs than now (Komplete Audio 2: 2 = 2).
pub fn offer(graph: &Graph, device: &Device) -> Option<ProAudioOffer> {
    if !device.can_switch_to_pro_audio() {
        return None;
    }
    let now = capture_channels(graph, device);
    match device.usb_channels.map(|c| c.capture) {
        Some(all) if all <= now => None,
        all => Some(ProAudioOffer { now, all }),
    }
}

/// Channels shown in groups of eight: `[[1, 8], [9, 16], [17, 18]]`.
pub fn channel_groups(channels: u32) -> Vec<[u32; 2]> {
    (1..=channels)
        .step_by(8)
        .map(|first| [first, (first + 7).min(channels)])
        .collect()
}

/// `device.name` of the card a device match points at. A match names a
/// node (`alsa_input.<card>.<profile part>`) or the card itself
/// (`alsa_card.<card>`); the node may not exist in the active profile, so
/// the card comes from the name when the graph cannot tell.
pub fn card_name(graph: &Graph, device: &str) -> Option<String> {
    if let Some(d) = graph
        .nodes
        .values()
        .find(|n| n.name() == device)
        .and_then(|n| device_of(graph, n))
    {
        return Some(d.name().to_owned());
    }
    if device.starts_with("alsa_card.") {
        return Some(device.to_owned());
    }
    let rest = device
        .strip_prefix("alsa_input.")
        .or_else(|| device.strip_prefix("alsa_output."))?;
    let (card, _) = rest.rsplit_once('.')?;
    Some(format!("alsa_card.{card}"))
}

/// The card (PipeWire device) a device match points at, if present.
pub fn card<'g>(graph: &'g Graph, device: &str) -> Option<&'g Device> {
    let name = card_name(graph, device)?;
    graph.devices.values().find(|d| d.name() == name)
}

/// The device a node belongs to (`device.id`).
pub fn device_of<'g>(graph: &'g Graph, node: &Node) -> Option<&'g Device> {
    graph.devices.get(&node.props.number("device.id")?)
}

/// Capture nodes of a card, as they are in its active profile.
pub fn card_sources<'g>(graph: &'g Graph, device: &'g Device) -> impl Iterator<Item = &'g Node> {
    graph.nodes.values().filter(move |n| {
        n.kind() == NodeKind::Source
            && !n.is_junc()
            && n.props.number("device.id") == Some(device.id)
    })
}
