// SPDX-License-Identifier: GPL-3.0-or-later

//! Onboarding (design boards 8 and 9, UX onboarding): a first launch with
//! no Junctions shows the system check, then the starting points. Whether
//! it is done lives in the GUI's own state file,
//! `$XDG_STATE_HOME/junc/gui.toml` (`~/.local/state/junc/gui.toml`), never
//! in the config: the config is juncd's, and someone who set JUNC up with
//! the CLI or an agent has Junctions and never sees onboarding.

use junc_ipc::view::Describe;
use std::path::{Path, PathBuf};

/// The step on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Step 1: the system check (board 8).
    Check,
    /// Step 2: pick a starting point (board 9).
    Templates,
}

/// The state file.
pub fn state_file() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".local/state"))
        })?;
    Some(state.join("junc/gui.toml"))
}

/// A yes/no the state file at `path` keeps (`onboarded`, `follow_theme`).
pub fn flag(path: &Path, key: &str) -> Option<bool> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| t.get(key).and_then(toml::Value::as_bool))
}

/// Sets `key` in the state file, keeping what else it holds.
pub fn set_flag(path: &Path, key: &str, value: bool) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut t = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .unwrap_or_default();
    t.insert(key.to_owned(), value.into());
    let body = toml::to_string(&t).map_err(std::io::Error::other)?;
    std::fs::write(
        path,
        format!("# junc-gui's own state; safe to delete (onboarding shows again).\n{body}"),
    )
}

/// Whether onboarding was finished, as `path` says.
pub fn done(path: &Path) -> bool {
    flag(path, "onboarded").unwrap_or(false)
}

/// Records that onboarding is finished.
pub fn mark_done(path: &Path) -> std::io::Result<()> {
    set_flag(path, "onboarded", true)
}

/// Whether to show onboarding: not done yet and juncd has no Junctions
/// (or cannot say yet).
pub fn needed(done: bool, d: Option<&Describe>) -> bool {
    !done && d.is_none_or(|d| d.junctions.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_is_remembered_in_the_state_file() {
        let dir = std::env::temp_dir().join(format!("junc-gui-onboarding-{}", std::process::id()));
        let file = dir.join("junc/gui.toml");
        assert!(!done(&file), "no file yet");
        mark_done(&file).expect("write");
        assert!(done(&file));
        std::fs::write(&file, "onboarded = false\n").expect("write");
        assert!(!done(&file));
        mark_done(&file).expect("write");
        set_flag(&file, "follow_theme", false).expect("write");
        assert!(done(&file), "other keys stay");
        assert_eq!(flag(&file, "follow_theme"), Some(false));
        std::fs::write(&file, "not toml [").expect("write");
        assert!(!done(&file), "a broken file shows onboarding again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_first_launch_without_junctions_onboards() {
        let golden = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/golden/describe/full.json"
        ))
        .expect("golden");
        let v: serde_json::Value = serde_json::from_str(&golden).expect("json");
        let mut d: Describe = serde_json::from_value(v["data"].clone()).expect("describe");
        assert!(!needed(false, Some(&d)), "set up with the CLI already");
        assert!(!needed(true, None));
        assert!(needed(false, None));
        d.junctions.clear();
        assert!(needed(false, Some(&d)));
        assert!(!needed(true, Some(&d)));
    }
}
