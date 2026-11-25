// SPDX-License-Identifier: GPL-3.0-or-later

//! Backend-neutral view of the live audio graph (docs/architecture.md §3.3).
//!
//! The PipeWire backend fills it from registry globals; tests and
//! diagnostics fill it from `pw-dump` JSON ([`Graph::from_pw_dump`]). Only
//! objects the client can see are here: nodes hidden by permissions (such as
//! the Asahi speaker nodes) simply do not exist for JUNC.

use serde_json::Value;
use std::collections::BTreeMap;

/// Object properties as strings (PipeWire properties are strings; `pw-dump`
/// turns some into JSON numbers and booleans, which are converted back).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Props(BTreeMap<String, String>);

impl Props {
    /// Value of a property.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    /// Boolean property (`true`/`1`).
    pub fn flag(&self, key: &str) -> bool {
        matches!(self.get(key), Some("true" | "1"))
    }

    /// Numeric property.
    pub fn number(&self, key: &str) -> Option<u32> {
        self.get(key)?.parse().ok()
    }

    /// Sets a property.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.0.insert(key.into(), value.into());
    }

    /// All properties.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    fn from_json(v: Option<&Value>) -> Self {
        let mut p = Self::default();
        if let Some(Value::Object(map)) = v {
            for (k, v) in map {
                let s = match v {
                    Value::String(s) => s.clone(),
                    Value::Null => continue,
                    other => other.to_string(),
                };
                p.set(k.clone(), s);
            }
        }
        p
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for Props {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self(
            iter.into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        )
    }
}

/// What a node is, from JUNC's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// An app playing audio (`Stream/Output/Audio`).
    AppPlayback,
    /// An app recording audio (`Stream/Input/Audio`).
    AppCapture,
    /// A device or virtual device apps play into (`Audio/Sink`).
    Sink,
    /// A device or virtual device apps record from (`Audio/Source…`).
    Source,
    /// `Audio/Duplex`.
    Duplex,
    /// Internal plumbing (`…/Internal`), e.g. filter chains and JUNC gain nodes.
    Internal,
    /// Anything else (video, MIDI, drivers).
    Other,
}

/// A node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Registry id (changes across restarts; never stored in config).
    pub id: u32,
    /// Properties.
    pub props: Props,
}

impl Node {
    /// `node.name`.
    pub fn name(&self) -> &str {
        self.props.get("node.name").unwrap_or("")
    }

    /// Human name: `node.description`, else `node.nick`, else the name.
    pub fn description(&self) -> &str {
        self.props
            .get("node.description")
            .or_else(|| self.props.get("node.nick"))
            .unwrap_or(self.name())
    }

    /// `media.class`.
    pub fn media_class(&self) -> &str {
        self.props.get("media.class").unwrap_or("")
    }

    /// Classification of the node.
    pub fn kind(&self) -> NodeKind {
        let class = self.media_class();
        if class.ends_with("/Internal") {
            return NodeKind::Internal;
        }
        match class {
            "Stream/Output/Audio" => NodeKind::AppPlayback,
            "Stream/Input/Audio" => NodeKind::AppCapture,
            "Audio/Sink" => NodeKind::Sink,
            "Audio/Source" | "Audio/Source/Virtual" => NodeKind::Source,
            "Audio/Duplex" => NodeKind::Duplex,
            _ => NodeKind::Other,
        }
    }

    /// Created and owned by JUNC (`junc.managed`).
    pub fn is_junc(&self) -> bool {
        self.props.flag("junc.managed")
    }

    /// One half of a filter chain or loopback (`node.link-group` set), such as
    /// the Asahi DSP streams: plumbing, never an app the user would pick.
    pub fn is_plumbing(&self) -> bool {
        self.props.get("node.link-group").is_some()
    }

    /// An app stream a user would recognise: playback or capture, not JUNC's
    /// own and not plumbing.
    pub fn is_app_stream(&self) -> bool {
        matches!(self.kind(), NodeKind::AppPlayback | NodeKind::AppCapture)
            && !self.is_junc()
            && !self.is_plumbing()
    }

    /// PipeWire device this node belongs to, if any.
    pub fn device_id(&self) -> Option<u32> {
        self.props.number("device.id")
    }
}

/// Port direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Input port (audio goes in).
    In,
    /// Output port (audio comes out), including monitor ports.
    Out,
}

/// A port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    /// Registry id.
    pub id: u32,
    /// Owning node.
    pub node: u32,
    /// Direction.
    pub direction: Direction,
    /// Position among the node's ports of this direction (`port.id`), which
    /// is the channel order.
    pub index: u32,
    /// `audio.channel` (FL, FR, AUX0, MONO…).
    pub channel: Option<String>,
    /// A monitor port of a sink.
    pub monitor: bool,
    /// `port.name` (for logs only; never used to find a port).
    pub name: String,
}

