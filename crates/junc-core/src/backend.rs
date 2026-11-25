// SPDX-License-Identifier: GPL-3.0-or-later

//! What the planner asks a backend to do, the backend trait, and an
//! in-memory backend for tests (hardware layer 1).

use crate::graph::{Direction, Graph, Link, Node, Port, Props};
use std::fmt;

/// Role of an object JUNC creates (`junc.role`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// The Junction source (the Bus apps record from).
    Junction,
    /// The Junction sink (apps play into it).
    JunctionSink,
    /// Per-Input gain node.
    InputGain,
    /// Per-Listen gain node.
    ListenGain,
    /// A link.
    Link,
}

impl Role {
    /// Value of `junc.role`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Junction => "junction",
            Self::JunctionSink => "junction-sink",
            Self::InputGain => "input-gain",
            Self::ListenGain => "listen-gain",
            Self::Link => "link",
        }
    }
}

/// A node to create through the adapter factory (`support.null-audio-sink`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSpec {
    /// `node.name`.
    pub name: String,
    /// `node.description`.
    pub description: String,
    /// `media.class`.
    pub media_class: &'static str,
    /// Channel count.
    pub channels: u8,
    /// Junction id it belongs to (`junc.junction`).
    pub junction: String,
    /// `junc.role`.
    pub role: Role,
    /// `junc.ref`.
    pub reference: String,
}

impl NodeSpec {
    /// Full property set, including the ownership tags and `object.linger`.
    pub fn props(&self) -> Props {
        let positions = channel_positions(self.channels).join(" ");
        [
            ("factory.name", "support.null-audio-sink".to_owned()),
            ("node.name", self.name.clone()),
            ("node.description", self.description.clone()),
            ("media.class", self.media_class.to_owned()),
            ("audio.channels", self.channels.to_string()),
            ("audio.position", format!("[ {positions} ]")),
            ("monitor.channel-volumes", "true".to_owned()),
            ("node.autoconnect", "false".to_owned()),
            ("node.dont-reconnect", "true".to_owned()),
            ("object.linger", "true".to_owned()),
            ("junc.managed", "true".to_owned()),
            ("junc.schema", "1".to_owned()),
            ("junc.junction", self.junction.clone()),
            ("junc.role", self.role.as_str().to_owned()),
            ("junc.ref", self.reference.clone()),
        ]
        .into_iter()
        .collect()
    }
}

/// Channel positions for a node of `n` channels: FL FR for 2, MONO for 1,
/// AUX0… otherwise.
pub fn channel_positions(n: u8) -> Vec<String> {
    match n {
        1 => vec!["MONO".into()],
        2 => vec!["FL".into(), "FR".into()],
        n => (0..n).map(|i| format!("AUX{i}")).collect(),
    }
}

/// A link to create through the link factory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSpec {
    /// Output node.
    pub output_node: u32,
    /// Output port.
    pub output_port: u32,
    /// Input node.
    pub input_node: u32,
    /// Input port.
    pub input_port: u32,
    /// Junction id (`junc.junction`).
    pub junction: String,
    /// `junc.ref`, e.g. `in/spotify/1->2`.
    pub reference: String,
}

impl LinkSpec {
    /// Full property set, including the ownership tags and `object.linger`.
    pub fn props(&self) -> Props {
        [
            ("link.output.node", self.output_node.to_string()),
            ("link.output.port", self.output_port.to_string()),
            ("link.input.node", self.input_node.to_string()),
            ("link.input.port", self.input_port.to_string()),
            ("object.linger", "true".to_owned()),
            ("junc.managed", "true".to_owned()),
            ("junc.schema", "1".to_owned()),
            ("junc.junction", self.junction.clone()),
            ("junc.role", Role::Link.as_str().to_owned()),
            ("junc.ref", self.reference.clone()),
        ]
        .into_iter()
        .collect()
    }
}

/// One change to the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Create a node.
    CreateNode(NodeSpec),
    /// Destroy a JUNC node.
    DestroyNode {
        /// Node id.
        node: u32,
    },
    /// Create a link.
    CreateLink(LinkSpec),
    /// Destroy a JUNC link.
    DestroyLink {
        /// Link id.
        link: u32,
    },
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CreateNode(n) => write!(
                f,
                "create {} node {} ({} ch)",
                n.media_class, n.name, n.channels
            ),
            Self::DestroyNode { node } => write!(f, "destroy node {node}"),
            Self::CreateLink(l) => write!(
                f,
                "link {}:{} -> {}:{} ({})",
                l.output_node, l.output_port, l.input_node, l.input_port, l.reference
            ),
            Self::DestroyLink { link } => write!(f, "unlink {link}"),
        }
    }
}

/// A backend error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendError(pub String);

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BackendError {}

/// What JUNC needs from an audio system.
pub trait Backend {
    /// The current visible graph.
    fn graph(&self) -> &Graph;
    /// Applies one action. Created objects appear in [`Backend::graph`] once
    /// the backend has seen them (immediately for the mock).
    fn apply(&mut self, action: &Action) -> Result<(), BackendError>;
}

