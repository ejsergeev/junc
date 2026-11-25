// SPDX-License-Identifier: GPL-3.0-or-later

//! The reconcile planner (docs/architecture.md §3.4): a pure function from
//! the desired config and the observed graph to the actions that bring the
//! graph closer to the config. The daemon runs it after every graph or config
//! change; `junc apply --dry-run` prints its output.
//!
//! Object layout per Junction (§3.2, spike S1): a source `junc.<id>` (the Bus
//! apps record from), a sink `junc.<id>.sink` whose monitor feeds the source,
//! one gain node per enabled Input and per enabled Listen output. Gain nodes
//! are as wide as the Bus: Input channels link straight into the Bus channel
//! they feed, and the gain node feeds the Bus one to one.
//!
//! Safety (F17): only objects tagged `junc.managed` are ever changed or
//! destroyed, and links of the GUI meters (`junc.role = meter`) are left alone.

use crate::backend::{Action, LinkSpec, NodeSpec, Role};
use crate::graph::{Direction, Graph, Node};
use crate::matcher::{Use, junction_node_name, resolve, resolve_output};
use crate::model::{Config, Junction, Kind};
use std::collections::{BTreeMap, HashSet};

/// Links the daemon must not (re)create right now, e.g. a link another tool
/// removed while the user has not chosen Restore or Leave as is (§3.5).
/// Entries are `(junction id, junc.ref)`.
pub type Held = HashSet<(String, String)>;

/// State of one config entry after planning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Routed.
    Active,
    /// Configured but its app or device is not there.
    Waiting,
    /// Turned off.
    Disabled,
}

/// Which config entry a status is about.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Entity {
    /// A Junction or Cable.
    Junction,
    /// An Input, by id.
    Input(String),
    /// A Listen output, by id.
    Listen(String),
    /// The app a Cable delivers to.
    CableTarget,
}

/// Status of an entry, with the nodes it resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityStatus {
    /// Junction id.
    pub junction: String,
    /// The entry.
    pub entity: Entity,
    /// Its state.
    pub status: Status,
    /// Live nodes it matched (apps can match several streams).
    pub nodes: Vec<u32>,
}

/// Result of planning.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Actions in a safe order: unlink, destroy, create nodes, link.
    pub actions: Vec<Action>,
    /// Status of every config entry.
    pub status: Vec<EntityStatus>,
    /// Nodes whose new name waits until no app uses them (spike S10).
    pub deferred: Vec<String>,
}