/// A link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// Registry id.
    pub id: u32,
    /// Output node.
    pub output_node: u32,
    /// Output port.
    pub output_port: u32,
    /// Input node.
    pub input_node: u32,
    /// Input port.
    pub input_port: u32,
    /// Properties, including JUNC's `junc.*` tags.
    pub props: Props,
}

impl Link {
    /// Created and owned by JUNC (`junc.managed`).
    pub fn is_junc(&self) -> bool {
        self.props.flag("junc.managed")
    }
}

/// A device profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// Profile index for `Profile` params.
    pub index: u32,
    /// Profile name, e.g. `pro-audio`, `a2dp-sink`.
    pub name: String,
    /// Human description.
    pub description: String,
    /// Not reported unavailable (`yes` or `unknown`).
    pub available: bool,
}

/// A device (card).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// Registry id.
    pub id: u32,
    /// Properties.
    pub props: Props,
    /// Profiles it offers.
    pub profiles: Vec<Profile>,
    /// Active profile name.
    pub active_profile: Option<String>,
    /// Channel counts ALSA reports for a USB card, whatever the profile
    /// (F15); `None` for other cards or when unknown.
    pub usb_channels: Option<crate::proaudio::UsbChannels>,
}

impl Device {
    /// `device.name`.
    pub fn name(&self) -> &str {
        self.props.get("device.name").unwrap_or("")
    }

    /// Bluetooth device.
    pub fn is_bluetooth(&self) -> bool {
        self.props.get("device.api") == Some("bluez5")
    }

    /// Its profiles come from ALSA UCM (`HiFi`, …) rather than ACP's
    /// generated `output:…+input:…` ones. UCM sets up the card's amplifiers
    /// and DSP (the Asahi built-in speakers); Pro Audio would bypass that.
    pub fn uses_ucm(&self) -> bool {
        let mut others = self
            .profiles
            .iter()
            .map(|p| p.name.as_str())
            .filter(|n| *n != "off" && *n != "pro-audio")
            .peekable();
        others.peek().is_some() && others.all(|n| !n.contains(':'))
    }

    /// Has a `pro-audio` profile that is not the active one, and is not a
    /// UCM card (see [`Self::uses_ucm`]).
    pub fn can_switch_to_pro_audio(&self) -> bool {
        !self.uses_ucm()
            && self.active_profile.as_deref() != Some("pro-audio")
            && self
                .profiles
                .iter()
                .any(|p| p.name == "pro-audio" && p.available)
    }
}

/// Default devices from the `default` metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Defaults {
    /// Sink apps play to now (`default.audio.sink`).
    pub sink: Option<String>,
    /// Source apps record from now (`default.audio.source`).
    pub source: Option<String>,
    /// User choice (`default.configured.audio.sink`), may differ from `sink`.
    pub configured_sink: Option<String>,
    /// User choice (`default.configured.audio.source`).
    pub configured_source: Option<String>,
}

/// The visible graph.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Graph {
    /// Nodes by id.
    pub nodes: BTreeMap<u32, Node>,
    /// Ports by id.
    pub ports: BTreeMap<u32, Port>,
    /// Links by id.
    pub links: BTreeMap<u32, Link>,
    /// Devices by id.
    pub devices: BTreeMap<u32, Device>,
    /// Defaults.
    pub defaults: Defaults,
    /// `target.object` of streams in the `default` metadata, by stream node
    /// id: where the user (or JUNC, for a Cable) sent the stream.
    pub targets: BTreeMap<u32, String>,
}

impl Graph {
    /// Node with this `node.name`, if exactly one is visible.
    pub fn node_by_name(&self, name: &str) -> Option<&Node> {
        let mut it = self.nodes.values().filter(|n| n.name() == name);
        let first = it.next()?;
        it.next().is_none().then_some(first)
    }

    /// Ports of a node in one direction, in channel order (`port.id`).
    pub fn ports_of(&self, node: u32, direction: Direction) -> impl Iterator<Item = &Port> {
        let mut ports: Vec<&Port> = self
            .ports
            .values()
            .filter(|p| p.node == node && p.direction == direction)
            .collect();
        ports.sort_by_key(|p| (p.index, p.id));
        ports.into_iter()
    }

    /// Port of `node` for 1-based channel `ch` in a direction.
    pub fn channel_port(&self, node: u32, direction: Direction, ch: u8) -> Option<&Port> {
        self.ports_of(node, direction)
            .nth(usize::from(ch).checked_sub(1)?)
    }

    /// Device of a node.
    pub fn device_of(&self, node: &Node) -> Option<&Device> {
        self.devices.get(&node.device_id()?)
    }

