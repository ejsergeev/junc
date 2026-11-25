// SPDX-License-Identifier: GPL-3.0-or-later

//! Finds the live nodes a config entry refers to (F2, F10). Config stores
//! identities (app name, binary, device `node.name`…), never node ids, so the
//! same entry keeps working across app restarts, re-plugs and reboots.

use crate::graph::{Graph, Node, NodeKind};
use crate::model::Match;

/// What the matched nodes will be used for; decides which node kinds count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Use {
    /// An Input feeding a Bus: app playback streams, sources, sink monitors,
    /// other Junctions.
    Input,
    /// The app a Cable delivers to: app capture streams.
    CableTarget,
}

/// `node.name` of the Junction source for a Junction id (`virtual-mic` →
/// `junc.virtual_mic`); the sink adds `.sink`.
pub fn junction_node_name(id: &str) -> String {
    format!("junc.{}", id.replace('-', "_"))
}

/// Node ids matching `m`, in id order. App matches may yield several streams
/// (a browser with two tabs playing); device matches yield at most one.
pub fn resolve(m: &Match, usage: Use, graph: &Graph) -> Vec<u32> {
    let pick = |pred: &dyn Fn(&Node) -> bool| -> Vec<u32> {
        graph
            .nodes
            .values()
            .filter(|n| pred(n))
            .map(|n| n.id)
            .collect()
    };
    if m.app.is_some() || m.binary.is_some() || m.role.is_some() || m.node.is_some() {
        let want = match usage {
            Use::Input => NodeKind::AppPlayback,
            Use::CableTarget => NodeKind::AppCapture,
        };
        return pick(&|n| n.kind() == want && n.is_app_stream() && app_matches(m, n));
    }
    if usage != Use::Input {
        return Vec::new();
    }
    if let Some(device) = &m.device {
        return resolve_device(device, m.profile.as_deref(), graph);
    }
    if let Some(sink) = &m.monitor {
        return pick(&|n| n.kind() == NodeKind::Sink && n.name() == sink);
    }
    if let Some(id) = &m.junction {
        let name = junction_node_name(id);
        return pick(&|n| n.is_junc() && n.kind() == NodeKind::Source && n.name() == name);
    }
    Vec::new()
}

/// Output node for a Listen target: `"default"` follows the system output.
pub fn resolve_output(target: &str, graph: &Graph) -> Option<u32> {
    let name = if target == "default" {
        graph.defaults.sink.as_deref()?
    } else {
        target
    };
    graph
        .nodes
        .values()
        .find(|n| n.kind() == NodeKind::Sink && !n.is_junc() && n.name() == name)
        .map(|n| n.id)
}

fn app_matches(m: &Match, n: &Node) -> bool {
    let p = &n.props;
    let eq_ci = |want: &Option<String>, key: &str| {
        want.as_ref()
            .is_none_or(|w| p.get(key).is_some_and(|v| v.eq_ignore_ascii_case(w)))
    };
    let binary_ok = m.binary.as_ref().is_none_or(|want| {
        p.get("application.process.binary")
            .is_some_and(|b| b == want || b.rsplit('/').next() == Some(want.as_str()))
    });
    binary_ok
        && eq_ci(&m.app, "application.name")
        && eq_ci(&m.role, "media.role")
        && eq_ci(&m.node, "node.name")
}

/// Hardware input by stable `node.name`; if no node has that name, a unique
/// source with that description (names can change when a driver changes,
/// descriptions usually do not).
fn resolve_device(device: &str, profile: Option<&str>, graph: &Graph) -> Vec<u32> {
    // With a profile (F15) the Input waits until its card runs that profile,
    // then takes the card's widest capture node: switching profiles renames
    // the nodes (`….analog-stereo` → `….pro-input-0`).
    if let Some(profile) = profile
        && let Some(card) = crate::proaudio::card(graph, device)
    {
        if card.active_profile.as_deref() != Some(profile) {
            return Vec::new();
        }
        if let Some(n) = graph
            .nodes
            .values()
            .find(|n| n.name() == device && n.props.number("device.id") == Some(card.id))
        {
            return vec![n.id];
        }
        return crate::proaudio::card_sources(graph, card)
            .max_by_key(|n| {
                (
                    n.props.number("audio.channels").unwrap_or(0),
                    std::cmp::Reverse(n.name()),
                )
            })
            .map(|n| vec![n.id])
            .unwrap_or_default();
    }
    let sources = || {
        graph
            .nodes
            .values()
            .filter(|n| n.kind() == NodeKind::Source && !n.is_junc())
    };
    if let Some(n) = sources().find(|n| n.name() == device) {
        return vec![n.id];
    }
    let by_desc: Vec<u32> = sources()
        .filter(|n| n.description() == device)
        .map(|n| n.id)
        .collect();
    if by_desc.len() == 1 {
        by_desc
    } else {
        Vec::new()
    }
}
