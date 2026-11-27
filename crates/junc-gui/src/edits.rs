// SPDX-License-Identifier: GPL-3.0-or-later

//! What the Main screen changes in the config (F4, F7, F8): pure functions
//! on [`Config`]. The caller reads the config, applies one, and sends it
//! back with the revision it read (`GetConfig` → `Apply`), as the CLI
//! does, so the file keeps its comments and concurrent edits are caught.
//! Junction-level changes (on/off, master volume, delete) have their own
//! D-Bus methods and do not come through here.

use junc_core::config::slugify;
use junc_core::model::{Config, Input, Listen, MAX_BUS_CHANNELS, Match};

/// An Input or a Listen output of a Junction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Input(String),
    Listen(String),
}

/// One change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// Turn it on or off.
    Enabled(Part, bool),
    /// Its volume in percent.
    Volume(Part, u16),
    /// Remove these channel pairs, given the pairs in use now (the default
    /// mapping when the config lists none): the rest become the config's
    /// own list.
    Unlink {
        part: Part,
        remove: Vec<[u8; 2]>,
        in_use: Vec<[u8; 2]>,
    },
    /// Add an Input taking `matcher`; its id comes from `label`.
    AddInput { matcher: Box<Match>, label: String },
    /// Add a Listen output to `target` (`default` or an output node.name);
    /// its id comes from `label`.
    AddListen { target: String, label: String },
    /// Add this many channels to the Bus (board 16: a pair or one).
    AddBus(u8),
    /// Input Options (board 15): mute the app elsewhere while captured.
    Exclusive { input: String, on: bool },
    /// Input Options: Input channel `ch` feeds Bus channel `bus` alone
    /// (`None`: nothing), given the pairs in use now.
    Feed {
        input: String,
        ch: u8,
        bus: Option<u8>,
        in_use: Vec<[u8; 2]>,
    },
    /// Input Options: remove the Input.
    RemoveInput(String),
}

/// Applies `edit` to Junction `junction`. Returns false when it or the
/// part is gone (someone else changed the config meanwhile).
pub fn apply(config: &mut Config, junction: &str, edit: &Edit) -> bool {
    let Some(j) = config.junction.iter_mut().find(|j| j.id == junction) else {
        return false;
    };
    let part = match edit {
        Edit::Enabled(p, _) | Edit::Volume(p, _) | Edit::Unlink { part: p, .. } => p,
        Edit::AddInput { matcher, label } => {
            let id = unique_id(label, j.input.iter().map(|i| i.id.as_str()));
            j.input.push(new_input(id, (**matcher).clone()));
            return true;
        }
        Edit::AddBus(n) => {
            let wanted = j.bus.channels.saturating_add(*n);
            if wanted > MAX_BUS_CHANNELS {
                return false;
            }
            j.bus.channels = wanted;
            return true;
        }
        Edit::AddListen { target, label } => {
            let id = unique_id(label, j.listen.iter().map(|l| l.id.as_str()));
            j.listen.push(new_listen(id, target));
            return true;
        }
        Edit::RemoveInput(id) => {
            let before = j.input.len();
            j.input.retain(|i| &i.id != id);
            return j.input.len() < before;
        }
        Edit::Exclusive { input, on } => {
            let Some(i) = j.input.iter_mut().find(|i| &i.id == input) else {
                return false;
            };
            i.exclusive = *on;
            return true;
        }
        Edit::Feed {
            input,
            ch,
            bus,
            in_use,
        } => {
            if bus.is_some_and(|b| b == 0 || b > j.bus.channels) {
                return false;
            }
            let Some(i) = j.input.iter_mut().find(|i| &i.id == input) else {
                return false;
            };
            let mut links: Vec<[u8; 2]> = i
                .links
                .as_deref()
                .unwrap_or(in_use)
                .iter()
                .filter(|p| p[0] != *ch)
                .copied()
                .collect();
            links.extend(bus.map(|b| [*ch, b]));
            links.sort_unstable();
            i.links = Some(links);
            return true;
        }
    };
    match part {
        Part::Input(id) => {
            let Some(i) = j.input.iter_mut().find(|i| &i.id == id) else {
                return false;
            };
            match edit {
                Edit::Enabled(_, on) => i.enabled = *on,
                Edit::Volume(_, v) => i.volume = *v,
                Edit::Unlink { remove, in_use, .. } => {
                    i.links = Some(remaining(i.links.as_deref(), in_use, remove));
                }
                _ => {}
            }
        }
        Part::Listen(id) => {
            let Some(l) = j.listen.iter_mut().find(|l| &l.id == id) else {
                return false;
            };
            match edit {
                Edit::Enabled(_, on) => l.enabled = *on,
                Edit::Volume(_, v) => l.volume = *v,
                Edit::Unlink { remove, in_use, .. } => {
                    l.links = Some(remaining(l.links.as_deref(), in_use, remove));
                }
                _ => {}
            }
        }
    }
    true
}

