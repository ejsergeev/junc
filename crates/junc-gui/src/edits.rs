// SPDX-License-Identifier: GPL-3.0-or-later

//! What the Main screen changes in the config (F4, F7, F8): pure functions
//! on [`Config`]. The caller reads the config, applies one, and sends it
//! back with the revision it read (`GetConfig` → `Apply`), as the CLI
//! does, so the file keeps its comments and concurrent edits are caught.
//! Junction-level changes (on/off, master volume, delete) have their own
//! D-Bus methods and do not come through here.

use junc_core::config::slugify;
use junc_core::model::{Config, Input, Match};

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
                Edit::AddInput { .. } => {}
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
                Edit::AddInput { .. } => {}
            }
        }
    }
    true
}

/// An id from `label` that no other Input of the Junction has: the slug,
/// then `-2`, `-3`, … (as the CLI's `junc input add` makes them).
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
}
