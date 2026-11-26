// SPDX-License-Identifier: GPL-3.0-or-later

//! Onboarding, the empty states and the Add Input picker (design boards
//! 7–11) against their scenarios' mock IPC, on Slint's testing backend:
//! the keyboard and the pointer go through the system check and the
//! starting points, New Junction makes and selects a Junction, and the
//! picker adds what it offers, but never twice (F2, F24, UX onboarding).

use i_slint_backend_testing::{ElementHandle, mock_elapsed_time};
use junc_gui::checks::Row;
use junc_gui::meter_service::Levels;
use junc_gui::onboarding::Step;
use junc_gui::scenario::{self, Mock};
use junc_gui::theme;
use junc_gui::ui::{self, AppWindow, View};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize, Model, SharedString};
use std::cell::RefCell;
use std::rc::Rc;

type Opened = (AppWindow, Rc<RefCell<junc_gui::app::App>>, Mock);

fn open(name: &str) -> Opened {
    let s = scenario::load(name).expect("scenario");
    let w = AppWindow::new().expect("window");
    ui::set_language("en");
    ui::apply(&w, &theme::fallback(false));
    w.window().set_size(LogicalSize::new(1440.0, 900.0));
    let (app, mock) = scenario::open(&s, &w, None, Levels::default());
    w.show().expect("show");
    settle();
    (w, app, mock)
}

fn press(w: &AppWindow, k: impl Into<SharedString>) {
    w.window()
        .dispatch_event(WindowEvent::KeyPressed { text: k.into() });
}

fn release(w: &AppWindow, k: impl Into<SharedString>) {
    w.window()
        .dispatch_event(WindowEvent::KeyReleased { text: k.into() });
}

fn key(w: &AppWindow, k: impl Into<SharedString>) {
    let k = k.into();
    press(w, k.clone());
    release(w, k);
}

/// `k` with Ctrl (and Shift) held.
fn ctrl(w: &AppWindow, shift: bool, k: &str) {
    press(w, Key::Control);
    if shift {
        press(w, Key::Shift);
    }
    key(w, k);
    if shift {
        release(w, Key::Shift);
    }
    release(w, Key::Control);
}

fn settle() {
    mock_elapsed_time(std::time::Duration::from_millis(50));
}

fn has(w: &AppWindow, label: &str) -> bool {
    ElementHandle::find_by_accessible_label(w, label)
        .next()
        .is_some()
}

fn row(id: &str, state: &'static str) -> Row {
    Row {
        id: id.to_owned(),
        state,
        fix: String::new(),
    }
}

#[test]
fn onboarding_empty_states_and_the_picker_work_from_the_keyboard() {
    i_slint_backend_testing::init_no_event_loop();

    // Board 8: pipewire-pulse is missing, so Enter does not continue; its
    // fix can be copied.
    let (w, app, mock) = open("onboardCheck");
    let view = w.global::<View>();
    assert_eq!(app.borrow().step(), Some(Step::Check));
    assert!(!view.get_checks_passed());
    assert_eq!(view.get_checks().row_count(), 4);
    key(&w, "\n");
    settle();
    assert_eq!(app.borrow().step(), Some(Step::Check), "Continue waits");
    ElementHandle::find_by_accessible_label(&w, "Copy")
        .next()
        .expect("Copy")
        .invoke_accessible_default_action();
    settle();
    assert_eq!(
        mock.copied().as_deref(),
        Some("sudo pacman -S pipewire-pulse")
    );
    // Installed meanwhile: R checks again and Enter goes on.
    mock.set_checks(
        ["pipewire", "wireplumber", "pipewire-pulse", "jack"]
            .map(|id| row(id, "ok"))
            .to_vec(),
    );
    key(&w, "r");
    settle();
    assert!(view.get_checks_passed());
    key(&w, "\n");
    settle();
    assert_eq!(app.borrow().step(), Some(Step::Templates));
    // Board 9: Esc goes back, Enter forward again.
    key(&w, Key::Escape);
    settle();
    assert_eq!(app.borrow().step(), Some(Step::Check));
    key(&w, "\n");
    settle();
    assert_eq!(app.borrow().step(), Some(Step::Templates));
    // From "Mic + App" (second) to "Start empty" (sixth): down twice;
    // right at the right column stays put.
    for k in [Key::DownArrow, Key::DownArrow, Key::RightArrow] {
        key(&w, k);
    }
    assert!(has(&w, "Create Start empty"), "the button names the choice");
    key(&w, "\n");
    settle();
    // Board 11: a New Junction, selected, with both empty states.
    assert_eq!(app.borrow().step(), None, "onboarding is done");
    let c = mock.config().expect("config");
    assert_eq!(c.junction.len(), 1);
    assert_eq!(c.junction[0].name, "New Junction");
    assert_eq!(app.borrow().selected(), Some("new-junction"));
    assert!(view.get_empty_inputs() && view.get_empty_listens());
    assert!(has(&w, "No inputs yet") && has(&w, "Not listening"));
    drop((w, app, mock));

    // Board 10: no Junctions; Ctrl+Shift+J makes one and selects it.
    let (w, app, mock) = open("emptyNone");
    assert!(has(&w, "No Junctions yet"));
    ctrl(&w, true, "J");
    settle();
    assert_eq!(mock.config().expect("config").junction.len(), 1);
    assert_eq!(app.borrow().selected(), Some("new-junction"));
    drop((w, app, mock));

    // Board 7: Ctrl+N opens the picker; Enter adds the row under the
    // cursor and closes it.
    let (w, app, mock) = open("default");
    let view = w.global::<View>();
    ctrl(&w, false, "n");
    settle();
    assert!(view.get_picker_open());
    let items = view.get_picker_items();
    let texts: Vec<String> = (0..items.row_count())
        .filter_map(|i| items.row_data(i))
        .map(|i| i.text.to_string())
        .collect();
    let spotify = texts.iter().position(|t| t == "Spotify").expect("Spotify");
    let firefox = texts.iter().position(|t| t == "Firefox").expect("Firefox");
    assert!(
        items.row_data(spotify).expect("row").added,
        "Spotify feeds it already"
    );
    // An added row cannot be picked again.
    view.invoke_picker_picked(i32::try_from(spotify).expect("index"));
    settle();
    let inputs = |m: &Mock| m.config().expect("config").junction[0].input.len();
    assert_eq!(inputs(&mock), 3);
    // The search narrows the rows; Esc closes without adding.
    view.set_picker_search("fire".into());
    view.invoke_picker_search_edited("fire".into());
    assert_eq!(
        view.get_picker_items().row_count(),
        2,
        "Firefox and the Cable “Firefox → OBS”"
    );
    key(&w, Key::Escape);
    settle();
    assert!(!view.get_picker_open());
    assert_eq!(inputs(&mock), 3);
    // The "+" on Inputs opens it again; the cursor row is added.
    ElementHandle::find_by_accessible_label(&w, "Add input")
        .next()
        .expect("+ on Inputs")
        .invoke_accessible_default_action();
    settle();
    assert!(view.get_picker_open());
    view.set_picker_cursor(i32::try_from(firefox).expect("index"));
    key(&w, "\n");
    settle();
    assert!(!view.get_picker_open());
    let c = mock.config().expect("config");
    let added = c.junction[0].input.last().expect("input");
    assert_eq!(added.id, "firefox");
    assert_eq!(added.matcher.binary.as_deref(), Some("firefox"));
    assert_eq!(app.borrow().main().expect("main").inputs.len(), 4);
}
