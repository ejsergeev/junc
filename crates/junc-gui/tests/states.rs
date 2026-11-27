// SPDX-License-Identifier: GPL-3.0-or-later

//! The Main screen's states of B18 and B19 (design boards 12–18, 20–23, 26, 27, 30)
//! against their scenarios' mock IPC, on Slint's testing backend: what
//! each shows and that the keyboard and the pointer change the config the
//! way juncd would be asked to (F3, F4, F5, F6, F8, F11).

use i_slint_backend_testing::{ElementHandle, mock_elapsed_time};
use junc_gui::meter_service::Levels;
use junc_gui::scenario::{self, Mock};
use junc_gui::theme;
use junc_gui::ui::{self, AppWindow, View};
use slint::platform::PointerEventButton;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, Model, SharedString};
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

fn click(w: &AppWindow, e: &ElementHandle) {
    let (p, s) = (e.absolute_position(), e.size());
    let position = LogicalPosition::new(p.x + s.width / 2.0, p.y + 10.0);
    w.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    for down in [true, false] {
        w.window().dispatch_event(if down {
            WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            }
        } else {
            WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            }
        });
    }
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
    drop((w, _app, mock));

    // Board 15: Spotify's Options. Exclusive off, channel 1 to Bus 3 from
    // its select (↓ twice), Esc closes, Options opens it again, Del
    // removes the Input.
    let (w, app, mock) = open("inputOptions");
    let view = w.global::<View>();
    let spotify = |m: &Mock| {
        m.config().expect("config").junction[0]
            .input
            .iter()
            .find(|i| i.id == "spotify")
            .cloned()
    };
    assert!(view.get_options_open());
    assert!(has(&w, "Spotify options"));
    let feeds: Vec<i32> = (0..view.get_options_feeds().row_count())
        .filter_map(|i| view.get_options_feeds().row_data(i))
        .collect();
    assert_eq!(feeds, [1, 2]);
    ElementHandle::find_by_accessible_label(&w, "Exclusive")
        .find(|e| e.accessible_checkable() == Some(true))
        .expect("the Exclusive switch")
        .invoke_accessible_default_action();
    settle();
    assert!(!spotify(&mock).expect("spotify").exclusive);
    let select = ElementHandle::find_by_accessible_label(&w, "Channel 1 feeds")
        .next()
        .expect("channel 1's select");
    click(&w, &select);
    assert_eq!(select.accessible_expanded(), Some(true));
    key(&w, Key::DownArrow);
    key(&w, Key::DownArrow);
    key(&w, "\n");
    settle();
    assert_eq!(
        spotify(&mock).expect("spotify").links,
        Some(vec![[1, 3], [2, 2]])
    );
    assert_eq!(select.accessible_value().as_deref(), Some("Ch 3"));
    key(&w, Key::Escape);
    settle();
    assert!(!view.get_options_open());
    view.invoke_options_toggled(0);
    settle();
    assert!(view.get_options_open());
    key(&w, Key::Delete);
    settle();
    assert!(!view.get_options_open());
    assert!(spotify(&mock).is_none(), "removed");
    assert_eq!(app.borrow().main().expect("main").inputs.len(), 2);
    drop((w, app, mock));

    // Board 12: a Cable. Exclusive and Listen from their switches, the
    // Listen output from its select, the apps from Change app….
    let (w, _app, mock) = open("cable");
    let view = w.global::<View>();
    assert!(view.get_cable());
    assert_eq!(
        (view.get_cable_from().as_str(), view.get_cable_to().as_str()),
        ("Firefox", "OBS")
    );
    let cable = |m: &Mock| {
        m.config()
            .expect("config")
            .junction
            .into_iter()
            .find(|j| j.id == "firefox-to-obs")
            .expect("the Cable")
    };
    let switch = |label: &str| {
        ElementHandle::find_by_accessible_label(&w, label)
            .find(|e| e.accessible_checkable() == Some(true))
            .unwrap_or_else(|| panic!("the {label} switch"))
    };
    switch("Exclusive").invoke_accessible_default_action();
    settle();
    assert!(cable(&mock).input[0].exclusive);
    switch("Listen").invoke_accessible_default_action();
    settle();
    let l = cable(&mock).listen;
    assert_eq!((l.len(), l[0].target.as_str()), (1, "default"));
    assert!(view.get_cable_listen());
    let outputs: Vec<String> = (0..view.get_cable_outputs().row_count())
        .filter_map(|i| view.get_cable_outputs().row_data(i))
        .map(|s| s.to_string())
        .collect();
    let hdmi = outputs
        .iter()
        .position(|o| o == "HDMI Output")
        .expect("HDMI Output");
    view.invoke_cable_listen_target(i32::try_from(hdmi).expect("index"));
    settle();
    assert_eq!(
        cable(&mock).listen[0].target,
        "alsa_output.platform-hdmi.hdmi-stereo"
    );
    // From: the playing apps, Firefox taken; pick Spotify.
    let change: Vec<ElementHandle> =
        ElementHandle::find_by_accessible_label(&w, "Change app…").collect();
    assert_eq!(change.len(), 2);
    change[0].invoke_accessible_default_action();
    settle();
    assert!(view.get_picker_open());
    assert_eq!(view.get_picker_cable(), 1);
    let items = view.get_picker_items();
    let rows: Vec<(String, bool)> = (0..items.row_count())
        .filter_map(|i| items.row_data(i))
        .map(|i| (i.text.to_string(), i.added))
        .collect();
    assert!(rows.contains(&("Firefox".to_owned(), true)));
    let spotify = rows.iter().position(|r| r.0 == "Spotify").expect("Spotify");
    view.invoke_picker_picked(i32::try_from(spotify).expect("index"));
    settle();
    assert!(!view.get_picker_open());
    assert_eq!(
        cable(&mock).input[0].matcher.binary.as_deref(),
        Some("spotify")
    );
    // To: the recording apps; OBS is the one it has. Esc closes.
    change[1].invoke_accessible_default_action();
    settle();
    assert_eq!(view.get_picker_cable(), 2);
    assert_eq!(view.get_picker_items().row_count(), 1);
    key(&w, Key::Escape);
    settle();
    assert!(!view.get_picker_open());
    drop((w, _app, mock));

    // Boards 13, 14: links from Soundux. Drag channel 1 onto Bus 1; then
    // Enter on channel 2, ↓ to the second target (Bus 2), Enter; Enter,
    // Esc makes nothing.
    let (w, _app, mock) = open("default");
    let view = w.global::<View>();
    let soundux = |m: &Mock| {
        m.config().expect("config").junction[0]
            .input
            .iter()
            .find(|i| i.id == "soundux")
            .and_then(|i| i.links.clone())
    };
    let centre = |label: &str| {
        let e = ElementHandle::find_by_accessible_label(&w, label)
            .next()
            .unwrap_or_else(|| panic!("{label}"));
        let (p, s) = (e.absolute_position(), e.size());
        LogicalPosition::new(p.x + s.width / 2.0, p.y + s.height / 2.0)
    };
    let (from, to) = (centre("Soundux channel 1"), centre("Bus channel 1 in"));

    w.window()
        .dispatch_event(WindowEvent::PointerMoved { position: from });
    w.window().dispatch_event(WindowEvent::PointerPressed {
        position: from,
        button: PointerEventButton::Left,
    });
    for t in [0.3, 0.7, 1.0] {
        let position =
            LogicalPosition::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
        w.window()
            .dispatch_event(WindowEvent::PointerMoved { position });
    }
    assert_eq!(view.get_linking(), 1);
    assert!(view.get_tip_visible(), "the tip names the link");
    w.window().dispatch_event(WindowEvent::PointerReleased {
        position: to,
        button: PointerEventButton::Left,
    });
    settle();
    assert_eq!(view.get_linking(), 0);
    assert_eq!(soundux(&mock), Some(vec![[1, 1], [1, 3], [2, 4]]));
    // A click focuses a port and links nothing.
    let port = ElementHandle::find_by_accessible_label(&w, "Soundux channel 2")
        .next()
        .expect("Soundux channel 2");
    click(&w, &port);
    settle();
    assert_eq!(view.get_linking(), 0);
    key(&w, "\n");
    assert_eq!(view.get_linking(), 2);
    key(&w, Key::DownArrow);
    key(&w, "\n");
    settle();
    assert_eq!(view.get_linking(), 0);
    assert_eq!(soundux(&mock), Some(vec![[1, 1], [1, 3], [2, 2], [2, 4]]));
    key(&w, "\n");
    assert_eq!(view.get_linking(), 2);
    key(&w, Key::Escape);
    settle();
    assert_eq!(view.get_linking(), 0);
    assert_eq!(soundux(&mock).map(|l| l.len()), Some(4));
    drop((w, _app, mock));

    // §7.4: Ctrl+2 selects the second Junction, Ctrl+1 the first; the
    // arrows move between cards (Space then toggles the one reached); Del
    // with the pointer on a link unlinks it.
    let (w, app, mock) = open("default");
    let view = w.global::<View>();
    ctrl(&w, "2");
    settle();
    assert_eq!(app.borrow().selected(), Some("firefox-to-obs"));
    ctrl(&w, "9");
    assert_eq!(app.borrow().selected(), Some("firefox-to-obs"), "no ninth");
    ctrl(&w, "1");
    settle();
    assert_eq!(app.borrow().selected(), Some("virtual-mic-for-zoom"));
    let config = |m: &Mock| m.config().expect("config").junction[0].clone();
    let spotify = ElementHandle::find_by_accessible_label(&w, "Spotify")
        .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Groupbox))
        .expect("the Spotify card");
    click(&w, &spotify);
    key(&w, Key::DownArrow);
    key(&w, " ");
    settle();
    assert!(!config(&mock).input[1].enabled, "↓ to Soundux, Space");
    key(&w, Key::RightArrow);
    key(&w, Key::RightArrow);
    key(&w, " ");
    settle();
    assert!(
        !config(&mock)
            .listen
            .iter()
            .find(|l| l.id == "hdmi")
            .expect("hdmi")
            .enabled,
        "→ → past the Bus to HDMI Output, Space"
    );
    let point = {
        let a = app.borrow();
        let (m, l) = (a.main().expect("main"), a.layout().expect("layout"));
        let i = m
            .links
            .iter()
            .position(|k| k.owner == "builtin" && k.pair == [1, 1])
            .expect("Ch 1 → Built-in 1");
        l.curves[i].expect("drawn").at(0.5)
    };
    view.invoke_pointer_moved(point.x, point.y);
    assert!(view.get_tip_visible());
    key(&w, Key::Delete);
    settle();
    let builtin = config(&mock)
        .listen
        .into_iter()
        .find(|l| l.id == "builtin")
        .expect("builtin");
    assert_eq!(builtin.links, Some(vec![[2, 2]]));
    drop((w, app, mock));

    // Board 26: F2 opens the name selected; typing replaces it, Enter
    // saves. F2 again, Esc: the name stays. The pencil opens it too.
    let (w, _app, mock) = open("default");
    let view = w.global::<View>();
    let name = |m: &Mock| m.config().expect("config").junction[0].name.clone();
    key(&w, Key::F2);
    settle();
    assert!(view.get_renaming());
    assert!(has(&w, "Name"));
    for c in "Zoom Mic".chars() {
        key(&w, c.to_string());
    }
    key(&w, "\n");
    settle();
    assert!(!view.get_renaming());
    assert_eq!(name(&mock), "Zoom Mic");
    key(&w, Key::F2);
    settle();
    key(&w, "x");
    key(&w, Key::Escape);
    settle();
    assert!(!view.get_renaming());
    assert_eq!(name(&mock), "Zoom Mic");
    ElementHandle::find_by_accessible_label(&w, "Rename")
        .next()
        .expect("pencil")
        .invoke_accessible_default_action();
    settle();
    assert!(view.get_renaming());
    drop((w, _app, mock));

    // Board 22: what goes with it and what stays; Esc keeps it, Ctrl+Delete
    // asks again, Ctrl+Enter deletes it.
    let (w, _app, mock) = open("deleteConfirm");
    let view = w.global::<View>();
    let count = |m: &Mock| m.config().expect("config").junction.len();
    assert!(view.get_confirming());
    assert!(has(&w, "Delete “Virtual Mic for Zoom”?"));
    assert!(has(&w, "3 Inputs · 12 Links"));
    assert!(has(&w, "Bus Ch 1–2, Ch 3–4 · 2 Listen outputs"));
    assert!(has(
        &w,
        "Spotify, Soundux and USB Microphone keep playing as before."
    ));
    key(&w, Key::Escape);
    settle();
    assert!(!view.get_confirming());
    assert_eq!(count(&mock), 2);
    ctrl(&w, &SharedString::from(Key::Delete));
    settle();
    assert!(view.get_confirming());
    ctrl(&w, "\n");
    settle();
    assert_eq!(count(&mock), 1);
    drop((w, _app, mock));

    // Board 21: the menu on Duplicate, Enter copies the Junction after it;
    // the Menu key opens it again, ↓ ↓ Enter turns it off; Esc closes it;
    // Ctrl+D copies it too.
    let (w, _app, mock) = open("contextMenu");
    let view = w.global::<View>();
    let names = |m: &Mock| -> Vec<String> {
        m.config()
            .expect("config")
            .junction
            .iter()
            .map(|j| j.name.clone())
            .collect()
    };
    assert_eq!(view.get_menu_index(), 0);
    assert!(has(&w, "Export preset…"));
    key(&w, "\n");
    settle();
    assert_eq!(view.get_menu_index(), -1);
    assert_eq!(
        names(&mock)[..2],
        ["Virtual Mic for Zoom", "Virtual Mic for Zoom copy"]
    );
    key(&w, Key::Menu);
    settle();
    assert_eq!(view.get_menu_index(), 0);
    key(&w, Key::DownArrow);
    key(&w, Key::DownArrow);
    key(&w, "\n");
    settle();
    assert!(!on(&mock));
    key(&w, Key::Menu);
    settle();
    key(&w, Key::Escape);
    settle();
    assert_eq!(view.get_menu_index(), -1);
    ctrl(&w, "d");
    settle();
    assert_eq!(names(&mock).len(), 4);
    drop((w, _app, mock));

    // Board 23: Spotify quit; its card and the counts wait for it.
    let (w, _app, _mock) = open("appClosed");
    assert_eq!(w.global::<View>().get_waiting(), 1);
    assert!(has(&w, "Waiting for Spotify…"));
    assert!(has(&w, "· 1 waiting"));
    drop((w, _app, _mock));

    // Board 27: a New Cable stays in the window until both apps are
    // chosen: Enter picks Discord, the To picker opens, Enter picks OBS
    // and the Cable is written and selected.
    let (w, app, mock) = open("newCable");
    let view = w.global::<View>();
    assert_eq!(app.borrow().selected(), Some(junc_gui::app::DRAFT));
    assert!(view.get_cable_draft());
    assert!(has(&w, "No app yet"));
    assert_eq!(mock.config().expect("config").junction.len(), 2);
    assert_eq!(view.get_picker_cable(), 1);
    key(&w, "\n");
    settle();
    assert!(view.get_picker_open());
    assert_eq!(view.get_picker_cable(), 2);
    key(&w, "\n");
    settle();
    let config = mock.config().expect("config");
    let cable = config.junction.last().expect("cable");
    assert_eq!(cable.name, "New Cable");
    assert_eq!(cable.input[0].matcher.binary.as_deref(), Some("discord"));
    assert_eq!(
        cable.cable.as_ref().and_then(|c| c.to.binary.as_deref()),
        Some("obs")
    );
    assert_eq!(app.borrow().selected(), Some("new-cable"));
    assert!(!view.get_cable_draft());
    drop((w, app, mock));

    // Ctrl+Shift+K makes one; Delete drops it without asking juncd.
    let (w, app, mock) = open("default");
    ctrl_shift(&w, true, "K");
    settle();
    assert_eq!(app.borrow().selected(), Some(junc_gui::app::DRAFT));
    key(&w, Key::Escape);
    settle();
    w.global::<View>().invoke_delete_junction();
    settle();
    assert_ne!(app.borrow().selected(), Some(junc_gui::app::DRAFT));
    assert_eq!(mock.config().expect("config").junction.len(), 2);

    // The New button's menu makes a Cable too.
    ElementHandle::find_by_accessible_label(&w, "New")
        .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Button))
        .expect("New")
        .invoke_accessible_default_action();
    settle();
    key(&w, Key::DownArrow);
    key(&w, "\n");
    settle();
    assert_eq!(app.borrow().selected(), Some(junc_gui::app::DRAFT));
    drop((w, app, mock));

    // Board 20: PipeWire does not answer. Doctor's rows show; L opens
    // juncd's journal; Enter checks again.
    let (w, _app, mock) = open("serviceDown");
    let view = w.global::<View>();
    assert!(has(&w, "Can't reach the audio service"));
    assert!(has(&w, "Audio service down"));
    assert_eq!(view.get_service_checks().row_count(), 2);
    key(&w, "l");
    assert!(mock.logs_opened());
    key(&w, "\n");
    assert!(view.get_checking());
    settle();
    assert!(!view.get_checking());
}
