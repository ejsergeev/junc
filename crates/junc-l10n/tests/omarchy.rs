// SPDX-License-Identifier: GPL-3.0-or-later

//! Live switch (F14 acceptance): picking a language in Omarchy rewrites
//! its language file; JUNC follows within a moment, without a restart,
//! whether the file is written in place or replaced.

use junc_l10n::{Sources, watch};
use std::sync::mpsc::channel;
use std::time::Duration;

#[test]
fn a_new_omarchy_language_is_followed_live() {
    let dir = std::env::temp_dir().join(format!("junc-omarchy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let file = dir.join("language");
    std::fs::write(&file, "en\n").expect("write");
    let sources = Sources {
        explicit: None,
        omarchy_file: Some(file.clone()),
        env: vec!["fr_FR.UTF-8".into()],
    };
    let (tx, rx) = channel();
    watch::spawn(sources, move |lang| {
        let _ = tx.send(lang);
    })
    .expect("watch");
    let next = || {
        rx.recv_timeout(Duration::from_secs(2))
            .expect("no change seen")
    };

    // In place, as `echo de > language` does.
    std::fs::write(&file, "de\n").expect("write");
    assert_eq!(next(), "de");
    // Replaced, as editors and atomic writers do.
    let tmp = dir.join("language.tmp");
    std::fs::write(&tmp, "pt-BR\n").expect("write");
    std::fs::rename(&tmp, &file).expect("rename");
    assert_eq!(next(), "pt-BR");
    // The same language again: no event for the caller.
    std::fs::write(&file, "pt-BR\n").expect("write");
    // Removed: the locale decides.
    std::fs::remove_file(&file).expect("remove");
    assert_eq!(next(), "fr");
    // Another file in the directory is ignored.
    std::fs::write(dir.join("theme"), "tokyo\n").expect("write");
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());

    // Not an Omarchy system: nothing to watch.
    let none = Sources {
        omarchy_file: Some(dir.join("missing-dir/language")),
        ..Sources::default()
    };
    assert!(watch::spawn(none, |_| {}).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
