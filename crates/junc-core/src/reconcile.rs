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

use crate::backend::Action;
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

/// Reconcile state for one daemon.
#[derive(Debug, Default)]
pub struct Reconciler {
    /// JUNC links in the last snapshot, by link id.
    last: HashMap<u32, Seen>,
    /// Links we asked to destroy and have not seen go yet.
    destroying: HashSet<u32>,
    /// Holds awaiting confirmation on the next snapshot.
    tentative: HashMap<(String, String), (u32, u32)>,
    /// Confirmed holds (notice raised, waiting for the user).
    confirmed: Held,
}

impl Reconciler {
    /// A reconciler with nothing held.
    pub fn new() -> Self {
        Self::default()
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

        // Holds for Junctions or entries no longer in the config are moot.
        let known: HashSet<&str> = config.junction.iter().map(|j| j.id.as_str()).collect();
        self.confirmed.retain(|(j, _)| known.contains(j.as_str()));

        let p = plan(config, graph, &self.held());
        for a in &p.actions {
            if let Action::DestroyLink { link } = a {
                self.destroying.insert(*link);
            }
        }
        Step {
            actions: p.actions.clone(),
            notices,
            plan: p,
        }
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
