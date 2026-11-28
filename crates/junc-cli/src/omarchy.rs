// SPDX-License-Identifier: GPL-3.0-or-later

//! `junc omarchy install|uninstall|status` (F21, architecture §10.4): put
//! JUNC into the Omarchy desktop for this user. Omarchy loads shell plugins
//! only from `~/.config/omarchy/plugins/`, and pacman cannot write user
//! homes, so this links the packaged bar plugin there, turns it on in the
//! bar and adds an "Audio routing · JUNC" row under Setup in the user's menu
//! extensions. Everything it adds is marked and removed again by
//! `uninstall`; nothing of the user's own is touched. It never binds a key:
//! it suggests Super+Shift+J instead.

use crate::edit::Result;
use junc_ipc::ErrorBody;
use junc_l10n::{tr, trf};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Where the package installs the plugin.
pub const INSTALLED: &str = "/usr/share/junc/omarchy/plugin";
/// The plugin's id (its folder name under `plugins/`).
pub const PLUGIN_ID: &str = "io.github.ejsergeev.junc";
/// The menu row's id: under Setup.
pub const MENU_ID: &str = "setup.junc";
/// The key JUNC suggests (free in Omarchy's defaults).
pub const KEYBINDING: &str = "Super+Shift+J";

const BEGIN: &str =
    "// junc: begin (added by `junc omarchy install`; `junc omarchy uninstall` removes it)";
const END: &str = "// junc: end";

/// The plugin folder: `$JUNC_OMARCHY_PLUGIN`, else the installed one.
pub fn plugin_dir() -> Result<PathBuf> {
    let dir = std::env::var_os("JUNC_OMARCHY_PLUGIN")
        .map_or_else(|| PathBuf::from(INSTALLED), PathBuf::from);
    if dir.join("manifest.json").is_file() {
        Ok(dir)
    } else {
        Err(ErrorBody::new(
            "E_OMARCHY",
            trf("no JUNC bar plugin at {dir}", &[("dir", &dir.display())]),
            Some("Install the junc-gui package, or set JUNC_OMARCHY_PLUGIN to the plugin folder."),
        ))
    }
}

/// `$XDG_CONFIG_HOME/omarchy`, else `~/.config/omarchy`.
pub fn omarchy_config(home: &Path) -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map_or_else(|| home.join(".config"), PathBuf::from)
        .join("omarchy")
}

/// What is at `plugins/<id>`.
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

/// Where JUNC stands in this user's Omarchy.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    /// `plugins/io.github.ejsergeev.junc`.
    pub plugin: State,
    /// The menu row is there.
    pub menu: bool,
    /// The shortcut JUNC suggests; never bound by JUNC.
    pub keybinding: &'static str,
}

/// Runs an Omarchy helper: `(command, args)` → it succeeded.
pub type Runner<'a> = &'a dyn Fn(&str, &[&str]) -> bool;

struct Paths {
    root: PathBuf,
    link: PathBuf,
    menu: PathBuf,
}

fn paths(root: &Path) -> Paths {
    let root = root.to_path_buf();
    Paths {
        link: root.join("plugins").join(PLUGIN_ID),
        menu: root.join("extensions/omarchy-menu.jsonc"),
        root,
    }
}

fn plugin_state(link: &Path, plugin: &Path) -> State {
    match std::fs::read_link(link) {
        Ok(target) if target == plugin => State::Linked,
        Ok(_) => State::UsersOwn,
        Err(_) if link.symlink_metadata().is_ok() => State::UsersOwn,
        Err(_) => State::Missing,
    }
}

/// Current state; `config` is the Omarchy settings folder
/// ([`omarchy_config`]).
pub fn status(config: &Path, plugin: &Path) -> Status {
    let p = paths(config);
    let menu = std::fs::read_to_string(&p.menu).is_ok_and(|t| t.contains(BEGIN));
    Status {
        plugin: plugin_state(&p.link, plugin),
        menu,
        keybinding: KEYBINDING,
    }
}

fn io(what: &Path, e: &std::io::Error) -> ErrorBody {
    ErrorBody::new(
        "E_OMARCHY",
        trf(
            "cannot change {f}: {e}",
            &[("f", &what.display()), ("e", e)],
        ),
        Some("Check the permissions of that folder."),
    )
}

/// The menu row, one JSONC member with a trailing comma (Omarchy's JSONC
/// takes trailing commas), between the markers.
fn menu_block(label: &str) -> String {
    let row = serde_json::json!({
        "icon": "\u{f062c}",
        "label": label,
        "when": "command -v junc-gui",
        "action": "uwsm-app -- junc-gui",
    });
    format!("  {BEGIN}\n  \"{MENU_ID}\": {row},\n  {END}\n")
}

