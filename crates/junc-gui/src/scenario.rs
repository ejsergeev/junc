// SPDX-License-Identifier: GPL-3.0-or-later

//! Scenarios (architecture §7.5, feature `scenarios`): a fixture in
//! `tests/fixtures/scenarios/<name>.toml` holds a config, a mocked graph
//! (`tests/fixtures/graphs/`), the theme and window size of its design
//! board. The mock IPC runs the config against the graph until it settles,
//! as juncd would, and builds the `Describe` snapshot juncd would send.

use junc_core::backend::{Backend, MockBackend};
use junc_core::graph::Graph;
use junc_core::reconcile::Reconciler;
use junc_ipc::view::{self, DefaultsView, Describe};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    #[allow(dead_code)] // documents the design board
    board: u32,
    title: String,
    #[serde(default)]
    theme: Option<String>,
    #[serde(default)]
    size: Option<[u32; 2]>,
    /// juncd's state; `down` means juncd is not running.
    #[serde(default)]
    daemon: Option<String>,
    #[serde(default)]
    graph: Option<String>,
    #[serde(default)]
    config: Option<String>,
}

/// A loaded scenario.
#[derive(Debug, Clone)]
pub struct Scenario {
    pub name: String,
    pub title: String,
    /// `dark` (Tokyo Night) or `light` (Flexoki Light).
    pub theme: String,
    pub size: (u32, u32),
    /// What juncd would answer; `None` when juncd is down.
    pub describe: Option<Describe>,
}

/// The repository's test fixtures (`$JUNC_FIXTURES` overrides it).
pub fn fixtures() -> PathBuf {
    std::env::var_os("JUNC_FIXTURES")
        .filter(|v| !v.is_empty())
        .map_or_else(
            || PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures")),
            PathBuf::from,
        )
}

/// Loads `tests/fixtures/scenarios/<name>.toml`.
pub fn load(name: &str) -> Result<Scenario, String> {
    let dir = fixtures();
    let path = dir.join("scenarios").join(format!("{name}.toml"));
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let f: Fixture = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let state = f.daemon.unwrap_or_else(|| "connected".to_owned());
    let describe = if state == "down" {
        None
    } else {
        let graph = match &f.graph {
            Some(g) => {
                let p = dir.join("graphs").join(g);
                let json =
                    std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
                Graph::from_pw_dump(&json)?
            }
            None => Graph::default(),
        };
        let config_text = f.config.unwrap_or_else(|| "schema = 1\n".to_owned());
        let config = junc_core::config::load(&config_text, &format!("{name}.toml"))
            .map_err(|d| {
                format!(
                    "{name}: {}",
                    d.iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            })?
            .config;
        Some(settle(&config, graph, &state))
    };
    Ok(Scenario {
        name: name.to_owned(),
        title: f.title,
        theme: f.theme.unwrap_or_else(|| "dark".to_owned()),
        size: f.size.map_or((1440, 900), |[w, h]| (w, h)),
        describe,
    })
}

/// Runs the reconciler against a mock backend until nothing is left to
/// do (a few steps: nodes first, then their links), then describes it.
fn settle(config: &junc_core::model::Config, graph: Graph, state: &str) -> Describe {
    let mut backend = MockBackend::new(graph);
    let mut r = Reconciler::new();
    let mut statuses = Vec::new();
    for _ in 0..8 {
        let step = r.on_graph(config, backend.graph());
        statuses = step.plan.status;
        if step.actions.is_empty() {
            break;
        }
        for a in &step.actions {
            let _ = backend.apply(a);
        }
    }
    let g = backend.graph();
    Describe {
        api_version: junc_ipc::API_VERSION,
        daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
        state: state.to_owned(),
        revision: 1,
        config_file: "~/.config/junc/junctions.toml".to_owned(),
        config_error: Vec::new(),
        junctions: config
            .junction
            .iter()
            .map(|j| view::junction(j, &statuses, Some(g)))
            .collect(),
        capture_apps: view::capture_apps(&statuses, Some(g)),
        defaults: DefaultsView {
            input: g.defaults.source.clone(),
            output: g.defaults.sink.clone(),
        },
        candidates: view::candidates(g),
        devices: view::devices(g),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_scenario_settles_like_juncd() {
        let s = load("default").expect("scenario");
        assert_eq!((s.theme.as_str(), s.size), ("dark", (1440, 900)));
        let d = s.describe.expect("juncd up");
        let names: Vec<&str> = d.junctions.iter().map(|j| j.name.as_str()).collect();
        assert_eq!(names, ["Virtual Mic for Zoom", "Firefox → OBS"]);
        let status = |i: usize| serde_json::to_value(d.junctions[i].status).expect("json");
        assert_eq!(status(0), "active", "settled, not still being built");
        assert!(
            d.capture_apps.iter().any(|a| a == "Spotify"),
            "{:?}",
            d.capture_apps
        );
        assert!(load("no-such-scenario").is_err());
    }
}
