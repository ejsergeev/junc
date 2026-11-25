// SPDX-License-Identifier: GPL-3.0-or-later

//! Reconcile loop state (docs/architecture.md §3.4–3.5): runs the planner on
//! every graph snapshot and keeps what the planner alone cannot know: which
//! JUNC links another tool removed (F17), so JUNC does not fight it.
//!
//! A JUNC link that disappears while both its nodes are still there, and that
//! JUNC did not destroy itself, is *held* at once (not recreated). If the next
//! snapshot still has both nodes, it was another tool: a
//! [`Notice::LinkOverridden`] is raised and the link stays held until the user
//! chooses Restore or Leave as is. If a node went away in the meantime (an app
//! quitting removes its links first), the hold is dropped silently and the
//! link waits for the app like any other.
//!
//! Links between JUNC's own nodes (`gain/…`, `sink/…`, `listen/…/bus/…`) are
//! always restored: they are internal wiring, not routing choices.

use crate::backend::{Action, DefaultKind};
use crate::defaults::{DefaultNotice, DefaultsState, plan_defaults};
use crate::graph::Graph;
use crate::model::Config;
use crate::plan::{Held, Plan, plan};
use std::collections::{HashMap, HashSet};

/// Something the user should know about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// Another tool removed a JUNC link (F17). Offer Restore / Leave as is.
    LinkOverridden {
        /// Junction id.
        junction: String,
        /// `junc.ref` of the link, e.g. `in/mic/1->2`.
        reference: String,
    },
    /// JUNC put back the previous default device (F16).
    DefaultRestored {
        /// Input or output.
        kind: DefaultKind,
        /// The device made default again; `None` if it was gone.
        to: Option<String>,
    },
    /// The user chose another default elsewhere while a Junction held it.
    DefaultTakenOver {
        /// Input or output.
        kind: DefaultKind,
        /// The Junction that was the default.
        junction: String,
    },
}

/// How the user answered a [`Notice::LinkOverridden`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Recreate the link.
    Restore,
    /// Keep it removed, also after restarts (adds it to `suppressed`).
    LeaveAsIs,
}

/// One reconcile step.
#[derive(Debug, Clone, Default)]
pub struct Step {
    /// Actions to apply.
    pub actions: Vec<Action>,
    /// New notices.
    pub notices: Vec<Notice>,
    /// The plan the actions came from (statuses, deferred renames).
    pub plan: Plan,
    /// Ask the backend for another snapshot even if nothing changes: a
    /// tentative hold is confirmed on the next snapshot, and an idle graph
    /// would never send one.
    pub refresh: bool,
}

/// A JUNC link as seen in a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Seen {
    /// `object.serial`: registry ids are reused, serials are not.
    serial: Option<u32>,
    junction: String,
    reference: String,
    output_node: u32,
    input_node: u32,
}

/// A stream JUNC sent somewhere with `target.object`.
#[derive(Debug, Clone)]
struct Targeted {
    junction: String,
    reference: String,
    value: String,
    /// The graph has shown the value since JUNC set it.
    confirmed: bool,
}

/// Reconcile state for one daemon.
#[derive(Debug, Default)]
pub struct Reconciler {
    /// Gain JUNC set on its nodes, by `object.serial` (ids are reused).
    volumes: HashMap<u32, u16>,
    /// Streams whose `target.object` JUNC set, by stream node id.
    targeted: HashMap<u32, Targeted>,
    /// JUNC links in the last snapshot, by link id.
    last: HashMap<u32, Seen>,
    /// Links we asked to destroy and have not seen go yet.
    destroying: HashSet<u32>,
    /// Holds awaiting confirmation on the next snapshot.
    tentative: HashMap<(String, String), (u32, u32)>,
    /// Confirmed holds (notice raised, waiting for the user).
    confirmed: Held,
    /// Saved previous defaults (persisted by the daemon).
    defaults: DefaultsState,
}

impl Reconciler {
    /// A reconciler with nothing held.
    pub fn new() -> Self {
        Self::default()
    }

    /// A reconciler that continues from saved defaults state.
    pub fn with_defaults(defaults: DefaultsState) -> Self {
        Self {
            defaults,
            ..Self::default()
        }
    }

    /// Defaults state to persist after each step.
    pub fn defaults(&self) -> &DefaultsState {
        &self.defaults
    }

    /// Links currently not recreated (tentative and confirmed).
    pub fn held(&self) -> Held {
        let mut h = self.confirmed.clone();
        h.extend(self.tentative.keys().cloned());
        h
    }

    /// Overridden links waiting for the user.
    pub fn overridden(&self) -> &Held {
        &self.confirmed
    }

