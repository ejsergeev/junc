// SPDX-License-Identifier: GPL-3.0-or-later

//! F21: an Omarchy theme switch retints JUNC without a restart. The swap is
//! done the way `omarchy-theme-set` does it (build next-theme, remove the
//! old directory, rename the new one into place), in a private directory.

use junc_gui::theme::{self, Color};
use junc_gui::theme_service::{self, Source};
use std::path::Path;
use std::sync::mpsc::channel;
use std::time::Duration;

const TOKYO: &str = "mode = \"dark\"\naccent = \"#7aa2f7\"\nselection = \"#292e42\"\nmuted = \"#414868\"\nbackground = \"#1a1b26\"\ndark_background = \"#13141c\"\ndarker_background = \"#0e0e14\"\nlighter_background = \"#24283b\"\nforeground = \"#a9b1d6\"\ndark_foreground = \"#565f89\"\nbright_foreground = \"#c0caf5\"\nred = \"#f7768e\"\nyellow = \"#e0af68\"\ngreen = \"#9ece6a\"\n";

fn flexoki() -> String {
    TOKYO
        .replace("\"dark\"", "\"light\"")
        .replace("#1a1b26", "#fffcf0")
        .replace("#a9b1d6", "#100f0f")
        .replace("#7aa2f7", "#205ea6")
}

fn write_theme(dir: &Path, colors: &str) {
    std::fs::create_dir_all(dir).expect("dir");
    std::fs::write(dir.join("colors.toml"), colors).expect("colors");
    std::fs::write(dir.join("shell.toml"), "[font]\nbase-size = 12\n").expect("shell");
}

#[test]
fn an_omarchy_theme_switch_retints_live() {
    let current = std::env::temp_dir().join(format!("junc-theme-{}", std::process::id()));
    let dir = current.join("theme");
    write_theme(&dir, TOKYO);

    let (start, source) = theme_service::current(Some(&dir), true, || true);
    assert_eq!(source, Source::Omarchy(dir.clone()));
    assert_eq!(start.surface_window, Color::rgb(0x1a, 0x1b, 0x26));

    let (tx, rx) = channel();
    theme_service::watch(
        dir.clone(),
        || false,
        move |t| {
            let _ = tx.send(t);
        },
    )
    .expect("watch");
    let next = || rx.recv_timeout(Duration::from_secs(3)).expect("no reload");

    // omarchy-theme-set: next-theme, rm -rf theme, mv next-theme theme.
    let staged = current.join("next-theme");
    write_theme(&staged, &flexoki());
    std::fs::remove_dir_all(&dir).expect("rm");
    std::fs::rename(&staged, &dir).expect("mv");
    let t = next();
    assert!(!t.dark, "light theme after the switch");
    assert_eq!(t.surface_window.hex(), "#fffcf0");
    assert!(
        rx.recv_timeout(Duration::from_millis(400)).is_err(),
        "one reload per switch"
    );

    // An edit in place to the new directory (theme development).
    std::fs::write(
        dir.join("colors.toml"),
        flexoki().replace("#205ea6", "#ff0000"),
    )
    .expect("edit");
    assert_eq!(next().accent.hex(), "#ff0000");

    // A listed theme without colours (Omarchy 4.5.0's Osaka Jade ships only
    // wallpapers): the built-in theme, not the last one's leftovers.
    std::fs::create_dir_all(staged.join("backgrounds")).expect("dir");
    std::fs::remove_dir_all(&dir).expect("rm");
    std::fs::rename(&staged, &dir).expect("mv");
    assert_eq!(next(), theme::fallback(false));

    // And back.
    write_theme(&staged, TOKYO);
    std::fs::remove_dir_all(&dir).expect("rm");
    std::fs::rename(&staged, &dir).expect("mv");
    assert!(next().dark);
    let _ = std::fs::remove_dir_all(&current);
}

#[test]
fn without_omarchy_the_builtin_theme_follows_the_colour_scheme() {
    let missing = Path::new("/nonexistent/omarchy/current/theme");
    let (t, s) = theme_service::current(Some(missing), true, || true);
    assert_eq!(s, Source::Builtin { light: true });
    assert_eq!(t, theme::fallback(true));
    let (t, s) = theme_service::current(None, true, || false);
    assert_eq!(s, Source::Builtin { light: false });
    assert!(t.dark);
    // Following Omarchy turned off: built in even when a theme is there.
    let dir = std::env::temp_dir().join(format!("junc-theme-off-{}", std::process::id()));
    write_theme(&dir, TOKYO);
    let (_, s) = theme_service::current(Some(&dir), false, || false);
    assert_eq!(s, Source::Builtin { light: false });
    assert!(theme_service::watch(missing.to_owned(), || false, |_| {}).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