impl Plan {
    /// Nothing to do.
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

/// `node.name` of the Junction sink.
pub fn sink_node_name(junction: &str) -> String {
    format!("{}.sink", junction_node_name(junction))
}

fn gain_node_name(junction: &str, input: &str) -> String {
    format!(
        "{}.in.{}",
        junction_node_name(junction),
        input.replace('-', "_")
    )
}

fn listen_node_name(junction: &str, listen: &str) -> String {
    format!(
        "{}.listen.{}",
        junction_node_name(junction),
        listen.replace('-', "_")
    )
}

/// Default channel mapping (F4): mono feeds the first two channels, otherwise
/// channel k feeds channel k as far as both sides go.
pub fn default_map(from_channels: usize, to_channels: usize) -> Vec<[u8; 2]> {
    let clamp = |n: usize| u8::try_from(n.min(64)).unwrap_or(64);
    match (from_channels, to_channels) {
        (0, _) | (_, 0) => Vec::new(),
        (1, t) => (1..=clamp(t.min(2))).map(|c| [1, c]).collect(),
        (f, t) => (1..=clamp(f.min(t))).map(|c| [c, c]).collect(),
    }
}

/// Plans the changes that bring `graph` towards `config`.
pub fn plan(config: &Config, graph: &Graph, held: &Held) -> Plan {
    let mut out = Plan::default();
    let existing: BTreeMap<&str, &Node> = graph
        .nodes
        .values()
        .filter(|n| n.is_junc() && n.props.get("junc.role") != Some("meter"))
        .map(|n| (n.name(), n))
        .collect();

    // --- nodes ------------------------------------------------------------
    let mut wanted: Vec<NodeSpec> = Vec::new();
    for j in &config.junction {
        wanted.extend(junction_nodes(j));
    }
    let wanted_names: HashSet<&str> = wanted.iter().map(|s| s.name.as_str()).collect();
    let mut destroy_nodes = Vec::new();
    let mut create_nodes = Vec::new();
    // Nodes that exist with the right shape this round; links only go to them.
    let mut ready: BTreeMap<String, u32> = BTreeMap::new();
    for spec in &wanted {
        match existing.get(spec.name.as_str()) {
            Some(n)
                if channel_count(n) == Some(spec.channels)
                    && n.media_class() == spec.media_class =>
            {
                // node.description is fixed at creation (spike S10), so a
                // rename recreates the node. While apps use it, that would
                // move them to the default device for a moment (a recording
                // app would hear the real mic), so the rename waits.
                if n.description() != spec.description {
                    if is_busy(n.id, graph) {
                        out.deferred.push(spec.name.clone());
                    } else {
                        destroy_nodes.push(Action::DestroyNode { node: n.id });
                        create_nodes.push(Action::CreateNode(spec.clone()));
                        continue;
                    }
                }
                ready.insert(spec.name.clone(), n.id);
            }
            Some(n) => {
                destroy_nodes.push(Action::DestroyNode { node: n.id });
                create_nodes.push(Action::CreateNode(spec.clone()));
            }
            None => create_nodes.push(Action::CreateNode(spec.clone())),
        }
    }
    for (name, n) in &existing {
        if !wanted_names.contains(name) {
            destroy_nodes.push(Action::DestroyNode { node: n.id });
        }
    }

    // --- links ------------------------------------------------------------
    let mut want_links: BTreeMap<(u32, u32), LinkSpec> = BTreeMap::new();
    for j in &config.junction {
        junction_links(j, graph, &ready, held, &mut want_links, &mut out.status);
    }
    let destroyed: HashSet<u32> = destroy_nodes
        .iter()
        .filter_map(|a| {
            if let Action::DestroyNode { node } = a {
                Some(*node)
            } else {
                None
            }
        })
        .collect();
    let mut destroy_links = Vec::new();
    for l in graph
        .links
        .values()
        .filter(|l| l.is_junc() && l.props.get("junc.role") != Some("meter"))
    {
        let gone_with_node =
            destroyed.contains(&l.output_node) || destroyed.contains(&l.input_node);
        if want_links.remove(&(l.output_port, l.input_port)).is_none() && !gone_with_node {
            destroy_links.push(Action::DestroyLink { link: l.id });
        }
    }
    // A foreign link on the same ports already carries the audio; adding a
    // JUNC link next to it would double the signal.
    want_links.retain(|(o, i), _| {
        !graph
            .links
            .values()
            .any(|l| l.output_port == *o && l.input_port == *i)
    });

    out.actions.extend(destroy_links);
    out.actions.extend(destroy_nodes);
    out.actions.extend(create_nodes);
    out.actions
        .extend(want_links.into_values().map(Action::CreateLink));
    out
}

/// An app (or another tool) uses the node: any link touching it that JUNC
/// did not make.
fn is_busy(node: u32, graph: &Graph) -> bool {
    graph
        .links
        .values()
        .any(|l| !l.is_junc() && (l.output_node == node || l.input_node == node))
}

fn channel_count(n: &Node) -> Option<u8> {
    n.props
        .number("audio.channels")
        .and_then(|c| u8::try_from(c).ok())
}

fn junction_nodes(j: &Junction) -> Vec<NodeSpec> {
    let base =
        |name: String, class: &'static str, role: Role, reference: String, description: String| {
            NodeSpec {
                name,
                description,
                media_class: class,
                channels: j.bus.channels,
                junction: j.id.clone(),
                role,
                reference,
            }
        };
    let mut v = vec![
        base(
            junction_node_name(&j.id),
            "Audio/Source/Virtual",
            Role::Junction,
            j.id.clone(),
            j.name.clone(),
        ),
        base(
            sink_node_name(&j.id),
            "Audio/Sink",
            Role::JunctionSink,
            format!("{}/sink", j.id),
            j.name.clone(),
        ),
    ];
    for i in j.input.iter().filter(|i| i.enabled) {
        v.push(base(
            gain_node_name(&j.id, &i.id),
            "Audio/Duplex/Internal",
            Role::InputGain,
            format!("in/{}", i.id),
            format!("JUNC {} · input {}", j.id, i.id),
        ));
    }
    for l in j.listen.iter().filter(|l| l.enabled) {
        v.push(base(
            listen_node_name(&j.id, &l.id),
            "Audio/Duplex/Internal",
            Role::ListenGain,
            format!("listen/{}", l.id),
            format!("JUNC {} · listen {}", j.id, l.id),
        ));
    }
    v
}

