// SPDX-License-Identifier: GPL-3.0-or-later

//! JUNC graphical interface (Slint): starts the window, follows the
//! Omarchy theme and juncd live. The logic is in the library.

#![forbid(unsafe_code)]

use junc_gui::{daemon, state, theme, theme_service, ui};
use slint::ComponentHandle;

fn main() -> Result<(), slint::PlatformError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(i) = args.iter().position(|a| a == "--scenario") {
        let name = args.get(i + 1).cloned().unwrap_or_default();
        return scenario(&name, &args);
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

/// The value after `flag`.
#[cfg(feature = "scenarios")]
fn arg<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).map(String::as_str)
}

/// `--scenario NAME [--theme light|dark] [--size WxH] [--snapshot OUT.png]`
/// (architecture §7.5, §11.3): the window fed by a scenario fixture
/// through the mock IPC, or the component gallery (`gallery`). With
/// `--snapshot` it renders once offscreen with the bundled font and
/// writes a PNG; otherwise it opens a normal window (AT-SPI checks).
#[cfg(feature = "scenarios")]
fn scenario(name: &str, args: &[String]) -> Result<(), slint::PlatformError> {
    use junc_gui::{scenario, snapshot};
    let fail = |e: String| slint::PlatformError::Other(e);
    let (theme_name, size, describe) = if name == "gallery" {
        ("dark".to_owned(), (1640, 1013), None)
    } else {
        let s = scenario::load(name).map_err(fail)?;
        (s.theme, s.size, s.describe)
    };
    let size = arg(args, "--size")
        .and_then(|v| v.split_once('x'))
        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
        .unwrap_or(size);
    let light = arg(args, "--theme").unwrap_or(&theme_name) == "light";
    let shot = arg(args, "--snapshot").map(std::path::PathBuf::from);
    let offscreen = match &shot {
        Some(_) => Some(snapshot::install(size.0, size.1)?),
        None => None,
    };
    let mut tokens = theme::fallback(light);
    if offscreen.is_some() {
        snapshot::FAMILY.clone_into(&mut tokens.font_family);
    }
    // Keep the window alive until rendered or closed.
    let (gallery, app);
    if name == "gallery" {
        let w = ui::GalleryWindow::new()?;
        ui::apply_to(&w.global::<ui::Tokens>(), &tokens);
        w.show()?;
        gallery = Some(w);
        app = None;
    } else {
        let w = ui::AppWindow::new()?;
        ui::apply(&w, &tokens);
        ui::show(&w, &state::snapshot(describe.as_ref()));
        w.show()?;
        app = Some(w);
        gallery = None;
    }
    match (offscreen, shot) {
        (Some(window), Some(path)) => {
            let rgb = snapshot::render(&window, size.0, size.1);
            snapshot::write_png(&path, size.0, size.1, &rgb).map_err(fail)?;
            drop((gallery, app));
            Ok(())
        }
        _ => {
            if let Some(w) = &app {
                w.window()
                    .set_size(slint::LogicalSize::new(size.0 as f32, size.1 as f32));
            }
            slint::run_event_loop()
        }
    }
}

#[cfg(not(feature = "scenarios"))]
fn scenario(_: &str, _: &[String]) -> Result<(), slint::PlatformError> {
    Err(slint::PlatformError::Other(
        "this junc-gui was built without the scenarios feature".into(),
    ))
}
