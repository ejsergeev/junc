// SPDX-License-Identifier: GPL-3.0-or-later

//! Import of existing setups (F13): null sinks (from `pactl load-module
//! module-null-sink` or a config fragment) and `pw-loopback`s found in the
//! running graph, with what they connect, so each can become a Junction or
//! a Cable. JUNC never removes the old setup; the user does when happy.

use crate::graph::{Graph, Node, NodeKind};
use crate::model::Match;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// One setup that can be imported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Importable {
    /// Stable id for `junc import ID`: the null sink's `node.name`, or the
    /// loopback's name.
    pub id: String,
    /// `null_sink` or `loopback`.
    pub kind: String,
    /// How it was made: `pactl load-module`, `config`, `pw-loopback` or
    /// `loopback module`.
    pub origin: String,
    /// Name to give the Junction or Cable.
    pub name: String,
    /// Channel count.
    pub channels: u8,
    /// What feeds it now, as Input matches (apps, devices, monitors).
    pub inputs: Vec<Match>,
    /// Outputs it plays to now, as Listen targets.
    pub listens: Vec<String>,
    /// Apps that record from it now.
    pub recorded_by: Vec<Match>,
    /// Set when it is one app into another app: import as a Cable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cable: Option<CableProposal>,
    /// `cable` when it can be a Cable, else `junction`.
    pub suggested: String,
}

/// One app into one app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CableProposal {
    /// The app heard.
    pub from: Match,
    /// The app that records it.
    pub to: Match,
}

fn app_match(n: &Node) -> Match {
    let binary = n.props.get("application.process.binary");
    Match {
        binary: binary.map(str::to_owned),
        app: binary
            .is_none()
            .then(|| n.props.get("application.name").map(str::to_owned))
            .flatten(),
        ..Match::default()
    }
}

/// Nodes linked into `node`.
fn feeders(g: &Graph, node: u32) -> impl Iterator<Item = &Node> {
    let ids: BTreeSet<u32> = g
        .links
        .values()
        .filter(|l| l.input_node == node)
        .map(|l| l.output_node)
        .collect();
    ids.into_iter().filter_map(|id| g.nodes.get(&id))
}

/// Nodes `node` is linked into.
fn consumers(g: &Graph, node: u32) -> impl Iterator<Item = &Node> {
    let ids: BTreeSet<u32> = g
        .links
        .values()
        .filter(|l| l.output_node == node)
        .map(|l| l.input_node)
        .collect();
    ids.into_iter().filter_map(|id| g.nodes.get(&id))
}

/// The Input match for something feeding a setup, if JUNC can take it.
fn input_match(n: &Node) -> Option<Match> {
    if n.is_junc() || n.is_plumbing() {
        return None;
    }
    match n.kind() {
        NodeKind::AppPlayback if n.is_app_stream() => Some(app_match(n)),
        NodeKind::Source => Some(Match {
            device: Some(n.name().to_owned()),
            ..Match::default()
        }),
        NodeKind::Sink => Some(Match {
            monitor: Some(n.name().to_owned()),
            ..Match::default()
        }),
        _ => None,
    }
}

fn push<T: PartialEq>(v: &mut Vec<T>, x: T) {
    if !v.contains(&x) {
        v.push(x);
    }
}

fn finish(mut i: Importable) -> Importable {
    let apps: Vec<&Match> = i
        .inputs
        .iter()
        .filter(|m| m.binary.is_some() || m.app.is_some())
        .collect();
    if apps.len() == 1 && i.inputs.len() == 1 && i.recorded_by.len() == 1 && i.listens.is_empty() {
        i.cable = Some(CableProposal {
            from: apps[0].clone(),
            to: i.recorded_by[0].clone(),
        });
    }
    i.suggested = if i.cable.is_some() {
        "cable"
    } else {
        "junction"
    }
    .to_owned();
    i
}

fn channels(n: &Node) -> u8 {
    n.props
        .number("audio.channels")
        .and_then(|c| u8::try_from(c).ok())
        .filter(|c| *c > 0)
        .unwrap_or(2)
}

