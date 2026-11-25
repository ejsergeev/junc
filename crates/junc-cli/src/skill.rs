// SPDX-License-Identifier: GPL-3.0-or-later

//! `junc skill link|unlink|status` (F25, architecture §9.4): make the JUNC
//! agent skill visible to the coding agents the user has, the way Omarchy
//! links its own skills: `<agent skills folder>/junc` → the installed
//! skill. pacman cannot write user homes, so this runs per user.

use crate::edit::Result;
use junc_ipc::ErrorBody;
use junc_l10n::trf;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Where the package installs the skill.
pub const INSTALLED: &str = "/usr/share/junc/agent/skills/junc";

/// The skill folder: `$JUNC_SKILL_DIR`, else the installed one.
pub fn skill_dir() -> Result<PathBuf> {
    let dir =
        std::env::var_os("JUNC_SKILL_DIR").map_or_else(|| PathBuf::from(INSTALLED), PathBuf::from);
    if dir.join("SKILL.md").is_file() {
        Ok(dir)
    } else {
        Err(ErrorBody::new(
            "E_SKILL",
            trf("no JUNC skill at {dir}", &[("dir", &dir.display())]),
            Some("Install the junc package, or set JUNC_SKILL_DIR to the skill folder."),
        ))
    }
}

/// The agents' skill folders that exist (the same set Omarchy links into).
/// A folder is never created: an agent the user does not have gets nothing.
pub fn folders(home: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = [
        ".agents/skills",
        ".claude/skills",
        ".codex/skills",
        ".pi/agent/skills",
        ".gemini/config/skills",
        ".hermes/skills",
    ]
    .iter()
    .map(|f| home.join(f))
    .collect();
    if let Ok(profiles) = std::fs::read_dir(home.join(".hermes/profiles")) {
        let mut p: Vec<PathBuf> = profiles
            .flatten()
            .map(|e| e.path().join("skills"))
            .collect();
        p.sort();
        out.extend(p);
    }
    out.retain(|f| f.is_dir());
    out
}

/// What is at `<folder>/junc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Our link.
    Linked,
    /// Nothing.
    Missing,
    /// Something of the user's own (a folder, a file, another link): kept.
    UsersOwn,
}

/// One agent folder and its `junc` entry.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub folder: PathBuf,
    pub state: State,
}

fn state(entry: &Path, skill: &Path) -> State {
    match std::fs::read_link(entry) {
        Ok(target) if target == skill => State::Linked,
        Ok(_) => State::UsersOwn,
        Err(_) if entry.symlink_metadata().is_ok() => State::UsersOwn,
        Err(_) => State::Missing,
    }
}

/// Current state in every agent folder.
pub fn status(home: &Path, skill: &Path) -> Vec<Entry> {
    folders(home)
        .into_iter()
        .map(|folder| {
            let state = state(&folder.join("junc"), skill);
            Entry { folder, state }
        })
        .collect()
}

fn io(folder: &Path, e: &std::io::Error) -> ErrorBody {
    ErrorBody::new(
        "E_SKILL",
        trf(
            "cannot change {f}: {e}",
            &[("f", &folder.display()), ("e", e)],
        ),
        Some("Check the permissions of that folder."),
    )
}

/// Links the skill where it is missing; idempotent.
pub fn link(home: &Path, skill: &Path) -> Result<Vec<Entry>> {
    for e in status(home, skill) {
        if e.state == State::Missing {
            std::os::unix::fs::symlink(skill, e.folder.join("junc"))
                .map_err(|x| io(&e.folder, &x))?;
        }
    }
    Ok(status(home, skill))
}

/// Removes only JUNC's own links.
pub fn unlink(home: &Path, skill: &Path) -> Result<Vec<Entry>> {
    for e in status(home, skill) {
        if e.state == State::Linked {
            std::fs::remove_file(e.folder.join("junc")).map_err(|x| io(&e.folder, &x))?;
        }
    }
    Ok(status(home, skill))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_only_into_existing_folders_and_keeps_the_users_own() {
        let home = std::env::temp_dir().join(format!("junc-skill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let skill = home.join("share/junc");
        std::fs::create_dir_all(&skill).expect("skill");
        std::fs::write(skill.join("SKILL.md"), "---\nname: junc\n---\n").expect("skill file");
        for f in [
            ".claude/skills",
            ".codex/skills",
            ".hermes/profiles/work/skills",
        ] {
            std::fs::create_dir_all(home.join(f)).expect("folder");
        }
        // The user wrote their own `junc` skill for Codex.
        std::fs::create_dir_all(home.join(".codex/skills/junc")).expect("own");

        let after = link(&home, &skill).expect("link");
        let states: Vec<(String, State)> = after
            .iter()
            .map(|e| {
                (
                    e.folder
                        .strip_prefix(&home)
                        .unwrap_or(&e.folder)
                        .display()
                        .to_string(),
                    e.state,
                )
            })
            .collect();
        assert_eq!(
            states,
            [
                (".claude/skills".to_owned(), State::Linked),
                (".codex/skills".to_owned(), State::UsersOwn),
                (".hermes/profiles/work/skills".to_owned(), State::Linked),
            ]
        );
        assert!(!home.join(".gemini").exists(), "no folders created");
        assert_eq!(link(&home, &skill).expect("again").len(), 3, "idempotent");

        let after = unlink(&home, &skill).expect("unlink");
        assert!(after.iter().all(|e| e.state != State::Linked));
        assert!(
            home.join(".codex/skills/junc").is_dir(),
            "the user's own skill stays"
        );
        let _ = std::fs::remove_dir_all(&home);
    }
}