/// In-memory backend over a [`Graph`]. Refuses to touch objects without the
/// JUNC tag, so tests catch any attempt (F17).
#[derive(Debug, Clone)]
pub struct MockBackend {
    graph: Graph,
    next_id: u32,
}

impl MockBackend {
    /// A backend starting from `graph`.
    pub fn new(graph: Graph) -> Self {
        let max = [
            graph.nodes.keys().max(),
            graph.ports.keys().max(),
            graph.links.keys().max(),
            graph.devices.keys().max(),
        ]
        .into_iter()
        .flatten()
        .max()
        .copied()
        .unwrap_or(0);
        Self {
            graph,
            next_id: max + 1000,
        }
    }

    /// Mutable access, to simulate apps and devices coming and going.
    pub fn graph_mut(&mut self) -> &mut Graph {
        &mut self.graph
    }

    fn id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    /// Removes a node with its ports and links, as PipeWire does when a
    /// client goes away or a device is unplugged.
    pub fn remove_node(&mut self, node: u32) {
        self.graph.nodes.remove(&node);
        self.graph.ports.retain(|_, p| p.node != node);
        self.graph
            .links
            .retain(|_, l| l.output_node != node && l.input_node != node);
    }

    /// Adds a node with `channels` ports in each listed direction, the way
    /// PipeWire names them for `class`. Returns the node id.
    pub fn add_node(&mut self, props: Props, channels: u8) -> u32 {
        let id = self.id();
        let class = props.get("media.class").unwrap_or("").to_owned();
        let positions = channel_positions(channels);
        let (inputs, outputs, monitor): (Option<&str>, Option<&str>, bool) = match class.as_str() {
            "Stream/Output/Audio" => (None, Some("output"), false),
            "Stream/Input/Audio" => (Some("input"), None, false),
            "Audio/Sink" => (Some("playback"), Some("monitor"), true),
            "Audio/Source" => (None, Some("capture"), false),
            "Audio/Source/Virtual" => (Some("input"), Some("capture"), false),
            _ => (Some("playback"), Some("capture"), false),
        };
        self.graph.nodes.insert(id, Node { id, props });
        for (dir, prefix) in [(Direction::In, inputs), (Direction::Out, outputs)] {
            let Some(prefix) = prefix else { continue };
            for (i, pos) in positions.iter().enumerate() {
                let pid = self.id();
                self.graph.ports.insert(
                    pid,
                    Port {
                        id: pid,
                        node: id,
                        direction: dir,
                        index: u32::try_from(i).unwrap_or(0),
                        channel: Some(pos.clone()),
                        monitor: monitor && dir == Direction::Out,
                        name: format!("{prefix}_{pos}"),
                    },
                );
            }
        }
        id
    }

    /// Removes a link as another tool would.
    pub fn remove_link(&mut self, link: u32) {
        self.graph.links.remove(&link);
    }

    /// Adds a link as another tool would (no JUNC tag).
    pub fn add_foreign_link(&mut self, output_port: u32, input_port: u32) -> Option<u32> {
        let (o, i) = (
            self.graph.ports.get(&output_port)?,
            self.graph.ports.get(&input_port)?,
        );
        let (on, inn) = (o.node, i.node);
        let id = self.id();
        self.graph.links.insert(
            id,
            Link {
                id,
                output_node: on,
                output_port,
                input_node: inn,
                input_port,
                props: Props::default(),
            },
        );
        Some(id)
    }
}

impl Backend for MockBackend {
    fn graph(&self) -> &Graph {
        &self.graph
    }

    fn apply(&mut self, action: &Action) -> Result<(), BackendError> {
        match action {
            Action::CreateNode(spec) => {
                self.add_node(spec.props(), spec.channels);
            }
            Action::DestroyNode { node } => {
                let n = self
                    .graph
                    .nodes
                    .get(node)
                    .ok_or_else(|| BackendError(format!("no node {node}")))?;
                if !n.is_junc() {
                    return Err(BackendError(format!(
                        "refusing to destroy foreign node {node}"
                    )));
                }
                self.remove_node(*node);
            }
            Action::CreateLink(spec) => {
                for (port, dir) in [
                    (spec.output_port, Direction::Out),
                    (spec.input_port, Direction::In),
                ] {
                    match self.graph.ports.get(&port) {
                        Some(p) if p.direction == dir => {}
                        _ => return Err(BackendError(format!("no {dir:?} port {port}"))),
                    }
                }
                let id = self.id();
                self.graph.links.insert(
                    id,
                    Link {
                        id,
                        output_node: spec.output_node,
                        output_port: spec.output_port,
                        input_node: spec.input_node,
                        input_port: spec.input_port,
                        props: spec.props(),
                    },
                );
            }
            Action::DestroyLink { link } => {
                let l = self
                    .graph
                    .links
                    .get(link)
                    .ok_or_else(|| BackendError(format!("no link {link}")))?;
                if !l.is_junc() {
                    return Err(BackendError(format!(
                        "refusing to destroy foreign link {link}"
                    )));
                }
                self.graph.links.remove(link);
            }
        }
        Ok(())
    }
}
