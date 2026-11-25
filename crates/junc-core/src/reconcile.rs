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
//! link waits for the app like any other. Nodes are compared by
//! `object.serial`: an app that restarts between two snapshots can get its old
//! registry id back, and its new stream must be linked, not held (F10).
//!
//! Links between JUNC's own nodes (`gain/…`, `sink/…`, `listen/…/bus/…`) are
//! always restored: they are internal wiring, not routing choices.

use crate::backend::{Action, DefaultKind, Role};
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
    /// The user switched a card JUNC keeps in a profile (F15) to another
    /// one elsewhere. JUNC keeps their choice: the Input's `profile` now
    /// names it (`None`: the card was turned off; the config is left as is
    /// and JUNC asks again when the card comes back).
    ProfileChanged {
        /// Junction id.
        junction: String,
        /// Input id.
        input: String,
        /// The profile the user picked, as saved in the config.
        profile: Option<String>,
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
    /// A change JUNC makes to the config itself (the user picked another
    /// device profile elsewhere); the daemon saves it.
    pub config: Option<Config>,
}

/// A card JUNC keeps in a profile, by `object.serial` (a re-plug is a new
/// card).
#[derive(Debug, Clone)]
struct CardProfile {
    /// The profile the config wants.
    want: String,
    /// The card has shown it since JUNC asked (or already had it).
    confirmed: bool,
    /// The user switched away while the card stayed: leave it alone.
    user: bool,
}

/// A JUNC link as seen in a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Seen {
    /// `object.serial`: registry ids are reused, serials are not.
    serial: Option<u32>,
    junction: String,
    reference: String,
    /// The linked nodes, by [`node_key`].
    output_node: NodeKey,
    input_node: NodeKey,
}

/// A node's id with its `object.serial`: the same id with another serial is
/// another node.
type NodeKey = (u32, Option<u32>);

fn node_key(graph: &Graph, id: u32) -> NodeKey {
    (
        id,
        graph
            .nodes
            .get(&id)
            .and_then(|n| n.props.number("object.serial")),
    )
}

