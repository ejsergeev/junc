// SPDX-License-Identifier: GPL-3.0-or-later

//! AppState (architecture §7.1): what the window shows, computed from
//! juncd's `Describe` snapshot. Plain data, so it is tested without a
//! window; `ui` turns it into Slint models.

use junc_ipc::view::{Describe, JunctionView};

/// One sidebar row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// `active`, `waiting`, `off`, `unknown`.
    pub status: String,
}

/// Everything the window shows.
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
            .map(|j| Row {
                id: j.id.clone(),
                name: j.name.clone(),
                enabled: j.enabled,
                status: status(j),
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
        d.junctions[0].enabled = false;
        assert_eq!(snapshot(Some(&d)).junctions[0].status, "off");
        let down = snapshot(None);
        assert_eq!((down.daemon.as_str(), down.junctions.len()), ("down", 0));
    }
}
