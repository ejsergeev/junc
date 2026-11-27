// SPDX-License-Identifier: GPL-3.0-or-later

//! AppState (architecture §7.1): the sidebar, computed from juncd's
//! `Describe` snapshot. Plain data, so it is tested without a window; `ui`
//! turns it into Slint models.

use crate::main_view::{self, CardState, Icon};
use junc_core::model::Kind;
use junc_ipc::view::{Describe, JunctionView};

/// One sidebar row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// `active`, `waiting`, `off`, `unknown`.
    pub status: String,
    /// `node.name` of its source, what its meter listens to.
    pub node: String,
    /// D-Bus object path, for its own methods.
    pub path: String,
    pub cable: bool,
    /// Its Inputs' kinds and names, for the second line.
    pub icons: Vec<Icon>,
    pub inputs: Vec<String>,
    /// Inputs that are on and wait for their device.
    pub offline: usize,
    /// App Inputs that are on and wait for their app (board 23).
    pub waiting: usize,
    pub volume: u16,
}

/// Everything the sidebar shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// juncd's state, or `down` when it is not reachable.
    pub daemon: String,
    pub junctions: Vec<Row>,
}

fn status(j: &JunctionView) -> String {
    if !j.enabled {
        return "off".to_owned();
    }
    serde_json::to_value(j.status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// The snapshot for juncd's answer, or for juncd being away.
pub fn snapshot(d: Option<&Describe>) -> Snapshot {
    let Some(d) = d else {
        return Snapshot {
            daemon: "down".to_owned(),
            junctions: Vec::new(),
        };
    };
    Snapshot {
        daemon: d.state.clone(),
        junctions: d
            .junctions
            .iter()
            .map(|j| {
                let m = main_view::main(d, &j.id);
                let cards = m.as_ref().map_or(&[][..], |m| &m.inputs[..]);
                Row {
                    id: j.id.clone(),
                    name: j.name.clone(),
                    enabled: j.enabled,
                    status: status(j),
                    node: j.node.clone(),
                    path: j.path.clone(),
                    cable: j.kind == Kind::Cable,
                    icons: cards.iter().map(|c| c.icon).collect(),
                    // The names of what feeds it now: Inputs that are on.
                    inputs: cards
                        .iter()
                        .filter(|c| c.enabled)
                        .map(|c| c.title.clone())
                        .collect(),
                    offline: cards
                        .iter()
                        .filter(|c| c.enabled && c.state == CardState::Offline)
                        .count(),
                    waiting: cards
                        .iter()
                        .filter(|c| c.enabled && c.state == CardState::NotRunning)
                        .count(),
                    volume: j.volume,
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_show_juncd_and_each_junction() {
        let golden = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/golden/describe/full.json"
        ))
        .expect("golden");
        let v: serde_json::Value = serde_json::from_str(&golden).expect("json");
        let mut d: Describe = serde_json::from_value(v["data"].clone()).expect("describe");
        let s = snapshot(Some(&d));
        assert_eq!(s.daemon, d.state);
        let ids: Vec<&str> = s.junctions.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["virtual-mic-for-zoom", "firefox-to-obs"]);
        assert_eq!(s.junctions[0].inputs[0], "Spotify");
        assert_eq!(s.junctions[0].icons, [Icon::Music, Icon::App, Icon::Mic]);
        assert!(s.junctions[1].cable && !s.junctions[0].cable);
        d.junctions[0].enabled = false;
        assert_eq!(snapshot(Some(&d)).junctions[0].status, "off");
        let down = snapshot(None);
        assert_eq!((down.daemon.as_str(), down.junctions.len()), ("down", 0));
    }
}