#[allow(clippy::too_many_lines)]
fn junction_links(
    j: &Junction,
    graph: &Graph,
    ready: &BTreeMap<String, u32>,
    held: &Held,
    want: &mut BTreeMap<(u32, u32), LinkSpec>,
    status: &mut Vec<EntityStatus>,
) {
    let st = |entity, status, nodes| EntityStatus {
        junction: j.id.clone(),
        entity,
        status,
        nodes,
    };
    let source = ready.get(&junction_node_name(&j.id)).copied();
    let sink = ready.get(&sink_node_name(&j.id)).copied();
    let bus = j.bus.channels;
    let mut add = |out_node: u32, out_ch: u8, in_node: u32, in_ch: u8, reference: String| {
        if held.contains(&(j.id.clone(), reference.clone())) {
            return;
        }
        let (Some(o), Some(i)) = (
            graph.channel_port(out_node, Direction::Out, out_ch),
            graph.channel_port(in_node, Direction::In, in_ch),
        ) else {
            return;
        };
        want.insert(
            (o.id, i.id),
            LinkSpec {
                output_node: out_node,
                output_port: o.id,
                input_node: in_node,
                input_port: i.id,
                junction: j.id.clone(),
                reference,
            },
        );
    };

    status.push(st(
        Entity::Junction,
        if j.enabled {
            Status::Active
        } else {
            Status::Disabled
        },
        source.into_iter().chain(sink).collect(),
    ));
    let (Some(source), Some(sink)) = (source, sink) else {
        // Nodes are being created; links follow on the next round.
        for i in &j.input {
            status.push(st(
                Entity::Input(i.id.clone()),
                if i.enabled {
                    Status::Waiting
                } else {
                    Status::Disabled
                },
                vec![],
            ));
        }
        return;
    };
    // Whatever is played into the Junction sink reaches the Bus.
    for ch in 1..=bus {
        add(sink, ch, source, ch, format!("sink/{ch}"));
    }

    for input in &j.input {
        let entity = Entity::Input(input.id.clone());
        if !j.enabled || !input.enabled {
            status.push(st(entity, Status::Disabled, vec![]));
            continue;
        }
        let matched = resolve(&input.matcher, Use::Input, graph);
        let gain = ready.get(&gain_node_name(&j.id, &input.id)).copied();
        if let Some(gain) = gain {
            for ch in 1..=bus {
                add(gain, ch, source, ch, format!("gain/{}/{ch}", input.id));
            }
            for &src in &matched {
                let n_src = graph.ports_of(src, Direction::Out).count();
                let pairs = input
                    .links
                    .clone()
                    .unwrap_or_else(|| default_map(n_src, usize::from(bus)));
                for [from, to] in pairs {
                    if !input.suppressed.contains(&[from, to]) {
                        add(src, from, gain, to, format!("in/{}/{from}->{to}", input.id));
                    }
                }
            }
        }
        let s = if matched.is_empty() {
            Status::Waiting
        } else {
            Status::Active
        };
        status.push(st(entity, s, matched));
    }

    for listen in &j.listen {
        let entity = Entity::Listen(listen.id.clone());
        if !j.enabled || !listen.enabled {
            status.push(st(entity, Status::Disabled, vec![]));
            continue;
        }
        let output = resolve_output(&listen.target, graph);
        if let (Some(lg), Some(out)) = (
            ready.get(&listen_node_name(&j.id, &listen.id)).copied(),
            output,
        ) {
            for ch in 1..=bus {
                add(source, ch, lg, ch, format!("listen/{}/bus/{ch}", listen.id));
            }
            let n_out = graph.ports_of(out, Direction::In).count();
            let pairs = listen
                .links
                .clone()
                .unwrap_or_else(|| default_map(usize::from(bus), n_out));
            for [from, to] in pairs {
                if !listen.suppressed.contains(&[from, to]) {
                    add(
                        lg,
                        from,
                        out,
                        to,
                        format!("listen/{}/{from}->{to}", listen.id),
                    );
                }
            }
        }
        let s = if output.is_some() {
            Status::Active
        } else {
            Status::Waiting
        };
        status.push(st(entity, s, output.into_iter().collect()));
    }

    if j.kind == Kind::Cable
        && let Some(cable) = &j.cable
    {
        let targets = if j.enabled {
            resolve(&cable.to, Use::CableTarget, graph)
        } else {
            Vec::new()
        };
        for &t in &targets {
            let n_in = graph.ports_of(t, Direction::In).count();
            for [from, to] in default_map(usize::from(bus), n_in) {
                add(source, from, t, to, format!("cable/{from}->{to}"));
            }
        }
        let s = match (j.enabled, targets.is_empty()) {
            (false, _) => Status::Disabled,
            (true, true) => Status::Waiting,
            (true, false) => Status::Active,
        };
        status.push(st(Entity::CableTarget, s, targets));
    }
}