/// An id from `label` that no other Input (or Listen output) of the
/// Junction has: the slug, then `-2`, `-3`, … (as the CLI makes them).
fn unique_id<'a>(label: &str, taken: impl Iterator<Item = &'a str> + Clone) -> String {
    let mut base = slugify(label);
    base.truncate(56);
    let base = match base.trim_end_matches('-') {
        "" => "input".to_owned(),
        b => b.to_owned(),
    };
    (1..)
        .map(|n| {
            if n == 1 {
                base.clone()
            } else {
                format!("{base}-{n}")
            }
        })
        .find(|id| taken.clone().all(|t| t != id))
        .unwrap_or(base)
}

/// A new Input with the model's defaults (on, 100 %, default links).
fn new_input(id: String, matcher: Match) -> Input {
    let mut t = toml::Table::new();
    t.insert("id".into(), id.into());
    t.insert(
        "match".into(),
        toml::Value::try_from(&matcher).unwrap_or_else(|e| unreachable!("{e}")),
    );
    toml::Value::Table(t)
        .try_into()
        .unwrap_or_else(|e| unreachable!("a minimal Input always parses: {e}"))
}

/// A Listen output to `target` with everything else at its default.
fn new_listen(id: String, target: &str) -> Listen {
    let mut t = toml::Table::new();
    t.insert("id".into(), id.into());
    t.insert("target".into(), target.into());
    toml::Value::Table(t)
        .try_into()
        .unwrap_or_else(|e| unreachable!("a minimal Listen always parses: {e}"))
}

