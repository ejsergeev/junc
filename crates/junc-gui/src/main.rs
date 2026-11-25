// SPDX-License-Identifier: GPL-3.0-or-later

//! JUNC graphical interface (Slint): starts the window, follows the
//! Omarchy theme live. The logic is in the library.

#![forbid(unsafe_code)]

use junc_gui::{theme, theme_service, ui};
use slint::ComponentHandle;

fn main() -> Result<(), slint::PlatformError> {
    let window = ui::AppWindow::new()?;
    let dir = theme::omarchy_dir();
    let (tokens, source) =
        theme_service::current(dir.as_deref(), true, theme_service::portal_prefers_light);
    ui::apply(&window, &tokens);
    if let theme_service::Source::Omarchy(dir) = source {
        let weak = window.as_weak();
        // Not on Omarchy or no inotify: the theme stays as it started.
        let _ = theme_service::watch(dir, move |t| {
            let weak = weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(w) = weak.upgrade() {
                    ui::apply(&w, &t);
                }
            });
        });
    }
    window.run()
}