/// `text` without our block (and nothing else changed).
fn without_block(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut inside = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        if t == BEGIN {
            inside = true;
        } else if inside && t == END {
            inside = false;
        } else if !inside {
            out.push_str(line);
        }
    }
    out
}

/// `text` with our block right after the opening brace, so the user's
/// entries (with or without trailing commas) stay valid.
fn with_block(text: &str, label: &str) -> Option<String> {
    let clean = without_block(text);
    if clean.trim().is_empty() {
        return Some(format!("{{\n{}}}\n", menu_block(label)));
    }
    let mut out = String::with_capacity(clean.len() + 200);
    let mut done = false;
    for line in clean.split_inclusive('\n') {
        out.push_str(line);
        if !done && !line.trim_start().starts_with("//") && line.contains('{') {
            // Only a line that is just the opening brace is safe to split.
            if line.trim() != "{" {
                return None;
            }
            if !line.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&menu_block(label));
            done = true;
        }
    }
    done.then_some(out)
}

fn write_menu(menu: &Path, text: &str) -> Result<()> {
    if let Some(dir) = menu.parent() {
        std::fs::create_dir_all(dir).map_err(|e| io(dir, &e))?;
    }
    // Written whole and renamed, so the shell never reads half a file.
    let tmp = menu.with_extension("jsonc.junc-tmp");
    std::fs::write(&tmp, text).map_err(|e| io(menu, &e))?;
    std::fs::rename(&tmp, menu).map_err(|e| io(menu, &e))
}

