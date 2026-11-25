// SPDX-License-Identifier: GPL-3.0-or-later

//! JSON views shared by `Describe`, the Junction `View` property and
//! `junc … --json`. They combine user intent (the config) with what the
//! daemon sees (plan statuses, the graph). Field names are part of API v1:
//! add fields freely, never rename or remove them.

use junc_core::config;
use junc_core::diag::{Diagnostic, Location, Severity};
use junc_core::graph::{Direction, Graph, NodeKind};
use junc_core::matcher::resolve_output;
use junc_core::model::{Config, DefaultRole, Junction, Kind, Match};
use junc_core::plan::{Entity, EntityStatus, Status, default_map};
use serde::{Deserialize, Serialize};

/// A diagnostic as JSON (owned, so clients can read it back).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diag {
    /// Stable code.
    pub code: String,
    /// `error` or `warning`.
    pub severity: String,
    /// What is wrong.
    pub message: String,
    /// File position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Loc>,
    /// How to fix it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// A file position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loc {
    /// File.
    pub file: String,
    /// 1-based line.
    pub line: usize,
    /// 1-based column.
    pub column: usize,
}

impl From<&Diagnostic> for Diag {
    fn from(d: &Diagnostic) -> Self {
        Self {
            code: d.code.to_owned(),
            severity: match d.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            }
            .to_owned(),
            message: d.message.clone(),
            location: d.location.as_ref().map(|l: &Location| Loc {
                file: l.file.clone(),
                line: l.line,
                column: l.column,
            }),
            hint: d.hint.clone(),
        }
    }
}

/// Diagnostics of a config text: empty when it is valid. Warnings of a
/// valid config are included.
pub fn validate_text(text: &str, file: &str) -> Vec<Diag> {
    match config::load(text, file) {
        Ok(l) => l.warnings.iter().map(Diag::from).collect(),
        Err(d) => d.iter().map(Diag::from).collect(),
    }
}

/// State of a config entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Routed.
    Active,
    /// Its app or device is not there.
    Waiting,
    /// Turned off.
    Disabled,
    /// The daemon has not planned it yet (no PipeWire).
    Unknown,
}

impl From<Status> for State {
    fn from(s: Status) -> Self {
        match s {
            Status::Active => Self::Active,
            Status::Waiting => Self::Waiting,
            Status::Disabled => Self::Disabled,
        }
    }
}

/// One Input with its state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputView {
    /// Id.
    pub id: String,
    /// What it takes audio from.
    #[serde(rename = "match")]
    pub matcher: Match,
    /// On or off.
    pub enabled: bool,
    /// Percent.
    pub volume: u16,
    /// Takes the source away from other apps.
    pub exclusive: bool,
    /// State.
    pub status: State,
    /// Names of the nodes it matched now.
    pub matched: Vec<String>,
    /// `[input channel, bus channel]` pairs in use: the config's own list,
    /// or the default mapping for the matched source (empty while waiting).
    pub links: Vec<[u8; 2]>,
    /// The links are set in the config rather than the default mapping.
    pub custom_links: bool,
    /// Links the user chose to leave removed.
    pub suppressed: Vec<[u8; 2]>,
}

/// One Listen output with its state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListenView {
    /// Id.
    pub id: String,
    /// Output device (`default` follows the default output).
    pub target: String,
    /// On or off.
    pub enabled: bool,
    /// Percent.
    pub volume: u16,
    /// State.
    pub status: State,
    /// `[bus channel, output channel]` pairs in use: the config's own list,
    /// or the default mapping for the output (empty while it is missing).
    pub links: Vec<[u8; 2]>,
    /// The links are set in the config rather than the default mapping.
    pub custom_links: bool,
    /// Links the user chose to leave removed.
    pub suppressed: Vec<[u8; 2]>,
}

