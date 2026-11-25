// SPDX-License-Identifier: GPL-3.0-or-later

//! Junctions as system defaults (F16, docs/architecture.md §3.7).
//!
//! "Use as default input/output" writes the Junction's node into the
//! `default.configured.audio.*` metadata, like `wpctl set-default`. The
//! previous value is saved first and restored when the Junction stops being
//! the default (turned off, deleted, or `default` cleared). If that device is
//! gone by then, the key is cleared and WirePlumber picks.
//!
//! JUNC never fights the user: if the default is changed elsewhere while a
//! Junction holds it, JUNC does not take it back, reports
//! [`DefaultNotice::TakenOver`], and treats the user's choice as the new
//! baseline.

use crate::backend::{Action, DefaultKind};
use crate::graph::Graph;
use crate::matcher::junction_node_name;
use crate::model::{Config, DefaultRole};
use crate::plan::sink_node_name;
use serde::{Deserialize, Serialize};

/// What JUNC remembers per default kind (persisted in the daemon's state
/// file, not the config: it is not user intent).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    /// The configured default before JUNC changed it; `None` inside means
    /// there was no configured default (WirePlumber chose).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<Option<String>>,
    /// The node JUNC last made default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied: Option<String>,
    /// JUNC asked for `applied` and has not seen it in a snapshot yet.
    #[serde(skip)]
    pub pending: bool,
    /// The user chose another default while this Junction node held it; JUNC
    /// does not take it back while the config still asks for the same node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yielded: Option<String>,
    /// JUNC asked to put back the previous default and has not seen it land
    /// (the `default` metadata lives in WirePlumber, so snapshots lag).
    #[serde(skip)]
    pub restoring: bool,
}

/// Persistent defaults state for both kinds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultsState {
    /// Default input.
    #[serde(default)]
    pub source: Saved,
    /// Default output.
    #[serde(default)]
    pub sink: Saved,
}

/// Something to tell the user about defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultNotice {
    /// JUNC put back the previous default (toast with Undo in the GUI).
    Restored {
        /// Input or output.
        kind: DefaultKind,
        /// The node made default again, or `None` if it was gone and the key
        /// was cleared.
        to: Option<String>,
    },
    /// The user picked another default elsewhere while a Junction held it.
    TakenOver {
        /// Input or output.
        kind: DefaultKind,
        /// Junction id that was the default.
        junction: String,
    },
}

/// Plans default changes for one snapshot; updates `state` as if the actions
/// succeed (the next snapshot confirms).
pub fn plan_defaults(
    config: &Config,
    graph: &Graph,
    state: &mut DefaultsState,
) -> (Vec<Action>, Vec<DefaultNotice>) {
    let mut actions = Vec::new();
    let mut notices = Vec::new();
    for kind in [DefaultKind::Source, DefaultKind::Sink] {
        let (role, saved, current) = match kind {
            DefaultKind::Source => (
                DefaultRole::Input,
                &mut state.source,
                &graph.defaults.configured_source,
            ),
            DefaultKind::Sink => (
                DefaultRole::Output,
                &mut state.sink,
                &graph.defaults.configured_sink,
            ),
        };
        let node_of = |id: &str| match kind {
            DefaultKind::Source => junction_node_name(id),
            DefaultKind::Sink => sink_node_name(id),
        };
        let owner = config
            .junction
            .iter()
            .find(|j| j.enabled && j.default == role);
        let want = owner
            .map(|j| node_of(&j.id))
            .filter(|n| graph.nodes.values().any(|x| x.name() == n && x.is_junc()));
        let is_ours = |name: &Option<String>| name.as_ref().is_some_and(|n| n.starts_with("junc."));

        if saved.pending && current == &saved.applied {
            saved.pending = false;
        }
        if saved.restoring {
            if is_ours(current) {
                continue; // the restore has not landed yet
            }
            saved.restoring = false;
        }
        // Someone else moved the default away from what JUNC set.
        if !saved.pending
            && let Some(applied) = &saved.applied
            && current.as_ref() != Some(applied)
            && !is_ours(current)
        {
            if let Some(j) = config.junction.iter().find(|j| &node_of(&j.id) == applied) {
                notices.push(DefaultNotice::TakenOver {
                    kind,
                    junction: j.id.clone(),
                });
            }
            let yielded = saved.applied.take();
            *saved = Saved {
                yielded,
                ..Saved::default()
            };
            continue;
        }
        // A yield lasts until the config asks for something else.
        if saved.yielded.is_some() {
            if want.is_some() && want == saved.yielded {
                continue;
            }
            if owner.map(|j| node_of(&j.id)) != saved.yielded {
                saved.yielded = None;
            } else {
                continue;
            }
        }

        match want {
            Some(w) if current.as_ref() != Some(&w) => {
                if saved.applied.as_deref() == Some(w.as_str()) {
                    continue; // already asked; waiting for the snapshot
                }
                if saved.previous.is_none() && !is_ours(current) {
                    saved.previous = Some(current.clone());
                }
                saved.applied = Some(w.clone());
                saved.pending = true;
                actions.push(Action::SetDefault {
                    kind,
                    name: Some(w),
                });
            }
            Some(w) => saved.applied = Some(w),
            None if is_ours(current) || saved.applied.is_some() => {
                // No Junction should be the default any more: put back the
                // previous one if it is still there.
                let previous = saved.previous.clone().flatten();
                let to = previous.filter(|p| graph.nodes.values().any(|x| x.name() == p));
                *saved = Saved::default();
                if is_ours(current) {
                    actions.push(Action::SetDefault {
                        kind,
                        name: to.clone(),
                    });
                    notices.push(DefaultNotice::Restored { kind, to });
                    saved.restoring = true;
                }
            }
            None => {}
        }
    }
    (actions, notices)
}
