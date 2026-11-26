// SPDX-License-Identifier: GPL-3.0-or-later

//! The Add Input picker (design board 7, F2): what a Junction can take
//! audio from, from juncd's candidates and Junctions, in the board's
//! groups. Plain data, so it is tested without a window; the window words
//! the groups and the monitors in the UI language.

use crate::main_view::{Icon, match_icon};
use junc_core::model::Match;
use junc_ipc::view::Describe;
use std::collections::BTreeSet;

/// The picker's groups, in the board's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    /// Running apps.
    Apps,
    /// Hardware inputs.
    Hardware,
    /// System audio: what an output plays.
    System,
    /// Other Junctions and Cables.
    Junctions,
}

/// One row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub group: Group,
    /// The app, device or Junction name; for system audio the output's
    /// name (the window says "Monitor of …").
    pub label: String,
    pub icon: Icon,
    /// A Cable among the Junctions.
    pub cable: bool,
    /// The Junction already has an Input taking this.
    pub added: bool,
    /// What the new Input matches.
    pub matcher: Match,
}

/// Whether two matches take the same thing.
fn same(a: &Match, b: &Match) -> bool {
    let key = |m: &Match| {
        (
            m.binary.clone().or_else(|| m.app.clone()),
            m.device.clone(),
            m.monitor.clone(),
            m.junction.clone(),
        )
    };
    key(a) == key(b)
}

/// Junctions that take audio from `id`, directly or through others: adding
/// one of them as an Input of `id` would make a loop.
fn fed_by(d: &Describe, id: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut todo = vec![id.to_owned()];
    while let Some(next) = todo.pop() {
        for j in &d.junctions {
            let takes = j
                .inputs
                .iter()
                .any(|i| i.matcher.junction.as_deref() == Some(next.as_str()));
            if takes && found.insert(j.id.clone()) {
                todo.push(j.id.clone());
            }
        }
    }
    found
}

/// The rows for Junction `junction`, grouped and sorted by name.
pub fn entries(d: &Describe, junction: &str) -> Vec<Entry> {
    let Some(me) = d.junctions.iter().find(|j| j.id == junction) else {
        return Vec::new();
    };
    let added = |m: &Match| me.inputs.iter().any(|i| same(&i.matcher, m));
    let mut out: Vec<Entry> = Vec::new();
    let mut push = |group: Group, label: String, matcher: Match, cable: bool| {
        if out
            .iter()
            .any(|e| e.group == group && same(&e.matcher, &matcher))
        {
            return;
        }
        out.push(Entry {
            group,
            label,
            icon: if group == Group::Junctions {
                Icon::Junction
            } else {
                match_icon(&matcher)
            },
            cable,
            added: added(&matcher),
            matcher,
        });
    };
    for c in &d.candidates {
        match c.kind.as_str() {
            "app_playback" => {
                let Some(name) = c.app.clone().or_else(|| c.binary.clone()) else {
                    continue;
                };
                let matcher = Match {
                    binary: c.binary.clone(),
                    app: c.binary.is_none().then(|| name.clone()),
                    ..Match::default()
                };
                push(Group::Apps, name, matcher, false);
            }
            "source" => {
                let matcher = Match {
                    device: Some(c.node.clone()),
                    ..Match::default()
                };
                push(Group::Hardware, c.description.clone(), matcher, false);
            }
            "sink" => {
                let matcher = Match {
                    monitor: Some(c.node.clone()),
                    ..Match::default()
                };
                push(Group::System, c.description.clone(), matcher, false);
            }
            _ => {}
        }
    }
    let loops = fed_by(d, junction);
    for j in &d.junctions {
        if j.id == junction || loops.contains(&j.id) {
            continue;
        }
        let matcher = Match {
            junction: Some(j.id.clone()),
            ..Match::default()
        };
        push(
            Group::Junctions,
            j.name.clone(),
            matcher,
            j.kind == junc_core::model::Kind::Cable,
        );
    }
    out.sort_by_key(|a| (a.group, a.label.to_lowercase()));
    out
}

