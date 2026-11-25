// SPDX-License-Identifier: GPL-3.0-or-later

//! Live theme reload (docs/architecture.md §7.7). `omarchy-theme-set`
//! builds `current/next-theme`, removes `current/theme` and renames the new
//! one into place, so the `current` directory is watched for that rename and
//! the theme directory itself for edits in place. Events are debounced to
//! the end of the swap, then the tokens are read once and handed over.
//! Without Omarchy (or with "Follow Omarchy theme" off) the built-in theme
//! follows the desktop's colour-scheme preference (XDG portal).

use crate::theme::{self, Tokens};
use inotify::{Inotify, WatchDescriptor, WatchMask};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long the directory must stay quiet before a reload.
pub const DEBOUNCE: Duration = Duration::from_millis(150);

/// Where the tokens came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// An Omarchy theme directory.
    Omarchy(PathBuf),
    /// Built in: Flexoki Light (`light`) or Tokyo Night.
    Builtin { light: bool },
}

/// The desktop prefers a light colour scheme (portal `color-scheme` = 2).
/// Any failure (no portal, no session bus) means no preference.
pub fn portal_prefers_light() -> bool {
    let read = async || -> zbus::Result<u32> {
        let conn = zbus::Connection::session().await?;
        let reply = conn
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.Settings"),
                "ReadOne",
                &("org.freedesktop.appearance", "color-scheme"),
            )
            .await?;
        let v: zbus::OwnedVariant = reply.body().deserialize()?;
        Ok(u32::try_from(v).unwrap_or(0))
    };
    zbus::block_on(read()).is_ok_and(|scheme| scheme == 2)
}

/// The tokens to use now: the Omarchy theme in `dir` if following it and
/// it reads, else the built-in theme for the colour-scheme preference.
pub fn current(
    dir: Option<&Path>,
    follow: bool,
    prefer_light: impl Fn() -> bool,
) -> (Tokens, Source) {
    if follow
        && let Some(d) = dir
        && let Ok(t) = theme::load(d)
    {
        return (t, Source::Omarchy(d.to_owned()));
    }
    let light = prefer_light();
    (theme::fallback(light), Source::Builtin { light })
}

/// Starts a thread that calls `changed` with the new tokens after every
/// theme switch or edit in the Omarchy theme directory `dir`. Fails when
/// its parent (`…/omarchy/current`) does not exist (not Omarchy).
pub fn watch(dir: PathBuf, changed: impl Fn(Tokens) + Send + 'static) -> std::io::Result<()> {
    let parent = dir
        .parent()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?
        .to_owned();
    let name = dir
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    let mut ino = Inotify::init()?;
    ino.watches().add(
        &parent,
        WatchMask::MOVED_TO | WatchMask::CREATE | WatchMask::DELETE | WatchMask::MOVED_FROM,
    )?;
    let edits = WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO | WatchMask::CREATE;
    let mut inner: Option<WatchDescriptor> = ino.watches().add(&dir, edits).ok();
    std::thread::Builder::new()
        .name("junc-theme".into())
        .spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                let Ok(events) = ino.read_events_blocking(&mut buf) else {
                    return;
                };
                let relevant = events
                    .into_iter()
                    .any(|e| Some(&e.wd) == inner.as_ref() || e.name.is_some_and(|n| n == name));
                if !relevant {
                    continue;
                }
                // Wait for the swap to finish: the directory stays quiet.
                loop {
                    std::thread::sleep(DEBOUNCE);
                    let quiet = ino
                        .read_events(&mut buf)
                        .map_or(true, |more| more.count() == 0);
                    if quiet {
                        break;
                    }
                }
                // The renamed-in directory is a new inode: watch it again.
                if let Some(old) = inner.take() {
                    let _ = ino.watches().remove(old);
                }
                inner = ino.watches().add(&dir, edits).ok();
                if let Ok(t) = theme::load(&dir) {
                    changed(t);
                }
            }
        })?;
    Ok(())
}
