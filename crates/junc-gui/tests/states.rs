// SPDX-License-Identifier: GPL-3.0-or-later

//! The Main screen's states of B18 (design boards 12–18, 30) against their
//! scenarios' mock IPC, on Slint's testing backend: what each shows and
//! that the keyboard and the pointer change the config the way juncd
//! would be asked to (F3, F4, F5, F6, F8, F11).

use i_slint_backend_testing::{ElementHandle, mock_elapsed_time};
use junc_gui::meter_service::Levels;
use junc_gui::scenario::{self, Mock};
use junc_gui::theme;
use junc_gui::ui::{self, AppWindow, View};
use slint::platform::WindowEvent;
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

fn key(w: &AppWindow, k: impl Into<SharedString>) {
    let text: SharedString = k.into();
    w.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.window().dispatch_event(WindowEvent::KeyReleased { text });
}

fn settle() {
    mock_elapsed_time(std::time::Duration::from_millis(50));
}

fn has(w: &AppWindow, label: &str) -> bool {
    ElementHandle::find_by_accessible_label(w, label)
        .next()
        .is_some()
}

fn on(mock: &Mock) -> bool {
    mock.config().expect("config").junction[0].enabled
}

#[test]
fn the_main_screen_states_work_from_the_keyboard_and_pointer() {
    i_slint_backend_testing::init_no_event_loop();

    // Board 18: off, a banner says so, nothing is linked; Space turns it
    // back on and the links return.
    let (w, app, mock) = open("junctionOff");
    let view = w.global::<View>();
    assert!(!view.get_junction_on());
    assert!(has(&w, "Junction is off — apps see silence"));
    assert_eq!(view.get_links().row_count(), 0);
    assert!(
        app.borrow().main().expect("main").inputs.len() == 3,
        "its cards stay"
    );
    key(&w, " ");
    settle();
    assert!(on(&mock));
    assert!(view.get_junction_on());
    assert!(!has(&w, "Junction is off — apps see silence"));
    assert!(view.get_links().row_count() > 0);
    // Off from its sidebar switch, on again from the banner's button.
    view.invoke_junction_toggled("virtual-mic-for-zoom".into(), false);
    settle();
    assert!(!on(&mock) && !view.get_junction_on());
    ElementHandle::find_by_accessible_label(&w, "Turn on")
        .next()
        .expect("Turn on")
        .invoke_accessible_default_action();
    settle();
    assert!(on(&mock));
}