/// Runs an Omarchy helper if it is there; `false` when it is not or fails.
/// The helpers talk to the running shell, so tests pass their own runner.
pub fn omarchy_cmd(cmd: &str, args: &[&str]) -> bool {
    std::process::Command::new(cmd)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Links the plugin, turns it on in the bar and adds the menu row;
/// idempotent. Re-running it rewrites the row in the current language.
pub fn install(config: &Path, plugin: &Path, run: Runner) -> Result<Status> {
    let p = paths(config);
    if !p.root.is_dir() {
        return Err(ErrorBody::new(
            "E_OMARCHY",
            trf(
                "no Omarchy settings at {dir}",
                &[("dir", &p.root.display())],
            ),
            Some("This is for the Omarchy desktop; run it as the user who logs in to it."),
        ));
    }
    match plugin_state(&p.link, plugin) {
        State::Missing => {
            let dir = p.link.parent().unwrap_or(&p.root);
            std::fs::create_dir_all(dir).map_err(|e| io(dir, &e))?;
            std::os::unix::fs::symlink(plugin, &p.link).map_err(|e| io(&p.link, &e))?;
        }
        State::Linked => {}
        State::UsersOwn => {
            return Err(ErrorBody::new(
                "E_OMARCHY",
                trf(
                    "{path} is not JUNC's; it was left alone",
                    &[("path", &p.link.display())],
                ),
                Some("Move it away, then run junc omarchy install again."),
            ));
        }
    }
    let text = std::fs::read_to_string(&p.menu).unwrap_or_default();
    let label = tr("Audio routing · JUNC");
    let Some(next) = with_block(&text, &label) else {
        return Err(ErrorBody::new(
            "E_OMARCHY",
            trf(
                "cannot find where to add the menu row in {path}",
                &[("path", &p.menu.display())],
            ),
            Some("Put the opening { of that file on a line of its own, then run this again."),
        ));
    };
    if next != text {
        write_menu(&p.menu, &next)?;
    }
    // Best effort: the shell learns of the new folder, and the bar takes
    // the widget at its default place. Without a running shell the plugin
    // is still there for `omarchy plugin enable`.
    run("omarchy-shell", &["shell", "rescanPlugins"]);
    run("omarchy-plugin-enable", &[PLUGIN_ID]);
    Ok(status(config, plugin))
}

/// Removes the menu row, takes the widget out of the bar and removes the
/// link; only JUNC's own things.
pub fn uninstall(config: &Path, plugin: &Path, run: Runner) -> Result<Status> {
    let p = paths(config);
    if let Ok(text) = std::fs::read_to_string(&p.menu) {
        let next = without_block(&text);
        if next != text {
            write_menu(&p.menu, &next)?;
        }
    }
    if plugin_state(&p.link, plugin) == State::Linked {
        run("omarchy-plugin-disable", &[PLUGIN_ID]);
        std::fs::remove_file(&p.link).map_err(|e| io(&p.link, &e))?;
        run("omarchy-shell", &["shell", "rescanPlugins"]);
    }
    Ok(status(config, plugin))
}

#[cfg(test)]
mod tests {
    use super::*;

    const USERS: &str = "{\n  // Extend the menu.\n  \"personal\": {\"icon\":\"\",\"label\":\"Personal\"},\n  \"personal.notes\": {\"label\":\"Notes\",\"action\":\"x\"}\n}\n";

    #[test]
    fn the_menu_row_goes_in_after_the_brace_and_comes_out_clean() {
        let added = with_block(USERS, "Audio routing · JUNC").expect("added");
        assert!(added.starts_with("{\n  // junc: begin"), "{added}");
        assert!(added.contains("\"setup.junc\": {"), "{added}");
        assert!(added.contains("Personal"), "the user's rows stay");
        // Idempotent, and a new language replaces the row.
        assert_eq!(
            with_block(&added, "Audio routing · JUNC").as_deref(),
            Some(added.as_str())
        );
        let ru = with_block(&added, "Маршрутизация звука · JUNC").expect("ru");
        assert_eq!(ru.matches("junc: begin").count(), 1);
        assert!(ru.contains("Маршрутизация"));
        assert_eq!(without_block(&ru), USERS);
        // A missing or empty file gets a whole object.
        assert_eq!(
            with_block("", "L").expect("new"),
            format!("{{\n{}}}\n", menu_block("L"))
        );
        // A brace sharing its line with a row is not split.
        assert_eq!(with_block("{ \"a\": {} }\n", "L"), None);
    }

    #[test]
    fn the_row_is_valid_json_once_comments_and_the_trailing_comma_go() {
        let added = with_block(USERS, "Audio routing · JUNC").expect("added");
        let json: String = added
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let v: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(v["setup.junc"]["action"], "uwsm-app -- junc-gui");
        assert_eq!(v["setup.junc"]["when"], "command -v junc-gui");
    }

    #[test]
    fn install_links_and_adds_uninstall_removes_only_ours() {
        let calls = std::cell::RefCell::new(Vec::<String>::new());
        let record = |cmd: &str, args: &[&str]| {
            calls.borrow_mut().push(format!("{cmd} {}", args.join(" ")));
            true
        };
        let home = std::env::temp_dir().join(format!("junc-omarchy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let plugin = home.join("share/plugin");
        std::fs::create_dir_all(&plugin).expect("plugin");
        std::fs::write(plugin.join("manifest.json"), "{}").expect("manifest");
        // Always a folder of the test's own: never the settings of whoever
        // runs it.
        let cfg = home.join(".config/omarchy");

        // Not an Omarchy user: nothing is created.
        assert!(install(&cfg, &plugin, &record).is_err());
        assert!(!cfg.exists());

        std::fs::create_dir_all(cfg.join("extensions")).expect("omarchy");
        std::fs::write(cfg.join("extensions/omarchy-menu.jsonc"), USERS).expect("menu");
        let st = install(&cfg, &plugin, &record).expect("install");
        assert_eq!(st.plugin, State::Linked);
        assert!(st.menu);
        assert_eq!(
            std::fs::read_link(cfg.join("plugins").join(PLUGIN_ID)).expect("link"),
            plugin
        );
        let st = install(&cfg, &plugin, &record).expect("again");
        assert_eq!((st.plugin, st.menu), (State::Linked, true));
        assert_eq!(
            calls.borrow()[..2],
            [
                "omarchy-shell shell rescanPlugins".to_owned(),
                format!("omarchy-plugin-enable {PLUGIN_ID}")
            ]
        );
        calls.borrow_mut().clear();

        let st = uninstall(&cfg, &plugin, &record).expect("uninstall");
        assert_eq!((st.plugin, st.menu), (State::Missing, false));
        assert_eq!(
            *calls.borrow(),
            [
                format!("omarchy-plugin-disable {PLUGIN_ID}"),
                "omarchy-shell shell rescanPlugins".to_owned()
            ]
        );
        assert_eq!(
            std::fs::read_to_string(cfg.join("extensions/omarchy-menu.jsonc")).expect("menu"),
            USERS
        );

        // A plugin folder of the user's own with our id is never replaced.
        std::fs::create_dir_all(cfg.join("plugins").join(PLUGIN_ID)).expect("own");
        assert!(install(&cfg, &plugin, &record).is_err());
        assert_eq!(
            uninstall(&cfg, &plugin, &record).expect("u").plugin,
            State::UsersOwn
        );
        assert!(cfg.join("plugins").join(PLUGIN_ID).is_dir());
        let _ = std::fs::remove_dir_all(&home);
    }
}
