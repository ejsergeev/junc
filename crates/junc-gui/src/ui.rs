// SPDX-License-Identifier: GPL-3.0-or-later

//! The Slint side: the generated window and assigning [`theme::Tokens`]
//! to its `Tokens` global in one pass (architecture §7.7 step 4).

use crate::theme::{self, Color};

/// The code Slint generates from `ui/app.slint`; the workspace lints do
/// not apply to it.
#[allow(
    unsafe_code,
    unused_qualifications,
    missing_debug_implementations,
    clippy::all,
    clippy::unwrap_used,
    clippy::todo,
    clippy::dbg_macro
)]
mod generated {
    slint::include_modules!();
}
pub use generated::*;

fn color(c: Color) -> slint::Color {
    slint::Color::from_argb_u8(c.a, c.r, c.g, c.b)
}

/// Retints the window: every token, in one pass; bindings re-render in
/// place.
pub fn apply(window: &AppWindow, t: &theme::Tokens) {
    let g = window.global::<Tokens>();
    g.set_dark(t.dark);
    g.set_surface_desktop(color(t.surface_desktop));
    g.set_surface_window(color(t.surface_window));
    g.set_surface_sidebar(color(t.surface_sidebar));
    g.set_surface_card(color(t.surface_card));
    g.set_surface_card_header(color(t.surface_card_header));
    g.set_text_default(color(t.text_default));
    g.set_text_strong(color(t.text_strong));
    g.set_text_subtle(color(t.text_subtle));
    g.set_mark_dim(color(t.mark_dim));
    g.set_line_divider(color(t.line_divider));
    g.set_accent(color(t.accent));
    g.set_selection(color(t.selection));
    g.set_meter_safe(color(t.meter_safe));
    g.set_meter_hot(color(t.meter_hot));
    g.set_meter_clip(color(t.meter_clip));
    g.set_status_urgent(color(t.status_urgent));
    g.set_status_ok(color(t.status_ok));
    g.set_status_warn(color(t.status_warn));
    g.set_control_normal_fill(color(t.control_normal_fill));
    g.set_control_normal_border(color(t.control_normal_border));
    g.set_control_hover_fill(color(t.control_hover_fill));
    g.set_control_hover_border(color(t.control_hover_border));
    g.set_control_focus_fill(color(t.control_focus_fill));
    g.set_control_focus_border(color(t.control_focus_border));
    g.set_control_selected_fill(color(t.control_selected_fill));
    g.set_control_pressed_fill(color(t.control_pressed_fill));
    g.set_text_selection_fill(color(t.text_selection_fill));
    g.set_track(color(t.track));
    g.set_halo(color(t.halo));
    g.set_font_family(t.font_family.as_str().into());
    // Slint lengths are f32; theme values are small (1.0, 12.0).
    #[allow(clippy::cast_possible_truncation)]
    {
        g.set_scale(t.scale as f32);
        g.set_font_base(t.font_size as f32);
    }
}

/// Shows a snapshot: the daemon's state and the sidebar rows.
pub fn show(window: &AppWindow, s: &crate::state::Snapshot) {
    let rows: Vec<JunctionItem> = s
        .junctions
        .iter()
        .map(|r| JunctionItem {
            id: r.id.as_str().into(),
            name: r.name.as_str().into(),
            enabled: r.enabled,
            status: r.status.as_str().into(),
        })
        .collect();
    window.set_junctions(slint::ModelRc::new(slint::VecModel::from(rows)));
    window.set_daemon_state(s.daemon.as_str().into());
}
