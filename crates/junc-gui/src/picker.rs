// SPDX-License-Identifier: GPL-3.0-or-later

//! The Add Input picker (design board 7, F2): what a Junction can take
//! audio from, from juncd's candidates and Junctions, in the board's
//! groups; and the Add Listen output picker (board 17, F6): the outputs it
//! can be heard on. Plain data, so it is tested without a window; the
//! window words the groups, the monitors and the default output in the UI
//! language.

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
    /// Output devices (Add Listen output).
    Outputs,
}

/// What picking a row adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// An Input taking this.
    Input(Match),
    /// A Listen output to this target (`default` or an output node.name).
    Listen(String),
    /// A Cable's app to take audio from (board 12, "Change app…").
    CableFrom(Match),
    /// A Cable's app to deliver to.
    CableTo(Match),
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
    /// The Junction already has an Input or Listen output taking this.
    pub added: bool,
    pub pick: Pick,
    /// What the row says when the cursor is on it (boards 35, 40).
    pub offer: Offer,
}

/// More about a hardware input, shown under its row.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Offer {
    #[default]
    None,
    /// The card gives more inputs in Pro Audio (F15, board 35): `now` of
    /// `all` channels; the alternative pick takes them all.
    ProAudio {
        now: u32,
        all: Option<u32>,
        matcher: Box<Match>,
    },
    /// A Bluetooth headset mic that switches the headset to its call
    /// profile (F18, board 40); adding it needs Shift+Enter.
    Bluetooth { profile: String },
}

impl Entry {
    /// What the new Input matches, for an Input row.
    pub fn matcher(&self) -> Option<&Match> {
        match &self.pick {
            Pick::Input(m) => Some(m),
            Pick::Listen(_) | Pick::CableFrom(_) | Pick::CableTo(_) => None,
        }
    }
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

/// The Pro Audio offer of the sound card capture node `node` belongs to:
/// ALSA names the node `alsa_input.<card>.<profile>` and the card
/// `alsa_card.<card>` (board 35).
fn pro_audio(d: &Describe, node: &str) -> Offer {
    let Some(card) = node
        .strip_prefix("alsa_input.")
        .and_then(|rest| rest.rsplit_once('.').map(|(card, _)| card))
    else {
        return Offer::None;
    };
    d.devices
        .iter()
        .find(|dev| dev.name.strip_prefix("alsa_card.") == Some(card))
        .and_then(|dev| {
            let o = dev.pro_audio?;
            Some(Offer::ProAudio {
                now: o.now,
                all: o.all,
                matcher: Box::new(Match {
                    device: Some(dev.name.clone()),
                    profile: Some(junc_core::proaudio::PRO_AUDIO.to_owned()),
                    ..Match::default()
                }),
            })
        })
        .unwrap_or_default()
}

/// The rows for Junction `junction`, grouped and sorted by name.
pub fn entries(d: &Describe, junction: &str) -> Vec<Entry> {
    let Some(me) = d.junctions.iter().find(|j| j.id == junction) else {
        return Vec::new();
    };
    let added = |m: &Match| me.inputs.iter().any(|i| same(&i.matcher, m));
    let mut out: Vec<Entry> = Vec::new();
    let mut push = |group: Group, label: String, matcher: Match, cable: bool, offer: Offer| {
        if out
            .iter()
            .any(|e| e.group == group && e.matcher().is_some_and(|m| same(m, &matcher)))
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
            pick: Pick::Input(matcher),
            offer,
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
                push(Group::Apps, name, matcher, false, Offer::None);
            }
            "source" => {
                let matcher = Match {
                    device: Some(c.node.clone()),
                    ..Match::default()
                };
                let offer = match &c.bluetooth {
                    Some(b) if b.switches_to_call => Offer::Bluetooth {
                        profile: b.profile.clone().unwrap_or_default(),
                    },
                    _ => pro_audio(d, &c.node),
                };
                push(
                    Group::Hardware,
                    c.description.clone(),
                    matcher,
                    false,
                    offer,
                );
            }
            "sink" => {
                let matcher = Match {
                    monitor: Some(c.node.clone()),
                    ..Match::default()
                };
                push(
                    Group::System,
                    c.description.clone(),
                    matcher,
                    false,
                    Offer::None,
                );
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
            Offer::None,
        );
    }
    out.sort_by_key(|a| (a.group, a.label.to_lowercase()));
    out
}

/// The outputs Junction `junction` can be heard on: the default output
/// (label empty: the window words it), then the output devices by name. A
/// row is added when a Listen output goes there already, by name or, for
/// one that follows the default, to the device it follows.
pub fn outputs(d: &Describe, junction: &str) -> Vec<Entry> {
    let Some(me) = d.junctions.iter().find(|j| j.id == junction) else {
        return Vec::new();
    };
    let default = d.defaults.output.as_deref();
    let goes_to = |node: &str| {
        me.listens
            .iter()
            .any(|l| l.target == node || (l.target == "default" && default == Some(node)))
    };
    let mut out = vec![Entry {
        group: Group::Outputs,
        label: String::new(),
        icon: Icon::Speaker,
        cable: false,
        added: me.listens.iter().any(|l| l.target == "default"),
        pick: Pick::Listen("default".into()),
        offer: Offer::None,
    }];
    let mut devices: Vec<Entry> = d
        .candidates
        .iter()
        .filter(|c| c.kind == "sink")
        .map(|c| Entry {
            group: Group::Outputs,
            label: if c.description.is_empty() {
                c.node.clone()
            } else {
                c.description.clone()
            },
            icon: if ["hdmi", "displayport"].iter().any(|s| c.node.contains(s)) {
                Icon::Screen
            } else {
                Icon::Speaker
            },
            cable: false,
            added: goes_to(&c.node),
            pick: Pick::Listen(c.node.clone()),
            offer: Offer::None,
        })
        .collect();
    devices.sort_by_key(|e| e.label.to_lowercase());
    out.extend(devices);
    out
}