/// The rows whose name contains `query` (any case), as indexes.
pub fn filter(entries: &[Entry], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| q.is_empty() || e.label.to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use junc_ipc::view::Candidate;

    fn describe() -> Describe {
        let golden = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/golden/describe/full.json"
        ))
        .expect("golden");
        let v: serde_json::Value = serde_json::from_str(&golden).expect("json");
        let mut d: Describe = serde_json::from_value(v["data"].clone()).expect("describe");
        let c = |node: &str, kind: &str, app: Option<&str>| Candidate {
            node: node.to_owned(),
            description: node.to_owned(),
            kind: kind.to_owned(),
            app: app.map(str::to_owned),
            binary: app.map(str::to_lowercase),
            bluetooth: None,
        };
        d.candidates = vec![
            c("spotify", "app_playback", Some("Spotify")),
            c("spotify.2", "app_playback", Some("Spotify")),
            c("discord", "app_playback", Some("Discord")),
            c("obs", "app_capture", Some("OBS")),
            c("USB Microphone", "source", None),
            c("HDMI Output", "sink", None),
            c("junc.firefox_to_obs", "junction", None),
        ];
        d.junctions[0].inputs[2].matcher.device = Some("USB Microphone".into());
        d
    }

    #[test]
    fn rows_are_grouped_and_say_what_is_added() {
        let d = describe();
        let e = entries(&d, "virtual-mic-for-zoom");
        let groups: Vec<Group> = e.iter().map(|e| e.group).collect();
        let mut sorted = groups.clone();
        sorted.sort();
        assert_eq!(groups, sorted, "in the board's group order");
        let spotify = e
            .iter()
            .find(|e| e.matcher.binary.as_deref() == Some("spotify"))
            .expect("spotify");
        assert!(spotify.added && spotify.group == Group::Apps);
        assert_eq!(spotify.icon, Icon::Music);
        let labels: Vec<(Group, &str, bool)> = e
            .iter()
            .map(|e| (e.group, e.label.as_str(), e.added))
            .collect();
        assert_eq!(
            labels,
            [
                (Group::Apps, "Discord", false),
                (Group::Apps, "Spotify", true),
                (Group::Hardware, "USB Microphone", true),
                (Group::System, "HDMI Output", false),
                (Group::Junctions, "Firefox → OBS", false),
            ],
            "one row per app; recording apps and Junction nodes are not sources here"
        );
        let cable = e
            .iter()
            .find(|e| e.group == Group::Junctions)
            .expect("the Cable");
        assert!(cable.cable && !cable.added);
        assert_eq!(cable.matcher.junction.as_deref(), Some("firefox-to-obs"));
        assert!(
            e.iter()
                .all(|e| e.matcher.junction.as_deref() != Some("virtual-mic-for-zoom")),
            "never itself"
        );
        assert!(entries(&d, "gone").is_empty());
    }

    #[test]
    fn junctions_that_would_loop_are_left_out() {
        let mut d = describe();
        // The Cable takes the Zoom mic: the mic may not take the Cable.
        let zoom = d.junctions[0].inputs[0].clone();
        let mut feed = zoom;
        feed.matcher = Match {
            junction: Some("virtual-mic-for-zoom".into()),
            ..Match::default()
        };
        d.junctions[1].inputs.push(feed);
        let e = entries(&d, "virtual-mic-for-zoom");
        assert!(e.iter().all(|e| e.group != Group::Junctions));
        // The other way round is fine.
        let e = entries(&d, "firefox-to-obs");
        assert!(
            e.iter()
                .any(|e| e.matcher.junction.as_deref() == Some("virtual-mic-for-zoom"))
        );
    }

    #[test]
    fn the_search_matches_names_in_any_case() {
        let e = entries(&describe(), "virtual-mic-for-zoom");
        let hits = filter(&e, "SPOT");
        assert!(!hits.is_empty());
        assert!(
            hits.iter()
                .all(|&i| e[i].label.to_lowercase().contains("spot"))
        );
        assert_eq!(filter(&e, "  ").len(), e.len());
        assert!(filter(&e, "no such thing").is_empty());
    }
}
