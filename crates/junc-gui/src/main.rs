// SPDX-License-Identifier: GPL-3.0-or-later

//! JUNC graphical interface (Slint): starts the window, follows the
//! Omarchy theme and juncd live. The logic is in the library.

#![forbid(unsafe_code)]

use junc_gui::actions::{Actions, Command, Dbus};
use junc_gui::{app, daemon, meter_service, onboarding, theme, theme_service, ui};
use slint::ComponentHandle;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

fn main() -> Result<(), slint::PlatformError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(i) = args.iter().position(|a| a == "--scenario") {
        let name = args.get(i + 1).cloned().unwrap_or_default();
        return scenario(&name, &args);
    }
    let window = ui::AppWindow::new()?;
    let dir = theme::omarchy_dir();
    // Settings › Follow Omarchy theme (board 25), on unless turned off.
    let pref = onboarding::state_file()
        .and_then(|f| onboarding::flag(&f, "follow_theme"))
        .unwrap_or(true);
    let follow = Arc::new(AtomicBool::new(pref));
    let (tokens, _) =
        theme_service::current(dir.as_deref(), pref, theme_service::portal_prefers_light);
    ui::apply(&window, &tokens);
    window.global::<ui::View>().set_follow_theme(pref);
    if let Some(dir) = dir.clone() {
        let weak = window.as_weak();
        let follow = follow.clone();
        // Not on Omarchy or no inotify: the theme stays as it started.
        let _ = theme_service::watch(dir, theme_service::portal_prefers_light, move |t| {
            if !follow.load(Ordering::Relaxed) {
                return;
            }
            let weak = weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(w) = weak.upgrade() {
                    ui::apply(&w, &t);
                }
            });
        });
    }
    {
        let weak = window.as_weak();
        app::set_theme_switch(Box::new(move |on| {
            follow.store(on, Ordering::Relaxed);
            let (t, _) =
                theme_service::current(dir.as_deref(), on, theme_service::portal_prefers_light);
            if let Some(w) = weak.upgrade() {
                ui::apply(&w, &t);
            }
        }));
    }
    // The Omarchy language, followed live (architecture §8).
    let sources = junc_l10n::Sources::from_env(None);
    ui::set_language(&junc_l10n::resolve(&sources));
    let _ = junc_l10n::watch::spawn(sources, |lang| {
        let _ = slint::invoke_from_event_loop(move || {
            ui::set_language(&lang);
            app::refresh_current();
        });
    });
    let levels = meter_service::Levels::default();
    // Without PipeWire the window shows no levels.
    let meters = meter_service::start(&window, &levels).map(Rc::new);
    let actions: Rc<dyn Actions> = match Dbus::spawn(None) {
        Ok(d) => Rc::new(d),
        Err(_) => Rc::new(Nowhere),
    };
    let controller = app::install(&window, actions, meters.clone(), levels);
    if let Some(file) = onboarding::state_file() {
        controller.borrow_mut().onboard_with(file);
    }
    app::set_current(&controller);
    // Without a session bus the window shows juncd as unreachable.
    let _ = daemon::follow(None, move |d| {
        let _ = slint::invoke_from_event_loop(move || app::update_current(d));
    });
    let run = window.run();
    drop((controller, meters));
    run
}

/// Where requests go when there is no session bus to send them on.
struct Nowhere;

impl Actions for Nowhere {
    fn send(&self, _: Command) {}
}

/// The value after `flag`.
#[cfg(feature = "scenarios")]
fn arg<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).map(String::as_str)
}

/// `--scenario NAME [--theme light|dark] [--size WxH] [--lang L]
/// [--snapshot OUT.png] [--meters]` (architecture §7.5, §11.3): the window fed by a scenario
/// fixture through the mock IPC, or the component gallery (`gallery`). With
/// `--snapshot` it renders once offscreen with the bundled font and
/// writes a PNG; otherwise it opens a normal window (AT-SPI checks).
/// `--meters` meters the scenario's Junctions on the PipeWire it finds (the
/// hidden-window gate); snapshots never do.
#[cfg(feature = "scenarios")]
fn scenario(name: &str, args: &[String]) -> Result<(), slint::PlatformError> {
    use junc_gui::{scenario, snapshot};
    let fail = |e: String| slint::PlatformError::Other(e);
    let loaded = if name == "gallery" {
        None
    } else {
        Some(scenario::load(name).map_err(fail)?)
    };
    let (theme_name, size) = loaded
        .as_ref()
        .map_or(("dark".to_owned(), (1640, 1013)), |s| {
            (s.theme.clone(), s.size)
        });
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
    let lang = arg(args, "--lang").unwrap_or("en");
    let mut tokens = theme::fallback(light);
    if offscreen.is_some() {
        snapshot::FAMILY.clone_into(&mut tokens.font_family);
    }
    // Keep the window alive until rendered or closed.
    let (mut gallery, mut window, mut controller, mut meters) = (None, None, None, None);
    if let Some(s) = &loaded {
        let w = ui::AppWindow::new()?;
        ui::set_language(lang);
        ui::apply(&w, &tokens);
        let levels = meter_service::Levels::default();
        if shot.is_none() && args.iter().any(|a| a == "--meters") {
            meters = meter_service::start(&w, &levels).map(Rc::new);
        }
        let (app, mock) = scenario::open(s, &w, meters.clone(), levels);
        w.show()?;
        if let Some(o) = &offscreen {
            snapshot::settle(o, size.0, size.1);
            if !scenario::hover(s, &app) {
                return Err(fail(format!("{name}: hover {:?} is not a link", s.hover)));
            }
        } else {
            // A real window lays out on the event loop; hover after it.
            let (s, a) = (s.clone(), Rc::downgrade(&app));
            slint::Timer::single_shot(std::time::Duration::from_millis(300), move || {
                if let Some(a) = a.upgrade() {
                    scenario::hover(&s, &a);
                }
            });
        }
        controller = Some((app, mock));
        window = Some(w);
    } else {
        let w = ui::GalleryWindow::new()?;
        ui::set_language(lang);
        ui::apply_to(&w.global::<ui::Tokens>(), &tokens);
        w.show()?;
        gallery = Some(w);
    }
    match (offscreen, shot) {
        (Some(window), Some(path)) => {
            let rgb = snapshot::render(&window, size.0, size.1);
            snapshot::write_png(&path, size.0, size.1, &rgb).map_err(fail)?;
            drop((gallery, window, controller, meters));
            Ok(())
        }
        _ => {
            if let Some(w) = &window {
                w.window()
                    .set_size(slint::LogicalSize::new(size.0 as f32, size.1 as f32));
            }
            let run = slint::run_event_loop();
            drop((controller, meters));
            run
        }
    }
}

#[cfg(not(feature = "scenarios"))]
fn scenario(_: &str, _: &[String]) -> Result<(), slint::PlatformError> {
    Err(slint::PlatformError::Other(
        "this junc-gui was built without the scenarios feature".into(),
    ))
}