/// A Junction or Cable with its state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JunctionView {
    /// Id (stable; used in paths and references).
    pub id: String,
    /// Name shown to people.
    pub name: String,
    /// `junction` or `cable`.
    pub kind: Kind,
    /// On or off.
    pub enabled: bool,
    /// Master volume in percent.
    pub volume: u16,
    /// Default role.
    pub default: DefaultRole,
    /// Bus width.
    pub channels: u8,
    /// PipeWire `node.name` of its source (what apps record from).
    pub node: String,
    /// D-Bus object path.
    pub path: String,
    /// State of the Junction itself.
    pub status: State,
    /// Inputs.
    pub inputs: Vec<InputView>,
    /// Listen outputs.
    pub listens: Vec<ListenView>,
    /// For a Cable: state of the app it delivers to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cable_target: Option<State>,
}

/// A node a user could pick as an Input or Listen target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    /// `node.name` (use as `match.node`, `match.device` or Listen target).
    pub node: String,
    /// Human name.
    pub description: String,
    /// `app_playback`, `app_capture`, `source`, `sink` or `junction`.
    pub kind: String,
    /// `application.name` for apps (use as `match.app`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// `application.process.binary` for apps (use as `match.binary`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary: Option<String>,
}

/// Current default devices.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultsView {
    /// Default input `node.name`.
    pub input: Option<String>,
    /// Default output `node.name`.
    pub output: Option<String>,
}

/// Everything an agent needs in one call (`junc describe --json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Describe {
    /// (major, minor).
    pub api_version: (u32, u32),
    /// Daemon package version.
    pub daemon_version: String,
    /// `starting`, `connected` or `waiting_for_pipewire`.
    pub state: String,
    /// Config revision (pass it to Apply).
    pub revision: u64,
    /// Path of the config file.
    pub config_file: String,
    /// Diagnostics of the file on disk when it is invalid (the last good
    /// config stays in use).
    pub config_error: Vec<Diag>,
    /// Junctions and Cables in config order.
    pub junctions: Vec<JunctionView>,
    /// Apps whose audio goes into an enabled Junction now.
    pub capture_apps: Vec<String>,
    /// Default devices now.
    pub defaults: DefaultsView,
    /// Nodes available to route.
    pub candidates: Vec<Candidate>,
}

fn status_of(statuses: &[EntityStatus], junction: &str, entity: &Entity) -> (State, Vec<u32>) {
    statuses
        .iter()
        .find(|s| s.junction == junction && &s.entity == entity)
        .map_or((State::Unknown, Vec::new()), |s| {
            (s.status.into(), s.nodes.clone())
        })
}

fn names(graph: Option<&Graph>, ids: &[u32]) -> Vec<String> {
    let Some(g) = graph else { return Vec::new() };
    ids.iter()
        .filter_map(|id| g.nodes.get(id))
        .map(|n| n.name().to_owned())
        .collect()
}

/// The view of one Junction.
pub fn junction(j: &Junction, statuses: &[EntityStatus], graph: Option<&Graph>) -> JunctionView {
    let (status, _) = status_of(statuses, &j.id, &Entity::Junction);
    JunctionView {
        id: j.id.clone(),
        name: j.name.clone(),
        kind: j.kind,
        enabled: j.enabled,
        volume: j.volume,
        default: j.default,
        channels: j.bus.channels,
        node: junc_core::matcher::junction_node_name(&j.id),
        path: crate::junction_path(&j.id),
        status,
        inputs: j
            .input
            .iter()
            .map(|i| {
                let (status, nodes) = status_of(statuses, &j.id, &Entity::Input(i.id.clone()));
                let links = i.links.clone().unwrap_or_else(|| {
                    let channels = |g: &Graph, n: u32| g.ports_of(n, Direction::Out).count();
                    graph
                        .zip(nodes.first())
                        .map(|(g, &n)| default_map(channels(g, n), usize::from(j.bus.channels)))
                        .unwrap_or_default()
                });
                InputView {
                    id: i.id.clone(),
                    matcher: i.matcher.clone(),
                    enabled: i.enabled,
                    volume: i.volume,
                    exclusive: i.exclusive,
                    status,
                    matched: names(graph, &nodes),
                    links,
                    custom_links: i.links.is_some(),
                    suppressed: i.suppressed.clone(),
                }
            })
            .collect(),
        listens: j
            .listen
            .iter()
            .map(|l| ListenView {
                id: l.id.clone(),
                target: l.target.clone(),
                enabled: l.enabled,
                volume: l.volume,
                status: status_of(statuses, &j.id, &Entity::Listen(l.id.clone())).0,
                links: l.links.clone().unwrap_or_else(|| {
                    graph
                        .and_then(|g| {
                            let out = resolve_output(&l.target, g)?;
                            let n = g.ports_of(out, Direction::In).count();
                            Some(default_map(usize::from(j.bus.channels), n))
                        })
                        .unwrap_or_default()
                }),
                custom_links: l.links.is_some(),
                suppressed: l.suppressed.clone(),
            })
            .collect(),
        cable_target: (j.kind == Kind::Cable)
            .then(|| status_of(statuses, &j.id, &Entity::CableTarget).0),
    }
}

