// SPDX-License-Identifier: GPL-3.0-or-later

//! Desired-state model: what `junctions.toml` (schema v1) describes.
//!
//! These types are the single source of truth for the config format: serde
//! reads and writes them, schemars turns them into the published JSON Schema
//! (`junc schema`), and the validator checks the rules serde cannot express.
//! Field docs become schema descriptions, so they are written for users and
//! agents.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Current config schema version.
pub const SCHEMA_VERSION: u32 = 1;
/// Largest Bus (F3).
pub const MAX_BUS_CHANNELS: u8 = 64;
/// Volumes are percentages, 0–150, mapped cubically to gain like PulseAudio.
pub const MAX_VOLUME: u16 = 150;

/// Linear gain for a volume percentage: cubic, as desktop mixers (and
/// PulseAudio) map their sliders, so 50 % sounds about half as loud.
/// 100 % is exactly 1.0, which keeps the path bit-exact.
pub fn gain(percent: u16) -> f32 {
    let x = f32::from(percent) / 100.0;
    x * x * x
}

/// A `junctions.toml` file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "JUNC configuration", rename = "JuncConfig")]
pub struct Config {
    /// Config schema version. This build reads 0 (migrated) and 1.
    pub schema: u32,
    /// Junctions and Cables, in the order the sidebar shows them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub junction: Vec<Junction>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema: SCHEMA_VERSION,
            junction: Vec::new(),
        }
    }
}

/// A named virtual device (or a Cable, a Junction that carries one app into
/// another).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Junction {
    /// Stable id: lowercase letters, digits and `-`. The PipeWire node is
    /// named `junc.<id with - replaced by _>`.
    #[schemars(regex(pattern = r"^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$"))]
    pub id: String,
    /// Name shown to people and to other apps (any script).
    pub name: String,
    /// `junction` or `cable`.
    #[serde(default)]
    pub kind: Kind,
    /// Off keeps the config but routes nothing; apps see silence (F8).
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Master volume in percent (F7).
    #[serde(default = "full")]
    #[schemars(range(max = 150))]
    pub volume: u16,
    /// Use this Junction as the system default input or output (F16).
    #[serde(default)]
    pub default: DefaultRole,
    /// Sample rate: `"follow"` the graph, or a fixed rate in Hz.
    #[serde(default)]
    pub rate: Rate,
    /// The Bus: the channels apps record from (F3).
    #[serde(default)]
    pub bus: Bus,
    /// What feeds the Bus (F2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<Input>,
    /// Outputs used to hear the Bus (F6).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub listen: Vec<Listen>,
    /// Required for `kind = "cable"`: the app that hears the Cable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cable: Option<Cable>,
}

/// Kind of a Junction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Mixes any Inputs into a Bus.
    #[default]
    Junction,
    /// One app straight into another app (F5).
    Cable,
}

/// Default-device role of a Junction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DefaultRole {
    /// Not a system default.
    #[default]
    None,
    /// System default input (apps that record from "default" get the Bus).
    Input,
    /// System default output (apps that play to "default" feed the Junction).
    Output,
}

/// Sample rate setting: `"follow"` or a rate in Hz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Rate {
    /// `"follow"`: use the graph's rate.
    Follow(Follow),
    /// Fixed rate in Hz, 44100–192000.
    Hz(#[schemars(range(min = 44100, max = 192000))] u32),
}

impl Default for Rate {
    fn default() -> Self {
        Self::Follow(Follow::Follow)
    }
}

/// The literal `"follow"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Follow {
    /// Follow the graph's rate.
    Follow,
}

/// The Bus of a Junction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bus {
    /// Number of channels, 1–64. Two channels are FL FR; more are AUX0….
    #[schemars(range(min = 1, max = 64))]
    pub channels: u8,
}

impl Default for Bus {
    fn default() -> Self {
        Self { channels: 2 }
    }
}

/// A source feeding a Junction's Bus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    /// Stable id within the Junction.
    #[schemars(regex(pattern = r"^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$"))]
    pub id: String,
    /// Which app stream, device, monitor or Junction this is (by identity,
    /// never by PipeWire node id).
    #[serde(rename = "match")]
    pub matcher: Match,
    /// Off keeps the Input and its Links in the config but routes nothing.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Input volume in percent (F7).
    #[serde(default = "full")]
    #[schemars(range(max = 150))]
    pub volume: u16,
    /// Remove the app from the default output while it is captured (F11).
    #[serde(default, skip_serializing_if = "is_false")]
    pub exclusive: bool,
    /// `[input channel, bus channel]` pairs, 1-based. Omitted = default mapping (F4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<[u8; 2]>>,
    /// Links another tool removed that the user chose to leave as is (F17).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suppressed: Vec<[u8; 2]>,
}

/// How an Input or Cable target is identified. Exactly one kind of key must be
/// given: app keys (`app`, `binary`, `role`, `node`), `device`, `monitor`,
/// `junction`, or `slot` (presets and templates only).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Match {
    /// App name (`application.name`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// App executable (`application.process.binary`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary: Option<String>,
    /// Media role (`media.role`), narrows an app match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Stream node name (`node.name`), narrows an app match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// Hardware input or output by stable `node.name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// System audio: the monitor of the sink with this `node.name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<String>,
    /// Another Junction, by id (nesting).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub junction: Option<String>,
    /// Template slot to fill in (presets and templates only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
    /// Device profile to apply when the device appears, e.g. `"pro-audio"` (F15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

/// Which kind of thing a [`Match`] identifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    /// An app stream.
    App,
    /// A hardware device.
    Device,
    /// A sink monitor.
    Monitor,
    /// Another Junction.
    Junction,
    /// A template slot.
    Slot,
}

impl MatchKind {
    /// Lowercase name used in messages.
    pub fn label(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Device => "device",
            Self::Monitor => "monitor",
            Self::Junction => "junction",
            Self::Slot => "slot",
        }
    }
}

impl Match {
    /// The kinds of keys present (valid matches have exactly one).
    pub fn kinds(&self) -> Vec<MatchKind> {
        let mut k = Vec::new();
        if self.app.is_some() || self.binary.is_some() || self.role.is_some() || self.node.is_some()
        {
            k.push(MatchKind::App);
        }
        if self.device.is_some() {
            k.push(MatchKind::Device);
        }
        if self.monitor.is_some() {
            k.push(MatchKind::Monitor);
        }
        if self.junction.is_some() {
            k.push(MatchKind::Junction);
        }
        if self.slot.is_some() {
            k.push(MatchKind::Slot);
        }
        k
    }
}

/// An output used to hear the Bus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Listen {
    /// Stable id within the Junction.
    #[schemars(regex(pattern = r"^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$"))]
    pub id: String,
    /// `"default"` follows the system output; otherwise an output `node.name`.
    pub target: String,
    /// Off keeps the Listen output in the config but routes nothing.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Listen volume in percent (F7).
    #[serde(default = "full")]
    #[schemars(range(max = 150))]
    pub volume: u16,
    /// `[bus channel, output channel]` pairs, 1-based. Omitted = default mapping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<[u8; 2]>>,
    /// Links another tool removed that the user chose to leave as is (F17).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suppressed: Vec<[u8; 2]>,
}

/// Target of a Cable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Cable {
    /// The app that hears the Cable as its input.
    pub to: Match,
}

fn yes() -> bool {
    true
}

fn full() -> u16 {
    100
}

fn is_false(b: &bool) -> bool {
    !*b
}
