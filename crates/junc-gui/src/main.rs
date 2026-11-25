// SPDX-License-Identifier: GPL-3.0-or-later

//! JUNC graphical interface (Slint): starts the window, follows the
//! Omarchy theme and juncd live. The logic is in the library.

#![forbid(unsafe_code)]

use junc_gui::{daemon, state, theme, theme_service, ui};
use slint::ComponentHandle;

fn main() -> Result<(), slint::PlatformError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--scenario")
        && args
            .iter()
            .skip_while(|a| *a != "--scenario")
            .nth(1)
            .map(String::as_str)
            == Some("gallery")
    {
        return gallery();
    }
    let window = ui::AppWindow::new()?;
    let dir = theme::omarchy_dir();
    let (tokens, source) =
        theme_service::current(dir.as_deref(), true, theme_service::portal_prefers_light);
    ui::apply(&window, &tokens);
    if let theme_service::Source::Omarchy(dir) = source {
        let weak = window.as_weak();
        // Not on Omarchy or no inotify: the theme stays as it started.
        let _ = theme_service::watch(dir, theme_service::portal_prefers_light, move |t| {
            let weak = weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(w) = weak.upgrade() {
                    ui::apply(&w, &t);
                }
            });
        });
    }
    let weak = window.as_weak();
    // Without a session bus the window shows juncd as unreachable.
    let _ = daemon::follow(None, move |d| {
        let snapshot = state::snapshot(d.as_ref());
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(w) = weak.upgrade() {
                ui::show(&w, &snapshot);
            }
        });
    });
    window.run()
}

/// `--scenario gallery`: every component in its states (B13), tinted by the
/// current theme like the app, or by `--theme light|dark`.
fn gallery() -> Result<(), slint::PlatformError> {
    let window = ui::GalleryWindow::new()?;
    // `--theme light|dark` picks a built-in theme (screenshots); otherwise
    // the current one, as the app does.
    let args: Vec<String> = std::env::args().collect();
    let forced = args
        .iter()
        .position(|a| a == "--theme")
        .and_then(|i| args.get(i + 1));
    let tokens = match forced.map(String::as_str) {
        Some("light") => theme::fallback(true),
        Some("dark") => theme::fallback(false),
        _ => {
            let dir = theme::omarchy_dir();
            theme_service::current(dir.as_deref(), true, theme_service::portal_prefers_light).0
        }
    };
    ui::apply_to(&window.global::<ui::Tokens>(), &tokens);
    window.run()
}