/// The node `key` names is still in the graph.
fn present(graph: &Graph, key: NodeKey) -> bool {
    graph.nodes.contains_key(&key.0) && node_key(graph, key.0) == key
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
    /// Cards JUNC keeps in a profile, by `object.serial`.
    cards: HashMap<u32, CardProfile>,
    /// Streams whose `target.object` JUNC set, by stream node id (the
    /// reference carries the stream's serial).
    targeted: HashMap<u32, Targeted>,
    /// JUNC links in the last snapshot, by link id.
    last: HashMap<u32, Seen>,
    /// Links we asked to destroy and have not seen go yet.
    destroying: HashSet<u32>,
    /// Holds awaiting confirmation on the next snapshot.
    tentative: HashMap<(String, String), (NodeKey, NodeKey)>,
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
            if present(graph, out) && present(graph, inn) {
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
                        output_node: node_key(graph, l.output_node),
                        input_node: node_key(graph, l.input_node),
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
            let both_present = present(graph, seen.output_node) && present(graph, seen.input_node);
            let key = (seen.junction.clone(), seen.reference.clone());
            if both_present && !self.confirmed.contains(&key) {
                self.tentative
                    .insert(key, (seen.output_node, seen.input_node));
            }
        }
        self.destroying.retain(|id| current.contains_key(id));
        self.last = current;

        // Streams JUNC sent to a Cable or an Exclusive gain node: a value
        // someone else wrote since is the user's choice (F17); hold it like
        // an overridden link.
        let holds = &mut self.confirmed;
        self.targeted.retain(|stream, t| {
            // Gone, or its id now belongs to the app's next stream.
            let serial = graph
                .nodes
                .get(stream)
                .map(|n| n.props.number("object.serial").unwrap_or(n.id).to_string());
            if serial.is_none()
                || serial.as_deref() != crate::plan::stream_target_serial(&t.reference)
            {
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
            .map(|n| n.props.number("object.serial").unwrap_or(n.id).to_string())
            .collect();
        self.confirmed.retain(|(j, r)| {
            known.contains(j.as_str())
                && crate::plan::stream_target_serial(r).is_none_or(|s| serials.contains(s))
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
        let (profile_actions, new_config) = self.track_profiles(config, graph, &mut notices);
        actions.extend(profile_actions);
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
            config: new_config,
        }
    }

    /// Records the streams this step sends to Cables and Exclusive gain
    /// nodes, and returns the streams JUNC sent earlier that nothing wants
    /// now (Cable deleted, Exclusive turned off, Junction or Input off, app
    /// no longer matched) to their default. Values someone else wrote are
    /// never touched.
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
        let wanted_targets = crate::plan::stream_targets(config, graph);
        // Streams already where JUNC wants them, and streams sent to a JUNC
        // Input gain node (only JUNC does that: gain nodes are not listed as
        // devices), are JUNC's to return: a daemon that restarted does not
        // remember having moved them.
        for t in &wanted_targets {
            if graph.targets.get(&t.stream) == Some(&t.target) {
                self.targeted.entry(t.stream).or_insert_with(|| Targeted {
                    junction: t.junction.clone(),
                    reference: t.reference.clone(),
                    value: t.target.clone(),
                    confirmed: true,
                });
            }
        }
        for (&stream, value) in &graph.targets {
            let Some(gain) = graph.node_by_name(value).filter(|n| {
                n.is_junc() && n.props.get("junc.role") == Some(Role::InputGain.as_str())
            }) else {
                continue;
            };
            if graph.nodes.contains_key(&stream) {
                self.targeted.entry(stream).or_insert_with(|| Targeted {
                    junction: gain
                        .props
                        .get("junc.junction")
                        .unwrap_or_default()
                        .to_owned(),
                    reference: crate::plan::stream_target_ref("exclusive", graph, stream),
                    value: value.clone(),
                    confirmed: true,
                });
            }
        }
        let wanted: HashSet<(u32, String)> = wanted_targets
            .into_iter()
            .map(|t| (t.stream, t.target))
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

    /// Device profiles (F15): a device Input with `profile` switches its
    /// card to it when the card appears (and when the config changes it).
    /// If the user then picks another profile elsewhere while the card
    /// stays, JUNC keeps their choice and saves it as the Input's `profile`
    /// (unless they turned the card off), so it is what comes back on the
    /// next re-plug.
    fn track_profiles(
        &mut self,
        config: &Config,
        graph: &Graph,
        notices: &mut Vec<Notice>,
    ) -> (Vec<Action>, Option<Config>) {
        let serial = |d: &crate::graph::Device| d.props.number("object.serial").unwrap_or(d.id);
        let mut actions = Vec::new();
        let mut new_config: Option<Config> = None;
        let mut seen = HashSet::new();
        for (jx, j) in config
            .junction
            .iter()
            .enumerate()
            .filter(|(_, j)| j.enabled)
        {
            for (ix, input) in j.input.iter().enumerate().filter(|(_, i)| i.enabled) {
                let (Some(device), Some(want)) = (&input.matcher.device, &input.matcher.profile)
                else {
                    continue;
                };
                let Some(card) = crate::proaudio::card(graph, device) else {
                    continue;
                };
                // Never on UCM cards: Pro Audio would bypass the speaker
                // setup UCM does (Asahi built-in audio).
                if want == crate::proaudio::PRO_AUDIO && card.uses_ucm() {
                    continue;
                }
                let key = serial(card);
                // One wish per card: the first Input in the config wins.
                if !seen.insert(key) {
                    continue;
                }
                let Some(active) = card.active_profile.clone() else {
                    continue; // profiles not read yet
                };
                let entry = self.cards.get_mut(&key).filter(|e| &e.want == want);
                match entry {
                    Some(e) if e.user => {}
                    Some(e) if &active == want => e.confirmed = true,
                    Some(e) if e.confirmed => {
                        e.user = true;
                        let keep = active != "off";
                        if keep {
                            let c = new_config.get_or_insert_with(|| config.clone());
                            c.junction[jx].input[ix].matcher.profile = Some(active.clone());
                        }
                        notices.push(Notice::ProfileChanged {
                            junction: j.id.clone(),
                            input: input.id.clone(),
                            profile: keep.then_some(active),
                        });
                    }
                    Some(_) => {} // JUNC's switch has not shown up yet
                    None if &active == want => {
                        self.cards.insert(
                            key,
                            CardProfile {
                                want: want.clone(),
                                confirmed: true,
                                user: false,
                            },
                        );
                    }
                    None => {
                        let Some(p) = card
                            .profiles
                            .iter()
                            .find(|p| &p.name == want && p.available)
                        else {
                            continue;
                        };
                        actions.push(Action::SetDeviceProfile {
                            device: card.id,
                            index: p.index,
                            profile: p.name.clone(),
                        });
                        self.cards.insert(
                            key,
                            CardProfile {
                                want: want.clone(),
                                confirmed: false,
                                user: false,
                            },
                        );
                    }
                }
            }
        }
        let present: HashSet<u32> = graph.devices.values().map(serial).collect();
        self.cards.retain(|k, _| present.contains(k));
        (actions, new_config)
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
        if crate::plan::stream_target_serial(reference).is_some() {
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