    /// Builds a graph from `pw-dump` output. Unknown object types are ignored.
    pub fn from_pw_dump(json: &str) -> Result<Self, String> {
        let objects: Vec<Value> =
            serde_json::from_str(json).map_err(|e| format!("pw-dump JSON: {e}"))?;
        let mut g = Self::default();
        for o in &objects {
            let id = o
                .get("id")
                .and_then(Value::as_u64)
                .and_then(|i| u32::try_from(i).ok());
            let ty = o.get("type").and_then(Value::as_str).unwrap_or("");
            let info = o.get("info");
            let props = Props::from_json(info.and_then(|i| i.get("props")));
            let Some(id) = id else { continue };
            match ty {
                "PipeWire:Interface:Node" => {
                    g.nodes.insert(id, Node { id, props });
                }
                "PipeWire:Interface:Port" => {
                    let Some(node) = props.number("node.id") else {
                        continue;
                    };
                    let direction = match info
                        .and_then(|i| i.get("direction"))
                        .and_then(Value::as_str)
                    {
                        Some("input") => Direction::In,
                        Some("output") => Direction::Out,
                        _ if props.get("port.direction") == Some("in") => Direction::In,
                        _ => Direction::Out,
                    };
                    g.ports.insert(
                        id,
                        Port {
                            id,
                            node,
                            direction,
                            index: props.number("port.id").unwrap_or(0),
                            channel: props.get("audio.channel").map(str::to_owned),
                            monitor: props.flag("port.monitor"),
                            name: props.get("port.name").unwrap_or("").to_owned(),
                        },
                    );
                }
                "PipeWire:Interface:Link" => {
                    let num = |k: &str| {
                        info.and_then(|i| i.get(k))
                            .and_then(Value::as_u64)
                            .and_then(|v| u32::try_from(v).ok())
                    };
                    let (Some(on), Some(op), Some(inn), Some(ip)) = (
                        num("output-node-id"),
                        num("output-port-id"),
                        num("input-node-id"),
                        num("input-port-id"),
                    ) else {
                        continue;
                    };
                    g.links.insert(
                        id,
                        Link {
                            id,
                            output_node: on,
                            output_port: op,
                            input_node: inn,
                            input_port: ip,
                            props,
                        },
                    );
                }
                "PipeWire:Interface:Device" => {
                    let params = info.and_then(|i| i.get("params"));
                    let profiles = params
                        .and_then(|p| p.get("EnumProfile"))
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(parse_profile).collect())
                        .unwrap_or_default();
                    let active_profile = params
                        .and_then(|p| p.get("Profile"))
                        .and_then(Value::as_array)
                        .and_then(|a| a.first())
                        .and_then(|p| p.get("name"))
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    g.devices.insert(
                        id,
                        Device {
                            id,
                            props,
                            profiles,
                            active_profile,
                            usb_channels: None,
                        },
                    );
                }
                "PipeWire:Interface:Metadata"
                    if o.get("props")
                        .and_then(|p| p.get("metadata.name"))
                        .and_then(Value::as_str)
                        == Some("default") =>
                {
                    read_defaults(o.get("metadata"), &mut g.defaults);
                    read_targets(o.get("metadata"), &mut g.targets);
                }
                _ => {}
            }
        }
        Ok(g)
    }
}

fn parse_profile(p: &Value) -> Option<Profile> {
    Some(Profile {
        index: u32::try_from(p.get("index")?.as_u64()?).ok()?,
        name: p.get("name")?.as_str()?.to_owned(),
        description: p
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        available: p.get("available").and_then(Value::as_str) != Some("no"),
    })
}

fn read_targets(entries: Option<&Value>, t: &mut BTreeMap<u32, String>) {
    for e in entries.and_then(Value::as_array).into_iter().flatten() {
        let subject = e
            .get("subject")
            .and_then(Value::as_u64)
            .and_then(|s| u32::try_from(s).ok());
        if let (Some(s), Some("target.object"), Some(v)) = (
            subject.filter(|s| *s != 0),
            e.get("key").and_then(Value::as_str),
            e.get("value").and_then(Value::as_str),
        ) {
            t.insert(s, v.to_owned());
        }
    }
}

fn read_defaults(entries: Option<&Value>, d: &mut Defaults) {
    for e in entries.and_then(Value::as_array).into_iter().flatten() {
        if e.get("subject").and_then(Value::as_u64) != Some(0) {
            continue;
        }
        // Values are JSON objects `{"name": "..."}` (sometimes as a string).
        let name = match e.get("value") {
            Some(Value::Object(m)) => m.get("name").and_then(Value::as_str).map(str::to_owned),
            Some(Value::String(s)) => serde_json::from_str::<Value>(s)
                .ok()
                .and_then(|v| v.get("name").and_then(Value::as_str).map(str::to_owned)),
            _ => None,
        };
        match e.get("key").and_then(Value::as_str) {
            Some("default.audio.sink") => d.sink = name,
            Some("default.audio.source") => d.source = name,
            Some("default.configured.audio.sink") => d.configured_sink = name,
            Some("default.configured.audio.source") => d.configured_source = name,
            _ => {}
        }
    }
}