/// A null sink made by the user: not JUNC's, not part of a device.
fn null_sink(g: &Graph, n: &Node) -> Option<Importable> {
    if n.props.get("factory.name") != Some("support.null-audio-sink")
        || n.is_junc()
        || !matches!(n.kind(), NodeKind::Sink | NodeKind::Source)
    {
        return None;
    }
    let origin = if n.props.get("pulse.module.id").is_some() {
        "pactl load-module"
    } else {
        "config"
    };
    let mut i = Importable {
        id: n.name().to_owned(),
        kind: "null_sink".into(),
        origin: origin.into(),
        // pipewire-pulse describes an undescribed sink as
        // "<name> Audio/Sink sink"; the name reads better.
        name: if n.description().starts_with(n.name()) {
            n.name().to_owned()
        } else {
            non_empty(n.description(), n.name())
        },
        channels: channels(n),
        inputs: Vec::new(),
        listens: Vec::new(),
        recorded_by: Vec::new(),
        cable: None,
        suggested: String::new(),
    };
    for f in feeders(g, n.id) {
        if let Some(m) = input_match(f) {
            push(&mut i.inputs, m);
        }
    }
    collect_outputs(g, n.id, &mut i);
    Some(finish(i))
}

fn collect_outputs(g: &Graph, node: u32, i: &mut Importable) {
    for c in consumers(g, node) {
        if c.is_junc() || c.is_plumbing() {
            continue;
        }
        match c.kind() {
            NodeKind::Sink => push(&mut i.listens, c.name().to_owned()),
            NodeKind::AppCapture if c.is_app_stream() => push(&mut i.recorded_by, app_match(c)),
            _ => {}
        }
    }
}

fn non_empty(a: &str, b: &str) -> String {
    if a.trim().is_empty() { b } else { a }.to_owned()
}

/// A loopback whose two sides are streams (what `pw-loopback` makes by
/// default): capture from something, play into something. Loopbacks that
/// present a virtual device (Omarchy's mic) and Bluetooth's own are not
/// routing setups.
fn loopback(g: &Graph, capture: &Node) -> Option<Importable> {
    let group = capture.props.get("node.link-group")?;
    if !group.starts_with("loopback-")
        || capture.kind() != NodeKind::AppCapture
        || capture.props.flag("bluez5.loopback")
    {
        return None;
    }
    let playback = g.nodes.values().find(|n| {
        n.id != capture.id
            && n.props.get("node.link-group") == Some(group)
            && n.kind() == NodeKind::AppPlayback
    })?;
    let origin = if capture
        .props
        .get("node.group")
        .is_some_and(|s| s.starts_with("pw-loopback"))
    {
        "pw-loopback"
    } else {
        "loopback module"
    };
    let name = non_empty(capture.description(), group);
    let mut i = Importable {
        id: capture
            .name()
            .strip_prefix("input.")
            .unwrap_or(capture.name())
            .to_owned(),
        kind: "loopback".into(),
        origin: origin.into(),
        name,
        channels: channels(playback),
        inputs: Vec::new(),
        listens: Vec::new(),
        recorded_by: Vec::new(),
        cable: None,
        suggested: String::new(),
    };
    for f in feeders(g, capture.id) {
        if let Some(m) = input_match(f) {
            push(&mut i.inputs, m);
        }
    }
    // Not linked (yet): what it is set to capture from.
    if i.inputs.is_empty()
        && let Some(t) = capture
            .props
            .get("target.object")
            .or_else(|| capture.props.get("node.target"))
        && let Some(n) = g.nodes.values().find(|n| n.name() == t)
        && let Some(m) = input_match(n)
    {
        push(&mut i.inputs, m);
    }
    collect_outputs(g, playback.id, &mut i);
    if i.listens.is_empty()
        && i.recorded_by.is_empty()
        && let Some(t) = playback.props.get("target.object")
        && g.nodes
            .values()
            .any(|n| n.name() == t && n.kind() == NodeKind::Sink && !n.is_junc())
    {
        push(&mut i.listens, t.to_owned());
    }
    Some(finish(i))
}

/// Everything in the graph that can be imported, sorted by id.
pub fn scan(graph: &Graph) -> Vec<Importable> {
    let mut out: Vec<Importable> = graph
        .nodes
        .values()
        .filter_map(|n| null_sink(graph, n).or_else(|| loopback(graph, n)))
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out.dedup_by(|a, b| a.id == b.id);
    out
}
