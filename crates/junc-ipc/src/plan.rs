// SPDX-License-Identifier: GPL-3.0-or-later

//! What applying a config would do, for `Plan`, `Apply` and
//! `junc apply --dry-run`: a plain-language summary of the change (from
//! the difference between the configs), the graph actions of the next
//! reconcile step with node names instead of registry ids, and the
//! resulting statuses.

use crate::view::{self, JunctionView};
use junc_core::backend::{Action, DefaultKind};
use junc_core::graph::Graph;
use junc_core::matcher::{Use, resolve};
use junc_core::model::{Config, DefaultRole, Input, Junction, Kind, Listen, Match};
use junc_core::plan::Plan;
use junc_l10n::{trf, trn};
use serde::{Deserialize, Serialize};

/// One graph action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ActionView {
    /// A node JUNC creates.
    CreateNode {
        node: String,
        media_class: String,
        channels: u8,
        junction: String,
        role: String,
    },
    /// A JUNC node removed.
    DestroyNode {
        node: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        junction: Option<String>,
    },
    /// A link JUNC creates; ends are `node.name:channel`.
    CreateLink {
        from: String,
        to: String,
        junction: String,
        #[serde(rename = "ref")]
        reference: String,
    },
    /// A JUNC link removed.
    DestroyLink {
        from: String,
        to: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        junction: Option<String>,
        #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
        reference: Option<String>,
    },
    /// The system default input or output changes (`None`: WirePlumber picks).
    SetDefault { kind: String, node: Option<String> },
    /// A stream is sent to a node (`None`: back to its default), e.g. the
    /// app a Cable delivers to.
    SetTarget {
        stream: String,
        target: Option<String>,
        junction: String,
        #[serde(rename = "ref")]
        reference: String,
    },
}

/// The plan as clients see it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanView {
    /// What changes, one sentence each.
    pub summary: Vec<String>,
    /// Graph actions of the next step.
    pub actions: Vec<ActionView>,
    /// False when new nodes are created first and their links follow once
    /// they exist (the actions then show only the first step).
    pub complete: bool,
    /// Nodes whose new name waits until no app uses them.
    pub deferred: Vec<String>,
    /// The Junctions with the statuses this plan gives them.
    pub junctions: Vec<JunctionView>,
}

fn node_name(graph: Option<&Graph>, id: u32) -> String {
    graph
        .and_then(|g| g.nodes.get(&id))
        .map_or_else(|| format!("#{id}"), |n| n.name().to_owned())
}

fn end(graph: Option<&Graph>, node: u32, port: u32) -> String {
    let ch = graph
        .and_then(|g| g.ports.get(&port))
        .map_or_else(|| format!("#{port}"), |p| (p.index + 1).to_string());
    format!("{}:{ch}", node_name(graph, node))
}

fn kind_name(k: DefaultKind) -> &'static str {
    match k {
        DefaultKind::Source => "input",
        DefaultKind::Sink => "output",
    }
}

fn action(a: &Action, graph: Option<&Graph>) -> ActionView {
    match a {
        Action::CreateNode(n) => ActionView::CreateNode {
            node: n.name.clone(),
            media_class: n.media_class.to_owned(),
            channels: n.channels,
            junction: n.junction.clone(),
            role: n.role.as_str().to_owned(),
        },
        Action::DestroyNode { node } => ActionView::DestroyNode {
            node: node_name(graph, *node),
            junction: graph
                .and_then(|g| g.nodes.get(node))
                .and_then(|n| n.props.get("junc.junction"))
                .map(str::to_owned),
        },
        Action::CreateLink(l) => ActionView::CreateLink {
            from: end(graph, l.output_node, l.output_port),
            to: end(graph, l.input_node, l.input_port),
            junction: l.junction.clone(),
            reference: l.reference.clone(),
        },
        Action::DestroyLink { link } => {
            let l = graph.and_then(|g| g.links.get(link));
            let tag = |k: &str| l.and_then(|l| l.props.get(k)).map(str::to_owned);
            ActionView::DestroyLink {
                from: l.map_or_else(
                    || format!("#{link}"),
                    |l| end(graph, l.output_node, l.output_port),
                ),
                to: l.map_or_else(
                    || format!("#{link}"),
                    |l| end(graph, l.input_node, l.input_port),
                ),
                junction: tag("junc.junction"),
                reference: tag("junc.ref"),
            }
        }
        Action::SetDefault { kind, name } => ActionView::SetDefault {
            kind: kind_name(*kind).to_owned(),
            node: name.clone(),
        },
        Action::SetTarget {
            node,
            target,
            junction,
            reference,
        } => ActionView::SetTarget {
            stream: node_name(graph, *node),
            target: target.clone(),
            junction: junction.clone(),
            reference: reference.clone(),
        },
    }
}

