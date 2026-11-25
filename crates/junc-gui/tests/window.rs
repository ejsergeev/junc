// SPDX-License-Identifier: GPL-3.0-or-later

//! The window skeleton on Slint's testing backend (no display): a theme
//! change retints the same window in place, the layout follows the tile
//! width (architecture §7.1), and the sidebar lists the Junctions.

use i_slint_backend_testing::ElementHandle;
use junc_gui::theme;
use junc_gui::ui::{self, AppWindow, JunctionItem, Tokens};
use slint::{ComponentHandle, LogicalSize, ModelRc, VecModel};

fn labelled(w: &AppWindow, label: &str) -> bool {
    ElementHandle::find_by_accessible_label(w, label)
        .next()
        .is_some()
}

#[test]
fn the_window_retints_resizes_and_lists_junctions() {
    i_slint_backend_testing::init_no_event_loop();
    let w = AppWindow::new().expect("window");

    // Retint in place: one window, two themes.
    let tk = theme::fallback(false);
    ui::apply(&w, &tk);
    let g = w.global::<Tokens>();
    assert_eq!(
        g.get_surface_window(),
        slint::Color::from_rgb_u8(0x1a, 0x1b, 0x26)
    );
    assert!(g.get_dark());
    let fx = theme::fallback(true);
    ui::apply(&w, &fx);
    assert_eq!(
        g.get_surface_window(),
        slint::Color::from_rgb_u8(0xff, 0xfc, 0xf0)
    );
    assert_eq!(g.get_accent(), slint::Color::from_rgb_u8(0x20, 0x5e, 0xa6));
    assert!(!g.get_dark());
    // Spacing and type follow [spacing] scale and [font] base-size.
    let mut big = fx.clone();
    big.scale = 1.5;
    big.font_size = 16.0;
    ui::apply(&w, &big);
    assert_eq!(g.get_size_control(), 42.0);
    assert_eq!(g.get_type_title(), 16.0 * 14.0 / 12.0);

    // Sizes (architecture §7.1).
    let size = |width: f32| {
        w.window().set_size(LogicalSize::new(width, 800.0));
        (w.get_sidebar_width(), w.get_stacked())
    };
    assert_eq!(size(1280.0), (300.0, false), "wide tile");
    assert_eq!(
        size(1099.0),
        (240.0, false),
        "narrow sidebar, three columns"
    );
    assert_eq!(size(1000.0), (240.0, true), "Listen below Inputs and Bus");

    // The sidebar: juncd down, then two Junctions.
    assert!(labelled(&w, "juncd is not running"));
    let items = VecModel::from(vec![
        JunctionItem {
            id: "zoom".into(),
            name: "Zoom Mic".into(),
            enabled: true,
            status: "active".into(),
        },
        JunctionItem {
            id: "obs".into(),
            name: "OBS Mix".into(),
            enabled: false,
            status: "off".into(),
        },
    ]);
    w.set_junctions(ModelRc::new(items));
    w.set_daemon_state("connected".into());
    assert!(labelled(&w, "Zoom Mic") && labelled(&w, "OBS Mix"));
}
