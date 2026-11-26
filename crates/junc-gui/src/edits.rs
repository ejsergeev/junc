// SPDX-License-Identifier: GPL-3.0-or-later

//! What the Main screen changes in the config (F4, F7, F8): pure functions
//! on [`Config`]. The caller reads the config, applies one, and sends it
//! back with the revision it read (`GetConfig` → `Apply`), as the CLI
//! does, so the file keeps its comments and concurrent edits are caught.
//! Junction-level changes (on/off, master volume, delete) have their own
//! D-Bus methods and do not come through here.

use junc_core::model::Config;

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
}

/// Applies `edit` to Junction `junction`. Returns false when it or the
/// part is gone (someone else changed the config meanwhile).
pub fn apply(config: &mut Config, junction: &str, edit: &Edit) -> bool {
    let Some(j) = config.junction.iter_mut().find(|j| j.id == junction) else {
        return false;
    };
    let part = match edit {
        Edit::Enabled(p, _) | Edit::Volume(p, _) | Edit::Unlink { part: p, .. } => p,
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
            }
        }
    }
    true
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
}