/// How a person would call what a matcher points at.
fn source(m: &Match, graph: Option<&Graph>) -> String {
    let named = |n: &str| {
        graph
            .and_then(|g| g.node_by_name(n))
            .map_or_else(|| n.to_owned(), |node| node.description().to_owned())
    };
    if let Some(v) = m.app.as_ref().or(m.binary.as_ref()).or(m.role.as_ref()) {
        v.clone()
    } else if let Some(v) = &m.node {
        named(v)
    } else if let Some(v) = &m.device {
        named(v)
    } else if let Some(v) = &m.monitor {
        trf("what {out} plays", &[("out", &named(v))])
    } else if let Some(v) = &m.junction {
        trf("Junction {id}", &[("id", v)])
    } else {
        String::new()
    }
}

fn present(m: &Match, graph: Option<&Graph>) -> bool {
    graph.is_some_and(|g| !resolve(m, Use::Input, g).is_empty())
}

fn target(l: &Listen, graph: Option<&Graph>) -> String {
    if l.target == "default" {
        trf("the default output", &[])
    } else {
        graph
            .and_then(|g| g.node_by_name(&l.target))
            .map_or_else(|| l.target.clone(), |n| n.description().to_owned())
    }
}

fn on_off(on: bool, name: &str) -> String {
    if on {
        trf("Turn “{name}” on", &[("name", &name)])
    } else {
        trf("Turn “{name}” off", &[("name", &name)])
    }
}

fn role(r: DefaultRole, name: &str) -> String {
    match r {
        DefaultRole::None => trf(
            "Stop using “{name}” as a default device",
            &[("name", &name)],
        ),
        DefaultRole::Input => trf("Make “{name}” the default input", &[("name", &name)]),
        DefaultRole::Output => trf("Make “{name}” the default output", &[("name", &name)]),
    }
}

fn add_input(i: &Input, name: &str, graph: Option<&Graph>, out: &mut Vec<String>) {
    let mut src = source(&i.matcher, graph);
    // Settings the user asked for belong in the plan they approve.
    if i.volume != 100 {
        src = trf("{src} at {p}%", &[("src", &src), ("p", &i.volume)]);
    }
    if i.exclusive {
        src = trf(
            "{src}, exclusive (no longer heard on the speakers)",
            &[("src", &src)],
        );
    }
    out.push(if !i.enabled {
        trf(
            "Add {src} to “{name}”, turned off",
            &[("src", &src), ("name", &name)],
        )
    } else if present(&i.matcher, graph) {
        trf("Add {src} to “{name}”", &[("src", &src), ("name", &name)])
    } else {
        trf(
            "Add {src} to “{name}” (routed when it appears)",
            &[("src", &src), ("name", &name)],
        )
    });
}

fn new_junction(j: &Junction, graph: Option<&Graph>, out: &mut Vec<String>) {
    if j.kind == Kind::Cable {
        let from = j
            .input
            .first()
            .map(|i| source(&i.matcher, graph))
            .unwrap_or_default();
        let to = j
            .cable
            .as_ref()
            .map(|c| source(&c.to, graph))
            .unwrap_or_default();
        out.push(trf(
            "Create Cable “{name}” from {from} to {to}",
            &[("name", &j.name), ("from", &from), ("to", &to)],
        ));
        return;
    }
    out.push(trn(
        "Create Junction “{name}” ({n} channel)",
        "Create Junction “{name}” ({n} channels)",
        u64::from(j.bus.channels),
        &[("name", &j.name)],
    ));
    if !j.enabled {
        out.push(on_off(false, &j.name));
    }
    for i in &j.input {
        add_input(i, &j.name, graph, out);
    }
    for l in &j.listen {
        out.push(trf(
            "Play “{name}” on {out}",
            &[("name", &j.name), ("out", &target(l, graph))],
        ));
    }
    if j.default != DefaultRole::None {
        out.push(role(j.default, &j.name));
    }
}