/// The apps Cable `junction` can take audio from (`to` false: playing
/// apps) or deliver to (`to` true: recording apps), by name; the one it
/// has now is added.
pub fn cable_apps(d: &Describe, junction: &str, to: bool) -> Vec<Entry> {
    let Some(me) = d.junctions.iter().find(|j| j.id == junction) else {
        return Vec::new();
    };
    let current = if to {
        me.cable_to.as_ref()
    } else {
        me.inputs.first().map(|i| &i.matcher)
    };
    let kind = if to { "app_capture" } else { "app_playback" };
    let mut out: Vec<Entry> = Vec::new();
    for c in d.candidates.iter().filter(|c| c.kind == kind) {
        let Some(name) = c.app.clone().or_else(|| c.binary.clone()) else {
            continue;
        };
        let matcher = Match {
            binary: c.binary.clone(),
            app: c.binary.is_none().then(|| name.clone()),
            ..Match::default()
        };
        let taken = |e: &Entry| match &e.pick {
            Pick::CableFrom(m) | Pick::CableTo(m) => same(m, &matcher),
            _ => false,
        };
        if out.iter().any(taken) {
            continue;
        }
        out.push(Entry {
            group: Group::Apps,
            label: name,
            icon: match_icon(&matcher),
            cable: false,
            added: current.is_some_and(|m| same(m, &matcher)),
            pick: if to {
                Pick::CableTo(matcher)
            } else {
                Pick::CableFrom(matcher)
            },
            offer: Offer::None,
        });
    }
    out.sort_by_key(|e| e.label.to_lowercase());
    out
}

/// The rows whose name contains `query` (any case), as indexes. The
/// default output (no label) shows only without a query.
pub fn filter(entries: &[Entry], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            q.is_empty() || (!e.label.is_empty() && e.label.to_lowercase().contains(&q))
        })
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
            .find(|e| {
                e.matcher()
                    .is_some_and(|m| m.binary.as_deref() == Some("spotify"))
            })
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
        assert_eq!(
            cable.matcher().and_then(|m| m.junction.as_deref()),
            Some("firefox-to-obs")
        );
        assert!(
            e.iter()
                .all(|e| e.matcher().and_then(|m| m.junction.as_deref())
                    != Some("virtual-mic-for-zoom")),
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
        assert!(e.iter().any(
            |e| e.matcher().and_then(|m| m.junction.as_deref()) == Some("virtual-mic-for-zoom")
        ));
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

    #[test]
    fn outputs_start_with_the_default_and_say_where_it_is_heard() {
        let mut d = describe();
        d.candidates.push(Candidate {
            node: "alsa_output.builtin".into(),
            description: "Built-in Audio".into(),
            kind: "sink".into(),
            app: None,
            binary: None,
            bluetooth: None,
        });
        d.candidates[5].node = "alsa_output.hdmi-stereo".into();
        d.defaults.output = Some("alsa_output.builtin".into());
        let j = &mut d.junctions[0];
        j.listens.truncate(1);
        j.listens[0].target = "default".into();
        let e = outputs(&d, "virtual-mic-for-zoom");
        let rows: Vec<(&str, Icon, bool)> = e
            .iter()
            .map(|e| (e.label.as_str(), e.icon, e.added))
            .collect();
        assert_eq!(
            rows,
            [
                ("", Icon::Speaker, true),
                ("Built-in Audio", Icon::Speaker, true),
                ("HDMI Output", Icon::Screen, false),
            ],
            "the default first; the device it follows is taken too"
        );
        assert_eq!(e[2].pick, Pick::Listen("alsa_output.hdmi-stereo".into()));
        assert!(e.iter().all(|e| e.matcher().is_none()));
        assert_eq!(filter(&e, "hdmi"), [2], "the search skips the default row");
        assert_eq!(filter(&e, "").len(), 3);
        assert!(outputs(&d, "gone").is_empty());
    }

    #[test]
    fn a_cable_picks_playing_apps_to_take_from_and_recording_apps_to_feed() {
        let mut d = describe();
        d.junctions[1].cable_to = Some(Match {
            binary: Some("obs".into()),
            ..Match::default()
        });
        d.junctions[1].inputs[0].matcher = Match {
            binary: Some("discord".into()),
            ..Match::default()
        };
        let from = cable_apps(&d, "firefox-to-obs", false);
        let rows: Vec<(&str, bool)> = from.iter().map(|e| (e.label.as_str(), e.added)).collect();
        assert_eq!(rows, [("Discord", true), ("Spotify", false)]);
        assert!(matches!(from[1].pick, Pick::CableFrom(_)));
        let to = cable_apps(&d, "firefox-to-obs", true);
        let rows: Vec<(&str, bool)> = to.iter().map(|e| (e.label.as_str(), e.added)).collect();
        assert_eq!(rows, [("OBS", true)]);
        assert!(matches!(to[0].pick, Pick::CableTo(_)));
        assert!(cable_apps(&d, "gone", true).is_empty());
    }
}