fn remaining(own: Option<&[[u8; 2]]>, in_use: &[[u8; 2]], remove: &[[u8; 2]]) -> Vec<[u8; 2]> {
    own.unwrap_or(in_use)
        .iter()
        .filter(|p| !remove.contains(p))
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"schema = 1
[[junction]]
id = "zoom"
name = "Zoom"
bus = { channels = 4 }
[[junction.input]]
id = "mic"
match = { device = "usb-mic" }
[[junction.input]]
id = "music"
match = { binary = "spotify" }
links = [[1, 3], [2, 4]]
[[junction.listen]]
id = "hp"
target = "default"
"#;

    fn config() -> Config {
        junc_core::config::load(CONFIG, "junctions.toml")
            .expect("config")
            .config
    }

    #[test]
    fn toggles_and_volumes_reach_the_right_part() {
        let mut c = config();
        assert!(apply(
            &mut c,
            "zoom",
            &Edit::Enabled(Part::Input("mic".into()), false)
        ));
        assert!(apply(
            &mut c,
            "zoom",
            &Edit::Volume(Part::Listen("hp".into()), 80)
        ));
        assert!(!c.junction[0].input[0].enabled);
        assert!(c.junction[0].input[1].enabled);
        assert_eq!(c.junction[0].listen[0].volume, 80);
        assert!(!apply(
            &mut c,
            "zoom",
            &Edit::Enabled(Part::Input("gone".into()), true)
        ));
        assert!(!apply(
            &mut c,
            "other",
            &Edit::Volume(Part::Listen("hp".into()), 1)
        ));
    }

    #[test]
    fn unlinking_keeps_the_other_pairs() {
        let mut c = config();
        // Default mapping in use: it becomes the Input's own list, less one.
        let mic = Part::Input("mic".into());
        assert!(apply(
            &mut c,
            "zoom",
            &Edit::Unlink {
                part: mic,
                remove: vec![[1, 1]],
                in_use: vec![[1, 1], [2, 2]],
            }
        ));
        assert_eq!(c.junction[0].input[0].links, Some(vec![[2, 2]]));
        // An own list is edited as it is.
        let music = Part::Input("music".into());
        assert!(apply(
            &mut c,
            "zoom",
            &Edit::Unlink {
                part: music,
                remove: vec![[2, 4]],
                in_use: vec![],
            }
        ));
        assert_eq!(c.junction[0].input[1].links, Some(vec![[1, 3]]));
    }

    #[test]
    fn added_inputs_get_their_own_id_and_validate() {
        let mut c = config();
        let firefox = Match {
            binary: Some("firefox".into()),
            ..Match::default()
        };
        let mic = Match {
            device: Some("alsa_input.other".into()),
            ..Match::default()
        };
        assert!(apply(
            &mut c,
            "zoom",
            &Edit::AddInput {
                matcher: Box::new(firefox),
                label: "Firefox".into()
            }
        ));
        // "Music" is taken by an Input already… as "music".
        assert!(apply(
            &mut c,
            "zoom",
            &Edit::AddInput {
                matcher: Box::new(mic),
                label: "Music".into()
            }
        ));
        let ids: Vec<&str> = c.junction[0].input.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["mic", "music", "firefox", "music-2"]);
        assert!(c.junction[0].input[2].enabled);
        assert_eq!(c.junction[0].input[2].volume, 100);
        let text = toml::to_string(&c).expect("toml");
        junc_core::config::load(&text, "junctions.toml").expect("still valid");
        assert!(!apply(
            &mut c,
            "gone",
            &Edit::AddInput {
                matcher: Box::default(),
                label: "x".into()
            }
        ));
    }

    #[test]
    fn added_listen_outputs_get_their_own_id_and_validate() {
        let mut c = config();
        let add = |c: &mut Config, target: &str, label: &str| {
            apply(
                c,
                "zoom",
                &Edit::AddListen {
                    target: target.into(),
                    label: label.into(),
                },
            )
        };
        assert!(add(&mut c, "alsa_output.hdmi", "HDMI Output"));
        assert!(add(&mut c, "default", "default"));
        assert!(add(&mut c, "alsa_output.hdmi-2", "HDMI Output"));
        let l = &c.junction[0].listen;
        let got: Vec<(&str, &str)> = l
            .iter()
            .map(|l| (l.id.as_str(), l.target.as_str()))
            .collect();
        assert_eq!(
            &got[got.len() - 3..],
            [
                ("hdmi-output", "alsa_output.hdmi"),
                ("default", "default"),
                ("hdmi-output-2", "alsa_output.hdmi-2"),
            ]
        );
        assert!(l.iter().all(|l| l.enabled && l.links.is_none()));
        let text = toml::to_string(&c).expect("toml");
        junc_core::config::load(&text, "junctions.toml").expect("still valid");
        assert!(!apply(
            &mut c,
            "gone",
            &Edit::AddListen {
                target: "default".into(),
                label: "default".into()
            }
        ));
    }

    #[test]
    fn the_bus_grows_by_a_pair_or_one_up_to_its_limit() {
        let mut c = config();
        assert!(apply(&mut c, "zoom", &Edit::AddBus(2)));
        assert!(apply(&mut c, "zoom", &Edit::AddBus(1)));
        assert_eq!(c.junction[0].bus.channels, 7);
        c.junction[0].bus.channels = MAX_BUS_CHANNELS - 1;
        assert!(!apply(&mut c, "zoom", &Edit::AddBus(2)), "never past 64");
        assert_eq!(c.junction[0].bus.channels, MAX_BUS_CHANNELS - 1);
        assert!(apply(&mut c, "zoom", &Edit::AddBus(1)));
        let text = toml::to_string(&c).expect("toml");
        junc_core::config::load(&text, "junctions.toml").expect("still valid");
        assert!(!apply(&mut c, "gone", &Edit::AddBus(1)));
    }

    #[test]
    fn input_options_change_exclusive_feeds_and_remove_it() {
        let mut c = config();
        let ex = |on| Edit::Exclusive {
            input: "music".into(),
            on,
        };
        assert!(apply(&mut c, "zoom", &ex(true)));
        assert!(c.junction[0].input[1].exclusive);
        // music: 1 → 3, 2 → 4. Channel 1 to Bus 1 instead; 2 to nothing.
        let feed = |ch, bus| Edit::Feed {
            input: "music".into(),
            ch,
            bus,
            in_use: vec![[1, 3], [2, 4]],
        };
        assert!(apply(&mut c, "zoom", &feed(1, Some(1))));
        assert_eq!(c.junction[0].input[1].links, Some(vec![[1, 1], [2, 4]]));
        assert!(apply(&mut c, "zoom", &feed(2, None)));
        assert_eq!(c.junction[0].input[1].links, Some(vec![[1, 1]]));
        assert!(
            !apply(&mut c, "zoom", &feed(1, Some(5))),
            "no Bus channel 5"
        );
        // mic has no list of its own: the pairs in use become it.
        assert!(apply(
            &mut c,
            "zoom",
            &Edit::Feed {
                input: "mic".into(),
                ch: 2,
                bus: Some(1),
                in_use: vec![[1, 1], [2, 2]],
            }
        ));
        assert_eq!(c.junction[0].input[0].links, Some(vec![[1, 1], [2, 1]]));
        let text = toml::to_string(&c).expect("toml");
        junc_core::config::load(&text, "junctions.toml").expect("still valid");
        assert!(apply(&mut c, "zoom", &Edit::RemoveInput("mic".into())));
        assert!(!apply(&mut c, "zoom", &Edit::RemoveInput("mic".into())));
        let ids: Vec<&str> = c.junction[0].input.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["music"]);
        assert!(!apply(
            &mut c,
            "zoom",
            &Edit::Exclusive {
                input: "mic".into(),
                on: true
            }
        ));
    }
}