fn changed_junction(old: &Junction, new: &Junction, graph: Option<&Graph>, out: &mut Vec<String>) {
    let name = &new.name;
    if old.name != new.name {
        out.push(trf(
            "Rename “{old}” to “{new}”",
            &[("old", &old.name), ("new", name)],
        ));
    }
    if old.enabled != new.enabled {
        out.push(on_off(new.enabled, name));
    }
    if old.volume != new.volume {
        out.push(trf(
            "Set “{name}” volume to {p}%",
            &[("name", name), ("p", &new.volume)],
        ));
    }
    if old.default != new.default {
        out.push(role(new.default, name));
    }
    if old.bus != new.bus {
        out.push(trn(
            "Give “{name}” {n} Bus channel",
            "Give “{name}” {n} Bus channels",
            u64::from(new.bus.channels),
            &[("name", name)],
        ));
    }
    if old.rate != new.rate || old.cable != new.cable || old.kind != new.kind {
        out.push(trf("Change the settings of “{name}”", &[("name", name)]));
    }
    for i in &new.input {
        match old.input.iter().find(|o| o.id == i.id) {
            None => add_input(i, name, graph, out),
            Some(o) if o == i => {}
            Some(o) => {
                let src = source(&i.matcher, graph);
                if o.enabled != i.enabled {
                    out.push(if i.enabled {
                        trf(
                            "Turn Input {src} of “{name}” on",
                            &[("src", &src), ("name", name)],
                        )
                    } else {
                        trf(
                            "Turn Input {src} of “{name}” off",
                            &[("src", &src), ("name", name)],
                        )
                    });
                }
                if o.volume != i.volume {
                    out.push(trf(
                        "Set Input {src} of “{name}” to {p}%",
                        &[("src", &src), ("name", name), ("p", &i.volume)],
                    ));
                }
                if o.matcher != i.matcher
                    || o.links != i.links
                    || o.exclusive != i.exclusive
                    || o.suppressed != i.suppressed
                {
                    out.push(trf(
                        "Change how {src} is routed into “{name}”",
                        &[("src", &src), ("name", name)],
                    ));
                }
            }
        }
    }
    for o in old
        .input
        .iter()
        .filter(|o| new.input.iter().all(|i| i.id != o.id))
    {
        let src = source(&o.matcher, graph);
        out.push(trf(
            "Remove {src} from “{name}”",
            &[("src", &src), ("name", name)],
        ));
    }
    for l in &new.listen {
        match old.listen.iter().find(|o| o.id == l.id) {
            None => out.push(trf(
                "Play “{name}” on {out}",
                &[("name", name), ("out", &target(l, graph))],
            )),
            Some(o) if o == l => {}
            Some(_) => out.push(trf(
                "Change how “{name}” plays on {out}",
                &[("name", name), ("out", &target(l, graph))],
            )),
        }
    }
    for o in old
        .listen
        .iter()
        .filter(|o| new.listen.iter().all(|l| l.id != o.id))
    {
        out.push(trf(
            "Stop playing “{name}” on {out}",
            &[("name", name), ("out", &target(o, graph))],
        ));
    }
}

/// One sentence per change from `old` to `new`.
pub fn summary(old: &Config, new: &Config, graph: Option<&Graph>) -> Vec<String> {
    let mut out = Vec::new();
    for o in old
        .junction
        .iter()
        .filter(|o| new.junction.iter().all(|j| j.id != o.id))
    {
        out.push(trf("Delete “{name}” and its routing", &[("name", &o.name)]));
    }
    for j in &new.junction {
        match old.junction.iter().find(|o| o.id == j.id) {
            None => new_junction(j, graph, &mut out),
            Some(o) => changed_junction(o, j, graph, &mut out),
        }
    }
    if out.is_empty() {
        out.push(trf("No changes", &[]));
    }
    out
}

/// The plan for going from `old` to `new`, where `p` is the planner's
/// result for `new` against `graph`.
pub fn view(old: &Config, new: &Config, p: &Plan, graph: Option<&Graph>) -> PlanView {
    let mut summary = summary(old, new, graph);
    if old == new && !p.actions.is_empty() {
        summary = vec![trf("Bring the audio graph in line with the config", &[])];
    }
    PlanView {
        summary,
        actions: p.actions.iter().map(|a| action(a, graph)).collect(),
        complete: !p.actions.iter().any(|a| matches!(a, Action::CreateNode(_))),
        deferred: p.deferred.clone(),
        junctions: new
            .junction
            .iter()
            .map(|j| view::junction(j, &p.status, graph))
            .collect(),
    }
}