    /// Processes a graph snapshot and returns what to do.
    pub fn on_graph(&mut self, config: &Config, graph: &Graph) -> Step {
        let mut notices = Vec::new();

        // Confirm or drop last round's tentative holds.
        let tentative = std::mem::take(&mut self.tentative);
        for (key, (out, inn)) in tentative {
            if graph.nodes.contains_key(&out) && graph.nodes.contains_key(&inn) {
                notices.push(Notice::LinkOverridden {
                    junction: key.0.clone(),
                    reference: key.1.clone(),
                });
                self.confirmed.insert(key);
            }
        }

        // Find JUNC links that vanished since the last snapshot.
        let current: HashMap<u32, Seen> = graph
            .links
            .values()
            .filter(|l| l.is_junc())
            .filter_map(|l| {
                Some((
                    l.id,
                    Seen {
                        serial: l.props.number("object.serial"),
                        junction: l.props.get("junc.junction")?.to_owned(),
                        reference: l.props.get("junc.ref")?.to_owned(),
                        output_node: l.output_node,
                        input_node: l.input_node,
                    },
                ))
            })
            .collect();
        for (id, seen) in &self.last {
            if current.get(id).is_some_and(|c| c.serial == seen.serial) {
                continue;
            }
            if self.destroying.remove(id) || is_internal(&seen.reference) {
                continue;
            }
            let both_present = graph.nodes.contains_key(&seen.output_node)
                && graph.nodes.contains_key(&seen.input_node);
            let key = (seen.junction.clone(), seen.reference.clone());
            if both_present && !self.confirmed.contains(&key) {
                self.tentative
                    .insert(key, (seen.output_node, seen.input_node));
            }
        }
        self.destroying.retain(|id| current.contains_key(id));
        self.last = current;

        // Streams JUNC sent to a Cable: a value someone else wrote since is
        // the user's choice (F17); hold it like an overridden link.
        let holds = &mut self.confirmed;
        self.targeted.retain(|stream, t| {
            if !graph.nodes.contains_key(stream) {
                return false;
            }
            if graph.targets.get(stream) == Some(&t.value) {
                t.confirmed = true;
                return true;
            }
            if !t.confirmed {
                return true; // our write has not shown up yet
            }
            let key = (t.junction.clone(), t.reference.clone());
            if holds.insert(key) {
                notices.push(Notice::LinkOverridden {
                    junction: t.junction.clone(),
                    reference: t.reference.clone(),
                });
            }
            false
        });

        // Holds for Junctions or entries no longer in the config are moot,
        // and so are holds for streams that are gone.
        let known: HashSet<&str> = config.junction.iter().map(|j| j.id.as_str()).collect();
        let serials: HashSet<String> = graph
            .nodes
            .values()
            .map(|n| crate::plan::cable_target_ref(graph, n.id))
            .collect();
        self.confirmed.retain(|(j, r)| {
            known.contains(j.as_str()) && (!r.starts_with("cable/target/") || serials.contains(r))
        });

        let p = plan(config, graph, &self.held());
        for a in &p.actions {
            if let Action::DestroyLink { link } = a {
                self.destroying.insert(*link);
            }
        }
        let mut actions = p.actions.clone();
        actions.extend(self.track_targets(config, graph, &p.actions));
        actions.extend(self.track_volumes(config, graph));
        let (default_actions, default_notices) = plan_defaults(config, graph, &mut self.defaults);
        actions.extend(default_actions);
        notices.extend(default_notices.into_iter().map(|n| match n {
            DefaultNotice::Restored { kind, to } => Notice::DefaultRestored { kind, to },
            DefaultNotice::TakenOver { kind, junction } => {
                Notice::DefaultTakenOver { kind, junction }
            }
        }));
        Step {
            actions,
            notices,
            plan: p,
            refresh: !self.tentative.is_empty(),
        }
    }

    /// Records the streams this step sends to Cables, and returns the
    /// streams JUNC sent earlier that no Cable wants now (Cable deleted or
    /// off, app no longer matched) to their default. Values someone else
    /// wrote are never touched.
    fn track_targets(&mut self, config: &Config, graph: &Graph, planned: &[Action]) -> Vec<Action> {
        for a in planned {
            if let Action::SetTarget {
                node,
                target: Some(value),
                junction,
                reference,
            } = a
            {
                self.targeted.insert(
                    *node,
                    Targeted {
                        junction: junction.clone(),
                        reference: reference.clone(),
                        value: value.clone(),
                        confirmed: false,
                    },
                );
            }
        }
        let wanted: HashSet<(u32, String)> = config
            .junction
            .iter()
            .filter(|j| j.enabled && j.kind == crate::model::Kind::Cable)
            .filter_map(|j| Some((j, j.cable.as_ref()?)))
            .flat_map(|(j, c)| {
                let name = crate::matcher::junction_node_name(&j.id);
                crate::matcher::resolve(&c.to, crate::matcher::Use::CableTarget, graph)
                    .into_iter()
                    .map(move |s| (s, name.clone()))
            })
            .collect();
        let mut out = Vec::new();
        self.targeted.retain(|stream, t| {
            if wanted.contains(&(*stream, t.value.clone())) {
                return true;
            }
            if graph.targets.get(stream) == Some(&t.value) {
                out.push(Action::SetTarget {
                    node: *stream,
                    target: None,
                    junction: t.junction.clone(),
                    reference: t.reference.clone(),
                });
            }
            false
        });
        out
    }

