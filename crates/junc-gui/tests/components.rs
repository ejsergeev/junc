// SPDX-License-Identifier: GPL-3.0-or-later

//! Components behave as the Spec says, from the keyboard and for assistive
//! technology (what AT-SPI reads is the accessible properties checked
//! here). Runs on Slint's testing backend, on the component gallery.

use i_slint_backend_testing::ElementHandle;
use junc_gui::ui::GalleryWindow;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, SharedString};

fn key(w: &GalleryWindow, k: impl Into<SharedString> + Clone) {
    let text: SharedString = k.into();
    w.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.window().dispatch_event(WindowEvent::KeyReleased { text });
}

fn with_shift(w: &GalleryWindow, k: Key) {
    let shift: SharedString = Key::Shift.into();
    w.window().dispatch_event(WindowEvent::KeyPressed {
        text: shift.clone(),
    });
    key(w, k);
    w.window()
        .dispatch_event(WindowEvent::KeyReleased { text: shift });
}

fn one(w: &GalleryWindow, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(w, label)
        .next()
        .unwrap_or_else(|| panic!("no element labelled {label:?}"))
}

#[test]
fn controls_work_from_the_keyboard_and_describe_themselves() {
    i_slint_backend_testing::init_no_event_loop();
    let w = GalleryWindow::new().expect("gallery");
    w.show().expect("show");

    // Switch: labelled, "on"/"off" described (validation fix), Space toggles.
    let zoom = one(&w, "Zoom Mic");
    assert_eq!(
        zoom.accessible_role(),
        Some(i_slint_backend_testing::AccessibleRole::Switch)
    );
    assert_eq!(zoom.accessible_checked(), Some(true));
    assert_eq!(zoom.accessible_description().as_deref(), Some("on"));
    // Tab order: Delete, Show Listen, Add input, the icon button (the
    // disabled button is skipped), then the first switch.
    for _ in 0..5 {
        key(&w, Key::Tab);
    }
    key(&w, ' ');
    assert_eq!(zoom.accessible_checked(), Some(false));
    assert_eq!(zoom.accessible_description().as_deref(), Some("off"));
    key(&w, ' ');
    assert_eq!(zoom.accessible_checked(), Some(true));

    // Slider: OBS Mix switch, then the volume slider (disabled switch
    // skipped).
    key(&w, Key::Tab);
    key(&w, Key::Tab);
    let volume = one(&w, "Zoom Mic volume");
    assert_eq!(volume.accessible_value().as_deref(), Some("50%"));
    key(&w, Key::RightArrow);
    assert_eq!(volume.accessible_value().as_deref(), Some("51%"));
    with_shift(&w, Key::RightArrow);
    assert_eq!(volume.accessible_value().as_deref(), Some("61%"));
    key(&w, Key::PageDown);
    assert_eq!(volume.accessible_value().as_deref(), Some("51%"));
    key(&w, Key::Home);
    assert_eq!(volume.accessible_value().as_deref(), Some("0%"));
    key(&w, Key::End);
    assert_eq!(volume.accessible_value().as_deref(), Some("100%"));
    assert_eq!(volume.accessible_value_maximum(), Some(100.0));
    // A boost slider stops End at 100 %, not at its 150 % top.
    key(&w, Key::Tab);
    key(&w, Key::End);
    let boost = one(&w, "Boost");
    assert_eq!(boost.accessible_value().as_deref(), Some("100%"));
    boost.invoke_accessible_increment_action();
    assert_eq!(boost.accessible_value().as_deref(), Some("101%"));

    // Buttons: labels, shortcut as description, disabled state.
    let add = one(&w, "Add input");
    assert_eq!(add.accessible_description().as_deref(), Some("Ctrl+N"));
    assert_eq!(one(&w, "Disabled").accessible_enabled(), Some(false));
    let show = one(&w, "Show Listen");
    assert_eq!(show.accessible_checked(), Some(true));
    show.invoke_accessible_default_action();
    assert_eq!(show.accessible_checked(), Some(false));
}

#[test]
fn cards_ports_and_channel_groups_describe_their_state() {
    i_slint_backend_testing::init_no_event_loop();
    let w = GalleryWindow::new().expect("gallery");
    w.show().expect("show");

    // Cards are labelled groups; a state other than "on and present" is
    // their description, and their switch is labelled by the card title.
    let mic = ElementHandle::find_by_accessible_label(&w, "USB Microphone")
        .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Groupbox))
        .expect("card");
    assert_eq!(mic.accessible_description().as_deref(), Some("Offline"));
    let off = ElementHandle::find_by_accessible_label(&w, "Soundux")
        .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Groupbox))
        .expect("card");
    assert_eq!(off.accessible_description().as_deref(), Some("off"));
    let switch = ElementHandle::find_by_accessible_label(&w, "Soundux")
        .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Switch))
        .expect("switch");
    switch.invoke_accessible_default_action();
    assert_eq!(off.accessible_description().as_deref(), Some(""));
    // The card's volume slider is labelled after the card.
    assert_eq!(
        one(&w, "Spotify volume").accessible_value().as_deref(),
        Some("70%")
    );

    // Ports say whether they are linked.
    assert_eq!(
        one(&w, "Ch 2 linked").accessible_description().as_deref(),
        Some("linked")
    );

    // Channel groups: status as description, expandable.
    let group = one(&w, "In 9–16");
    assert_eq!(group.accessible_description().as_deref(), Some("8 ch"));
    assert_eq!(group.accessible_expanded(), Some(false));
    group.invoke_accessible_expand_action();
    assert_eq!(group.accessible_expanded(), Some(true));
    assert_eq!(
        one(&w, "In 1–2").accessible_description().as_deref(),
        Some("2 of 2 linked")
    );
}
