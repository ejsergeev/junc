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

fn key(w: &AppWindow, k: impl Into<SharedString>) {
    let text: SharedString = k.into();
    w.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.window().dispatch_event(WindowEvent::KeyReleased { text });
}

/// `k` with Ctrl (and Shift) held.
fn ctrl_shift(w: &AppWindow, shift: bool, k: &str) {
    let hold = |t: SharedString, down: bool| {
        w.window().dispatch_event(if down {
            WindowEvent::KeyPressed { text: t }
        } else {
            WindowEvent::KeyReleased { text: t }
        });
    };
    hold(Key::Control.into(), true);
    if shift {
        hold(Key::Shift.into(), true);
    }
    key(w, k);
    if shift {
        hold(Key::Shift.into(), false);
    }
    hold(Key::Control.into(), false);
}

fn ctrl(w: &AppWindow, k: &str) {
    ctrl_shift(w, false, k);
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
    drop((w, app, mock));

    // Board 17: Ctrl+M opens the Add Listen output picker; added outputs
    // are refused; Enter adds the default output; Esc closes.
    let (w, _app, mock) = open("addListen");
    let view = w.global::<View>();
    assert!(view.get_picker_open() && view.get_picker_listen());
    key(&w, Key::Escape);
    settle();
    assert!(!view.get_picker_open());
    ctrl(&w, "m");
    settle();
    assert!(view.get_picker_open() && view.get_picker_listen());
    let items = view.get_picker_items();
    let texts: Vec<String> = (0..items.row_count())
        .filter_map(|i| items.row_data(i))
        .map(|i| i.text.to_string())
        .collect();
    assert_eq!(
        texts,
        [
            "Default output (follows system)",
            "Built-in Audio Analog Stereo",
            "HDMI Output"
        ]
    );
    let listens = |m: &Mock| m.config().expect("config").junction[0].listen.clone();
    view.invoke_picker_picked(2);
    settle();
    assert_eq!(listens(&mock).len(), 2, "HDMI Output is heard already");
    key(&w, "\n");
    settle();
    assert!(!view.get_picker_open());
    let l = listens(&mock);
    assert_eq!(l.len(), 3);
    assert_eq!(
        (l[2].id.as_str(), l[2].target.as_str()),
        ("default", "default")
    );
    drop((w, _app, mock));

    // Board 11: the empty Listen column's button opens it too.
    let (w, _app, mock) = open("emptyJunction");
    let view = w.global::<View>();
    ElementHandle::find_by_accessible_label(&w, "Add listen output")
        .next()
        .expect("the empty state's button")
        .invoke_accessible_default_action();
    settle();
    assert!(view.get_picker_open() && view.get_picker_listen());
    key(&w, "\n");
    settle();
    let c = mock.config().expect("config");
    let j = c
        .junction
        .iter()
        .find(|j| j.id == "new-junction")
        .expect("selected");
    assert_eq!(j.listen.len(), 1);
    drop((w, _app, mock));

    // Board 16: Enter adds the stereo pair; Ctrl+Shift+N opens it again,
    // ↓ picks mono; Esc and a click outside add nothing.
    let (w, _app, mock) = open("addBus");
    let view = w.global::<View>();
    let channels = |m: &Mock| m.config().expect("config").junction[0].bus.channels;
    assert!(view.get_adding_bus());
    assert!(has(&w, "Apps that pick this Junction see 6 channels."));
    key(&w, "\n");
    settle();
    assert!(!view.get_adding_bus());
    assert_eq!(channels(&mock), 6);
    ctrl_shift(&w, true, "N");
    settle();
    assert!(view.get_adding_bus());
    key(&w, Key::DownArrow);
    assert!(has(&w, "Apps that pick this Junction see 7 channels."));
    key(&w, "\n");
    settle();
    assert_eq!(channels(&mock), 7);
    ElementHandle::find_by_accessible_label(&w, "Add Bus channels")
        .next()
        .expect("+ on Bus")
        .invoke_accessible_default_action();
    settle();
    assert!(view.get_adding_bus());
    key(&w, Key::Escape);
    settle();
    assert!(!view.get_adding_bus());
    assert_eq!(channels(&mock), 7);
}
