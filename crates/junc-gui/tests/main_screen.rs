// SPDX-License-Identifier: GPL-3.0-or-later

//! The Main screen against the default scenario's mock IPC, on Slint's
//! testing backend (no display): what it shows to assistive technology,
//! and that the keyboard, the pointer and the switches change the config
//! the way juncd would be asked to (F4, F8).

use i_slint_backend_testing::{AccessibleRole, ElementHandle, mock_elapsed_time};
use junc_gui::layout::Point;
use junc_gui::meter_service::Levels;
use junc_gui::scenario::{self, Mock};
use junc_gui::theme;
use junc_gui::ui::{self, AppWindow, View};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, SharedString};

fn one(w: &AppWindow, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(w, label)
        .next()
        .unwrap_or_else(|| panic!("no element labelled {label:?}"))
}

fn key(w: &AppWindow, k: impl Into<SharedString>) {
    let text: SharedString = k.into();
    w.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.window().dispatch_event(WindowEvent::KeyReleased { text });
}

fn click(w: &AppWindow, e: &ElementHandle) {
    let (p, s) = (e.absolute_position(), e.size());
    let position = LogicalPosition::new(p.x + s.width / 2.0, p.y + s.height / 2.0);
    w.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    w.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    w.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

/// Lets the mock IPC answer and the window lay out.
fn settle() {
    mock_elapsed_time(std::time::Duration::from_millis(50));
}

fn links(mock: &Mock, input: &str) -> Option<Vec<[u8; 2]>> {
    let c = mock.config().expect("config");
    c.junction[0]
        .input
        .iter()
        .find(|i| i.id == input)
        .and_then(|i| i.links.clone())
}

#[test]
fn the_main_screen_routes_from_keyboard_pointer_and_switches() {
    i_slint_backend_testing::init_no_event_loop();
    let s = scenario::load("default").expect("scenario");
    let w = AppWindow::new().expect("window");
    ui::apply(&w, &theme::fallback(false));
    w.window().set_size(LogicalSize::new(1440.0, 900.0));
    let (app, mock) = scenario::open(&s, &w, None, Levels::default());
    w.show().expect("show");
    settle();

    // What a screen reader finds: cards as groups, ports as buttons that
    // say whether they are linked, meters hidden.
    let spotify = one(&w, "Spotify");
    assert_eq!(spotify.accessible_role(), Some(AccessibleRole::Groupbox));
    let port = one(&w, "Spotify channel 1");
    assert_eq!(port.accessible_role(), Some(AccessibleRole::Button));
    assert_eq!(port.accessible_description().as_deref(), Some("linked"));
    assert_eq!(
        one(&w, "Bus channel 3 in")
            .accessible_description()
            .as_deref(),
        Some("linked"),
        "Soundux feeds Ch 3"
    );
    assert!(app.borrow().layout().is_some(), "the canvas is laid out");

    // Space on a focused card turns the Input off (F8): its links go and
    // the Bus channels it fed are free.
    let soundux = one(&w, "Soundux");
    click(&w, &soundux);
    key(&w, " ");
    settle();
    let c = mock.config().expect("config");
    assert!(!c.junction[0].input[1].enabled, "Soundux is off");
    assert_eq!(
        one(&w, "Bus channel 3 in")
            .accessible_description()
            .as_deref(),
        Some(""),
        "Ch 3 is free"
    );
    // Its switch turns it back on.
    let switch = ElementHandle::find_by_accessible_label(&w, "Soundux")
        .find(|e| e.accessible_role() == Some(AccessibleRole::Switch))
        .expect("switch");
    assert_eq!(switch.accessible_checked(), Some(false));
    switch.invoke_accessible_default_action();
    settle();
    assert!(mock.config().expect("config").junction[0].input[1].enabled);

    // A click on a link unlinks that pair only (F4): the default mapping
    // becomes the Input's own list, less the pair.
    let point = {
        let a = app.borrow();
        let (m, l) = (a.main().expect("main"), a.layout().expect("layout"));
        let i = m
            .links
            .iter()
            .position(|k| k.owner == "mic" && k.pair == [1, 1])
            .expect("mic 1 -> 1");
        l.curves[i].expect("curve").at(0.5)
    };
    let view = w.global::<View>();
    view.invoke_pointer_moved(point.x, point.y);
    assert!(view.get_tip_visible(), "the tooltip names the link");
    assert_eq!(view.get_tip_owner(), "USB Microphone");
    view.invoke_link_clicked();
    settle();
    assert_eq!(links(&mock, "mic"), Some(vec![[2, 2]]));
    view.invoke_pointer_moved(Point { x: 0.0, y: 0.0 }.x, 0.0);
    assert!(!view.get_tip_visible());

    // Del on a focused port removes every link at it.
    let port = one(&w, "Soundux channel 2");
    click(&w, &port);
    key(&w, Key::Delete);
    settle();
    assert_eq!(links(&mock, "soundux"), Some(vec![[1, 3]]));

    // Show Listen off: two columns, no Listen cards.
    let listen = ElementHandle::find_by_accessible_label(&w, "Show Listen")
        .next()
        .expect("Show Listen");
    listen.invoke_accessible_default_action();
    settle();
    assert!(app.borrow().layout().expect("layout").listens.is_empty());
    assert!(
        ElementHandle::find_by_accessible_label(&w, "HDMI Output")
            .next()
            .is_none()
    );
}