    /// Volumes (F7): the Junction's master on its source node, each Input's
    /// and Listen output's on its gain node. A node gets its volume when JUNC
    /// first sees it (also 100 %: a node left by an earlier daemon may hold
    /// another value) and whenever the config changes it. JUNC does not watch
    /// for changes made elsewhere (a mixer moving the Junction's source), so
    /// it never fights them.
    fn track_volumes(&mut self, config: &Config, graph: &Graph) -> Vec<Action> {
        let mut want: Vec<(String, u16)> = Vec::new();
        for j in &config.junction {
            want.push((crate::matcher::junction_node_name(&j.id), j.volume));
            for i in &j.input {
                want.push((crate::plan::gain_node_name(&j.id, &i.id), i.volume));
            }
            for l in &j.listen {
                want.push((crate::plan::listen_node_name(&j.id, &l.id), l.volume));
            }
        }
        let key = |n: &crate::graph::Node| n.props.number("object.serial").unwrap_or(n.id);
        let mut actions = Vec::new();
        for (name, percent) in want {
            let Some(node) = graph
                .nodes
                .values()
                .find(|n| n.name() == name && n.is_junc())
            else {
                continue;
            };
            if self.volumes.get(&key(node)) == Some(&percent) {
                continue;
            }
            let channels = node
                .props
                .number("audio.channels")
                .and_then(|c| u8::try_from(c).ok())
                .unwrap_or_else(|| {
                    u8::try_from(
                        graph
                            .ports_of(node.id, crate::graph::Direction::Out)
                            .count(),
                    )
                    .unwrap_or(2)
                });
            self.volumes.insert(key(node), percent);
            actions.push(Action::SetVolume {
                node: node.id,
                channels,
                percent,
            });
        }
        let present: HashSet<u32> = graph
            .nodes
            .values()
            .filter(|n| n.is_junc())
            .map(key)
            .collect();
        self.volumes.retain(|k, _| present.contains(k));
        actions
    }

    /// Whether a link or stream target is held, waiting for the user.
    pub fn is_held(&self, junction: &str, reference: &str) -> bool {
        self.confirmed
            .contains(&(junction.to_owned(), reference.to_owned()))
    }

    /// Applies the user's answer. For [`Resolution::LeaveAsIs`] returns the
    /// config with the link added to `suppressed` (the caller saves it).
    pub fn resolve(
        &mut self,
        config: &Config,
        junction: &str,
        reference: &str,
        how: Resolution,
    ) -> Option<Config> {
        let key = (junction.to_owned(), reference.to_owned());
        let was_held = self.confirmed.remove(&key) | self.tentative.remove(&key).is_some();
        if !was_held || how == Resolution::Restore {
            return None;
        }
        if reference.starts_with("cable/target/") {
            // A stream lives as long as the app runs; the config has no
            // place for it. Keep holding it until it goes.
            self.confirmed.insert(key);
            return None;
        }
        suppress(config, junction, reference)
    }
}

/// Internal wiring between JUNC's own nodes.
fn is_internal(reference: &str) -> bool {
    reference.starts_with("gain/") || reference.starts_with("sink/") || reference.contains("/bus/")
}

/// The config with the link named by `reference` added to its entry's
/// `suppressed` list, or `None` if the reference names no such entry.
pub fn suppress(config: &Config, junction: &str, reference: &str) -> Option<Config> {
    let (kind, rest) = reference.split_once('/')?;
    let (entry, pair) = rest.split_once('/')?;
    let (from, to) = pair.split_once("->")?;
    let pair = [from.parse().ok()?, to.parse().ok()?];
    let mut out = config.clone();
    let j = out.junction.iter_mut().find(|j| j.id == junction)?;
    let list = match kind {
        "in" => &mut j.input.iter_mut().find(|i| i.id == entry)?.suppressed,
        "listen" => &mut j.listen.iter_mut().find(|l| l.id == entry)?.suppressed,
        _ => return None,
    };
    if !list.contains(&pair) {
        list.push(pair);
    }
    Some(out)
}