/// Apps captured into an enabled Junction: the `application.name` (or node
/// name) of every app stream an active Input matched.
pub fn capture_apps(statuses: &[EntityStatus], graph: Option<&Graph>) -> Vec<String> {
    let Some(g) = graph else { return Vec::new() };
    let mut out: Vec<String> = statuses
        .iter()
        .filter(|s| matches!(s.entity, Entity::Input(_)) && s.status == Status::Active)
        .flat_map(|s| s.nodes.iter())
        .filter_map(|id| g.nodes.get(id))
        .filter(|n| n.is_app_stream())
        .map(|n| {
            n.props
                .get("application.name")
                .unwrap_or(n.name())
                .to_owned()
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Nodes a user could route.
pub fn candidates(graph: &Graph) -> Vec<Candidate> {
    graph
        .nodes
        .values()
        .filter(|n| !n.is_plumbing() && !n.name().is_empty())
        .filter_map(|n| {
            let kind = match n.kind() {
                NodeKind::Source if n.is_junc() => "junction",
                _ if n.is_junc() => return None,
                NodeKind::AppPlayback => "app_playback",
                NodeKind::AppCapture => "app_capture",
                NodeKind::Source => "source",
                NodeKind::Sink => "sink",
                _ => return None,
            };
            let app = matches!(kind, "app_playback" | "app_capture");
            let prop = |k: &str| app.then(|| n.props.get(k).map(str::to_owned)).flatten();
            Some(Candidate {
                node: n.name().to_owned(),
                description: n.description().to_owned(),
                kind: kind.to_owned(),
                app: prop("application.name"),
                binary: prop("application.process.binary"),
            })
        })
        .collect()
}

/// Inputs to [`describe`] that come from the daemon rather than the config.
#[derive(Debug, Clone, Copy)]
pub struct DaemonInfo<'a> {
    /// Daemon state string.
    pub state: &'a str,
    /// Config revision.
    pub revision: u64,
    /// Config file path.
    pub config_file: &'a str,
    /// Diagnostics of an invalid file on disk.
    pub config_error: Option<&'a [Diagnostic]>,
}

/// The full description.
pub fn describe(
    info: DaemonInfo<'_>,
    config: &Config,
    statuses: &[EntityStatus],
    graph: Option<&Graph>,
) -> Describe {
    Describe {
        api_version: crate::API_VERSION,
        daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
        state: info.state.to_owned(),
        revision: info.revision,
        config_file: info.config_file.to_owned(),
        config_error: info
            .config_error
            .map(|d| d.iter().map(Diag::from).collect())
            .unwrap_or_default(),
        junctions: config
            .junction
            .iter()
            .map(|j| junction(j, statuses, graph))
            .collect(),
        capture_apps: capture_apps(statuses, graph),
        defaults: graph.map_or_else(DefaultsView::default, |g| DefaultsView {
            input: g.defaults.source.clone(),
            output: g.defaults.sink.clone(),
        }),
        candidates: graph.map(candidates).unwrap_or_default(),
    }
}
