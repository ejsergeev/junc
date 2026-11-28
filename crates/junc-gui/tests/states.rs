// SPDX-License-Identifier: GPL-3.0-or-later

//! The Main screen's states of B18 and B19 (design boards 12–18, 20–27, 30, 31, 35–43, 46–49)
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
    open_in(name, Some("en"))
}

/// `name` in `lang`, or in the language already chosen (`None`).
fn open_in(name: &str, lang: Option<&str>) -> Opened {
    let s = scenario::load(name).expect("scenario");
    let w = AppWindow::new().expect("window");
    if let Some(lang) = lang {
        ui::set_language(lang);
    }
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
    drop((w, _app, mock));

    // Board 25: Settings asks systemd; the switch enables or disables
    // the unit; Esc leaves; Ctrl+, opens it again, a sidebar row leaves.
    let (w, _app, mock) = open("settings");
    let view = w.global::<View>();
    assert!(view.get_settings_open());
    assert!(view.get_service_known());
    assert!(view.get_start_at_login());
    view.invoke_start_at_login_changed(false);
    settle();
    assert!(!mock.start_at_login());
    assert!(!view.get_start_at_login());
    view.invoke_follow_theme_changed(false);
    assert!(!view.get_follow_theme());
    key(&w, Key::Escape);
    settle();
    assert!(!view.get_settings_open());
    ctrl(&w, ",");
    settle();
    assert!(view.get_settings_open());
    view.invoke_select("firefox-to-obs".into());
    settle();
    assert!(!view.get_settings_open());
    drop((w, _app, mock));

    // Board 24: two setups start checked, the loopback as a Cable; the
    // unconnected one is checked by hand, the Cable made a Junction, Enter
    // imports three; Esc imports nothing.
    let (w, _app, mock) = open("import");
    let view = w.global::<View>();
    let count = |m: &Mock| m.config().expect("config").junction.len();
    assert!(view.get_import_open());
    assert_eq!(view.get_import_count(), 2);
    assert!(has(&w, "Import 2"));
    let items = view.get_import_items();
    assert!(items.row_data(1).expect("loopback").as_cable);
    assert!(!items.row_data(2).expect("monitor").checked);
    view.invoke_import_toggled(2);
    view.invoke_import_cable(1, false);
    view.invoke_import_cable(0, true);
    assert!(
        !items.row_data(0).expect("sink").as_cable,
        "a sink cannot be a Cable"
    );
    assert_eq!(view.get_import_count(), 3);
    key(&w, "\n");
    settle();
    assert!(!view.get_import_open());
    let names: Vec<String> = mock
        .config()
        .expect("config")
        .junction
        .iter()
        .map(|j| j.name.clone())
        .collect();
    assert_eq!(
        names[2..],
        ["discord_mix", "firefox → obs", "stream_monitor"]
    );
    view.invoke_open_import();
    settle();
    key(&w, Key::Escape);
    settle();
    assert!(!view.get_import_open());
    assert_eq!(count(&mock), 5);
    drop((w, _app, mock));

    // Board 31: Follow Omarchy off opens the picker: English first, then
    // the 46 languages; typing filters, Enter shows the UI in German; the
    // switch back on follows Omarchy (English here).
    let (w, _app, _mock) = open("langPicker");
    let view = w.global::<View>();
    assert!(!view.get_follow_language());
    assert!(view.get_lang_open());
    assert_eq!(view.get_lang_items().row_count(), 46);
    assert_eq!(
        view.get_lang_items().row_data(0).expect("current").text,
        "English"
    );
    for c in "deu".chars() {
        key(&w, c.to_string());
    }
    settle();
    assert_eq!(view.get_lang_items().row_count(), 1);
    key(&w, "\n");
    settle();
    assert!(!view.get_lang_open());
    assert_eq!(view.get_language_name(), "Deutsch");
    assert!(has(&w, "Einstellungen"));
    view.invoke_follow_language_changed(true);
    settle();
    assert!(view.get_follow_language());
    assert_eq!(view.get_language_name(), "English");
    assert!(has(&w, "Settings"));

    // Board 32: Omarchy set to Arabic shows English until RTL (PRD).
    let sources = junc_l10n::Sources {
        explicit: None,
        omarchy_file: None,
        env: vec!["ar_EG.UTF-8".to_owned()],
    };
    ui::set_language(&junc_l10n::resolve(&sources));
    let (w, _app, _mock) = open_in("default", None);
    assert!(has(&w, "Rename"), "Arabic shows English");
    drop((w, _app, _mock));

    // Board 37: Virtual Mic for Zoom is the default input; ↓ Space makes
    // it the default output instead, Space again none; Esc closes.
    let (w, _app, mock) = open("useDefault");
    let view = w.global::<View>();
    let role = |m: &Mock| m.config().expect("config").junction[0].default;
    assert_eq!(view.get_default_role(), "input");
    assert!(view.get_defaults_open());
    assert_eq!(role(&mock), junc_core::model::DefaultRole::Input);
    key(&w, Key::DownArrow);
    key(&w, " ");
    settle();
    assert_eq!(role(&mock), junc_core::model::DefaultRole::Output);
    assert_eq!(view.get_default_role(), "output");
    key(&w, " ");
    settle();
    assert_eq!(role(&mock), junc_core::model::DefaultRole::None);
    key(&w, Key::Escape);
    settle();
    assert!(!view.get_defaults_open());
    drop((w, _app, mock));

    // Board 39: another tool removed USB Microphone 2 → Ch 2; the banner
    // names it, Enter restores it.
    let (w, _app, mock) = open("linkOverridden");
    let view = w.global::<View>();
    assert_eq!(
        view.get_overridden(),
        "Another app removed the link USB Microphone 2 (R) → Ch 2"
    );
    key(&w, "\n");
    settle();
    assert_eq!(mock.resolved().as_deref(), Some("restore"));
    assert_eq!(view.get_overridden(), "");
    drop((w, _app, mock));

    // Board 38: the default input Junction was turned off; the toast says
    // what is the default now, Ctrl+Z turns it on and makes it the default
    // again.
    let (w, _app, mock) = open("defaultRestored");
    let view = w.global::<View>();
    assert_eq!(view.get_toast(), "Default input restored to USB Microphone");
    assert!(view.get_toast_undo());
    assert!(!on(&mock));
    ctrl(&w, "z");
    settle();
    assert!(on(&mock));
    assert_eq!(
        mock.config().expect("config").junction[0].default,
        junc_core::model::DefaultRole::Input
    );
    assert_eq!(view.get_toast(), "");
    drop((w, _app));

    // Board 35: the USB card gives 18 inputs in Pro Audio; Enter would add
    // its 2, Shift+Enter adds the card in Pro Audio.
    let (w, _app, mock) = open("proAudio");
    let view = w.global::<View>();
    let cursor = usize::try_from(view.get_picker_cursor()).expect("cursor");
    let row = view.get_picker_items().row_data(cursor).expect("row");
    assert_eq!(row.detail, "2 of 18 channels");
    assert_eq!(row.alt, "Use all channels");
    assert!(has(&w, "Switches the device to the Pro Audio profile"));
    let hold = |down: bool| {
        w.window().dispatch_event(if down {
            WindowEvent::KeyPressed {
                text: Key::Shift.into(),
            }
        } else {
            WindowEvent::KeyReleased {
                text: Key::Shift.into(),
            }
        });
    };
    hold(true);
    key(&w, "\n");
    hold(false);
    settle();
    let added = mock.config().expect("config").junction[0]
        .input
        .last()
        .cloned()
        .expect("input");
    assert_eq!(added.matcher.profile.as_deref(), Some("pro-audio"));
    assert_eq!(
        added.matcher.device.as_deref(),
        Some("alsa_card.usb-JUNC_Test_18in-00")
    );
    drop((w, _app, mock));

    // Board 40: a headset mic warns; Enter does not add it, Shift+Enter
    // adds it anyway.
    let (w, _app, mock) = open("btWarning");
    let view = w.global::<View>();
    let inputs = |m: &Mock| m.config().expect("config").junction[0].input.len();
    let before = inputs(&mock);
    assert!(has(&w, "Add anyway"));
    key(&w, "\n");
    settle();
    assert_eq!(inputs(&mock), before, "Enter does not add a warned mic");
    assert!(view.get_picker_open());
    w.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Shift.into(),
    });
    key(&w, "\n");
    w.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Shift.into(),
    });
    settle();
    assert_eq!(inputs(&mock), before + 1);
    drop((w, _app, mock));

    // Board 36: 18 channels in groups of eight, the first open; opening
    // In 9–16 gives each of its channels a row and a port.
    let (w, app, _mock) = open("multichannel");
    let view = w.global::<View>();
    let card = view
        .get_cards()
        .iter()
        .position(|c| c.title == "JUNC Test 18in")
        .expect("card");
    let rows = |v: &View| v.get_cards().row_data(card).expect("card").rows.row_count();
    let ports = |v: &View| {
        v.get_ports()
            .iter()
            .filter(|p| p.column == 0 && p.owner == "JUNC Test 18in")
            .count()
    };
    assert_eq!(
        rows(&view),
        1 + 8 + 2,
        "In 1–8 open, In 9–16 and In 17–18 closed"
    );
    assert_eq!(ports(&view), 8 + 2);
    let index = view.get_cards().row_data(card).expect("card").index;
    view.invoke_group_toggled(index, 9);
    settle();
    assert_eq!(rows(&view), 1 + 8 + 1 + 8 + 1);
    assert_eq!(ports(&view), 8 + 8 + 1);
    assert!(
        app.borrow()
            .main()
            .is_some_and(|m| m.inputs[2].groups[1].expanded)
    );
    drop((w, app, _mock));

    // Board 47: the config file does not validate; the banner says where,
    // Open file opens it, Esc dismisses it.
    let (w, _app, mock) = open("configError");
    let view = w.global::<View>();
    assert_eq!(
        view.get_config_error(),
        "Config error in junctions.toml line 14: unknown key 'volum'. Using last good config."
    );
    view.invoke_open_config();
    assert!(
        mock.opened()
            .is_some_and(|f| f.ends_with(".config/junc/junctions.toml"))
    );
    key(&w, Key::Escape);
    settle();
    assert_eq!(view.get_config_error(), "");
    drop((w, _app, mock));

    // Board 43: juncd was updated; What's new opens the release notes.
    let (w, _app, mock) = open("updated");
    let view = w.global::<View>();
    assert_eq!(view.get_toast(), "JUNC updated to 1.1 · audio kept running");
    view.invoke_open_news();
    assert!(
        mock.opened()
            .is_some_and(|u| u.ends_with("/releases/tag/v1.1"))
    );
    assert_eq!(view.get_toast(), "");
    drop((w, _app, mock));

    // Board 49: Mic + App with the mic picked and the app's picker open;
    // Enter picks Discord, Enter creates the Junction from the template.
    let (w, app, mock) = open("templateSlots");
    let view = w.global::<View>();
    let count = |m: &Mock| m.config().expect("config").junction.len();
    let before = count(&mock);
    assert_eq!(view.get_onboarding(), 3);
    assert!(view.get_slot_picker_open());
    assert!(!view.get_slot_ready());
    key(&w, "\n");
    settle();
    assert!(view.get_slot_ready());
    key(&w, "\n");
    settle();
    assert_eq!(view.get_onboarding(), 0);
    let config = mock.config().expect("config");
    assert_eq!(count(&mock), before + 1);
    let made = config.junction.last().expect("made");
    assert!(made.input.iter().any(|i| i.matcher.device.as_deref()
        == Some("alsa_input.usb-Generic_USB_Microphone-00.analog-stereo")));
    assert!(
        made.input
            .iter()
            .any(|i| i.matcher.binary.as_deref() == Some("discord"))
    );
    assert_eq!(app.borrow().selected(), Some(made.id.as_str()));
    drop((w, app, mock));

    // Board 46: the preset's parts, the USB mic and HDMI Output tied to
    // this machine; Enter exports to the presets folder; Ctrl+E opens it.
    let (w, _app, mock) = open("exportPreset");
    let view = w.global::<View>();
    assert!(view.get_export_open());
    assert_eq!(view.get_export_file(), "virtual-mic-for-zoom.junc.toml");
    let devices: Vec<bool> = view.get_export_rows().iter().map(|r| r.device).collect();
    assert_eq!(
        devices,
        [true, false, false, true],
        "Inputs, Bus, Links, Listen"
    );
    key(&w, "\n");
    settle();
    assert!(!view.get_export_open());
    assert!(
        mock.exported()
            .is_some_and(|p| p.ends_with("presets/virtual-mic-for-zoom.junc.toml"))
    );
    assert!(view.get_toast().starts_with("Exported to "));
    ctrl(&w, "e");
    settle();
    assert!(view.get_export_open());
    drop((w, _app, mock));

    // Board 48: six built-in templates and the fixtures' preset; the
    // preset (no slots) is added as it is; Ctrl+I lists presets only.
    let (w, app, mock) = open("newMenu");
    let view = w.global::<View>();
    let count = |m: &Mock| m.config().expect("config").junction.len();
    let before = count(&mock);
    assert!(view.get_tpl_open());
    assert_eq!(view.get_tpl_items().row_count(), 7);
    view.set_tpl_cursor(6);
    key(&w, "\n");
    settle();
    assert!(!view.get_tpl_open());
    assert_eq!(count(&mock), before + 1);
    assert_eq!(app.borrow().selected(), Some("virtual-mic-for-zoom-2"));
    ctrl(&w, "i");
    settle();
    assert_eq!(view.get_tpl_items().row_count(), 1);
    drop((w, app, mock));

    // Board 42: one issue among doctor's checks; R runs them again; Esc
    // goes back to Settings.
    let (w, _app, _mock) = open("doctorIssue");
    let view = w.global::<View>();
    assert!(view.get_doctor_open());
    assert_eq!(view.get_doctor_issues(), 1);
    assert_eq!(view.get_doctor_items().row_count(), 5);
    assert!(has(&w, "1 issue found."));
    key(&w, "r");
    assert!(view.get_doctor_checking());
    settle();
    assert!(!view.get_doctor_checking());
    key(&w, Key::Escape);
    settle();
    assert!(!view.get_doctor_open());
    assert!(view.get_settings_open());
    drop((w, _app, _mock));

    // Board 41: the report's first lines; Enter copies all of it; Save
    // writes it to ~/junc-bug-report.md.
    let (w, _app, mock) = open("bugReport");
    let view = w.global::<View>();
    assert!(view.get_report_open());
    assert!(view.get_report_preview().starts_with("junc 0.1.0"));
    key(&w, "\n");
    settle();
    assert!(!view.get_report_open());
    assert!(
        mock.copied()
            .is_some_and(|t| t.starts_with("## JUNC bug report"))
    );
    view.invoke_open_report();
    settle();
    view.invoke_report_save();
    assert!(
        mock.saved()
            .is_some_and(|p| p.ends_with("junc-bug-report.md"))
    );
}
