// SPDX-License-Identifier: GPL-3.0-or-later

//! The Slint side: the generated window and assigning [`theme::Tokens`]
//! to its `Tokens` global in one pass (architecture §7.7 step 4).

use crate::layout::{Layout, Point, Rect, row_centre};
use crate::main_view::{self, Card, CardState, End, Icon, Main};
use crate::meter_service::Slot;
use crate::picker::{Entry, Group};
use crate::state::Row;
use crate::theme::{self, Color};
use slint::{ModelRc, VecModel};

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
    apply_to(&window.global::<Tokens>(), t);
}

/// [`apply`] for any window's `Tokens` global (the app, the gallery).
pub fn apply_to(g: &Tokens<'_>, t: &theme::Tokens) {
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

thread_local! {
    static LANGUAGE: std::cell::RefCell<String> = std::cell::RefCell::new("en".to_owned());
}

/// Shows the UI in `code` (a junc-l10n language code): Slint switches every
/// `@tr` in place; lists joined here follow. Unknown codes show English.
///
/// Call it after creating the first window, always: before that Slint has
/// no translations to select, and at that moment it picks one from the
/// system locale by language alone, so `en_US` would get the `en_XA`
/// pseudo-locale. JUNC's language comes from Omarchy (junc-l10n) instead.
pub fn set_language(code: &str) {
    let lang = if slint::select_bundled_translation(code).is_ok() {
        code
    } else {
        let _ = slint::select_bundled_translation("en");
        "en"
    };
    LANGUAGE.with(|l| lang.clone_into(&mut l.borrow_mut()));
}

/// How names are listed in the UI language: "a, b" or "a、b".
fn list(names: &[String]) -> String {
    let cjk = LANGUAGE.with(|l| {
        let l = l.borrow();
        l.starts_with("ja") || l.starts_with("zh")
    });
    names.join(if cjk { "、" } else { ", " })
}

/// The Nerd Font glyph of a card or sidebar icon.
pub fn glyph(icon: Icon) -> &'static str {
    match icon {
        Icon::Music => "\u{f075a}",
        Icon::App => "\u{f11d9}",
        Icon::Mic => "\u{f036c}",
        Icon::Monitor => "\u{f0f5f}",
        Icon::Junction => "\u{f062d}",
        Icon::Speaker => "\u{f04c3}",
        Icon::Screen => "\u{f0379}",
    }
}

/// An Input card's channel rows for its Options panel (no meters).
pub fn option_channels(card: &Card) -> Vec<ChannelItem> {
    card.channels
        .iter()
        .map(|c| ChannelItem {
            number: i32::from(c.number),
            position: c.position.into(),
            meter: -1,
            source: 0,
        })
        .collect()
}

/// The onboarding check rows.
pub fn checks(rows: &[crate::checks::Row]) -> Vec<CheckItem> {
    rows.iter()
        .map(|r| CheckItem {
            id: r.id.as_str().into(),
            state: r.state.into(),
            fix: r.fix.as_str().into(),
        })
        .collect()
}

/// The picker rows `shown` (indexes into `entries`) in the UI language:
/// each group's caption on its first row, monitors as "Monitor of …".
pub fn picker(words: &Words<'_>, entries: &[Entry], shown: &[usize]) -> Vec<PickerItem> {
    let mut last = None;
    shown
        .iter()
        .filter_map(|&i| entries.get(i))
        .map(|e| {
            let group = if last == Some(e.group) {
                String::new()
            } else {
                last = Some(e.group);
                match e.group {
                    Group::Apps => words.invoke_running_apps(),
                    Group::Hardware => words.invoke_hardware_inputs(),
                    Group::System => words.invoke_system_audio(),
                    Group::Junctions => words.invoke_junctions(),
                    Group::Outputs => words.invoke_output_devices(),
                }
                .to_uppercase()
            };
            let text = if e.group == Group::System {
                words.invoke_monitor_of(e.label.as_str().into())
            } else if e.group == Group::Outputs && e.label.is_empty() {
                words.invoke_default_output()
            } else {
                e.label.as_str().into()
            };
            PickerItem {
                group: group.into(),
                text,
                detail: if e.cable {
                    words.invoke_cable()
                } else {
                    "".into()
                },
                added: e.added,
                icon: glyph(e.icon).into(),
            }
        })
        .collect()
}

/// The sidebar rows; row `i`'s meter is level `i`.
pub fn sidebar(rows: &[Row], selected: Option<&str>) -> Vec<JunctionItem> {
    rows.iter()
        .zip(0..)
        .map(|(r, i)| JunctionItem {
            id: r.id.as_str().into(),
            name: r.name.as_str().into(),
            enabled: r.enabled,
            status: r.status.as_str().into(),
            node: r.node.as_str().into(),
            cable: r.cable,
            icons: r
                .icons
                .iter()
                .map(|i| glyph(*i))
                .collect::<Vec<_>>()
                .join(" ")
                .into(),
            inputs: list(&r.inputs).into(),
            offline: i32::try_from(r.offline).unwrap_or(i32::MAX),
            volume: i32::from(r.volume),
            selected: selected == Some(r.id.as_str()),
            meter: i,
        })
        .collect()
}

/// The Slint models of the canvas.
#[derive(Debug, Default)]
pub struct Canvas {
    pub cards: Vec<CardItem>,
    pub ports: Vec<PortItem>,
    pub links: Vec<LinkItem>,
}

fn px(v: f32) -> f32 {
    v
}

fn int(v: impl TryInto<i32>) -> i32 {
    v.try_into().unwrap_or(i32::MAX)
}

/// The canvas of `m` laid out as `l`, the link `hovered` drawn as such.
/// Channel meters take levels from `meter_base` on, in card order (Inputs,
/// Bus, Listen), as [`slots`] lists them.
pub fn canvas(m: &Main, l: &Layout, hovered: Option<usize>, meter_base: usize) -> Canvas {
    let mut out = Canvas::default();
    let mut meter = meter_base;
    let mut channels =
        |chs: &[main_view::Channel], sources: &dyn Fn(u8) -> u8| -> Vec<ChannelItem> {
            chs.iter()
                .map(|c| {
                    let item = ChannelItem {
                        number: i32::from(c.number),
                        position: c.position.into(),
                        meter: int(meter),
                        source: i32::from(sources(c.number)),
                    };
                    meter += 1;
                    item
                })
                .collect()
        };
    let none = |_: u8| 0u8;
    for (i, (c, r)) in m.inputs.iter().zip(&l.inputs).enumerate() {
        out.cards
            .push(card(0, i, c, r, channels(&c.channels, &none)));
    }
    for (i, (g, r)) in m.bus.iter().zip(&l.bus).enumerate() {
        out.cards.push(CardItem {
            column: 1,
            index: int(i),
            on: true,
            state: "ok".into(),
            first: i32::from(g.first),
            stereo: g.channels.len() == 2,
            x: px(r.x),
            y: px(r.y),
            w: px(r.w),
            h: px(r.h),
            channels: ModelRc::new(VecModel::from(channels(&g.channels, &none))),
            ..CardItem::default()
        });
    }
    for (i, (c, r)) in m.listens.iter().zip(&l.listens).enumerate() {
        // Stacked, a Listen row says which Bus channel feeds it.
        let source = |ch: u8| {
            m.links
                .iter()
                .find(|k| k.to == End::Listen { card: i, ch })
                .and_then(|k| match k.from {
                    End::BusOut { ch } => Some(ch),
                    _ => None,
                })
                .unwrap_or(0)
        };
        out.cards
            .push(card(2, i, c, r, channels(&c.channels, &source)));
    }

    let linked = |end: End| m.links.iter().any(|k| k.from == end || k.to == end);
    let mut port =
        |x: f32, y: f32, state: &str, column: i32, card: usize, ch: u8, side: i32, owner: &str| {
            out.ports.push(PortItem {
                x: px(x),
                y: px(y),
                state: state.into(),
                column,
                card: int(card),
                ch: i32::from(ch),
                side,
                owner: owner.into(),
            });
        };
    for (i, (c, r)) in m.inputs.iter().zip(&l.inputs).enumerate() {
        for (k, ch) in c.channels.iter().enumerate() {
            let state = if c.state == CardState::Offline {
                "offline"
            } else if c.enabled
                && linked(End::Input {
                    card: i,
                    ch: ch.number,
                })
            {
                "linked"
            } else {
                "free"
            };
            port(
                r.x + r.w,
                r.y + row_centre(k),
                state,
                0,
                i,
                ch.number,
                1,
                &c.title,
            );
        }
    }
    for (g, r) in m.bus.iter().zip(&l.bus) {
        for (k, ch) in g.channels.iter().enumerate() {
            let y = r.y + row_centre(k);
            let state = |end| if linked(end) { "linked" } else { "free" };
            port(
                r.x,
                y,
                state(End::BusIn { ch: ch.number }),
                1,
                0,
                ch.number,
                0,
                "",
            );
            if !l.stacked && !l.listens.is_empty() {
                port(
                    r.x + r.w,
                    y,
                    state(End::BusOut { ch: ch.number }),
                    1,
                    0,
                    ch.number,
                    1,
                    "",
                );
            }
        }
    }
    if !l.stacked {
        for (i, (c, r)) in m.listens.iter().zip(&l.listens).enumerate() {
            for (k, ch) in c.channels.iter().enumerate() {
                let state = if c.state == CardState::Offline {
                    "offline"
                } else if linked(End::Listen {
                    card: i,
                    ch: ch.number,
                }) {
                    "linked"
                } else {
                    "free"
                };
                port(
                    r.x,
                    r.y + row_centre(k),
                    state,
                    2,
                    i,
                    ch.number,
                    0,
                    &c.title,
                );
            }
        }
    }

    for (i, (link, curve)) in m.links.iter().zip(&l.curves).enumerate() {
        let Some(c) = curve else { continue };
        out.links
            .push(link_item(c.from, c.to, link.offline, hovered == Some(i)));
    }
    out
}

/// A link drawn from `from` to `to`, in a box spanning both.
pub fn link_item(from: Point, to: Point, offline: bool, hovered: bool) -> LinkItem {
    let (x, y) = (from.x.min(to.x), from.y.min(to.y) - 8.0);
    LinkItem {
        x: px(x),
        y: px(y),
        w: px((to.x - from.x).abs().max(1.0)),
        h: px((to.y - from.y).abs() + 16.0),
        x1: px(from.x - x),
        y1: px(from.y - y),
        x2: px(to.x - x),
        y2: px(to.y - y),
        offline,
        hovered,
    }
}

/// The end a port of the canvas stands for.
pub fn port_end(column: i32, card: i32, ch: i32, side: i32) -> Option<End> {
    let card = usize::try_from(card).ok()?;
    let ch = u8::try_from(ch).ok()?;
    Some(match (column, side) {
        (0, _) => End::Input { card, ch },
        (1, 0) => End::BusIn { ch },
        (1, _) => End::BusOut { ch },
        _ => End::Listen { card, ch },
    })
}

fn card(column: i32, i: usize, c: &Card, r: &Rect, channels: Vec<ChannelItem>) -> CardItem {
    CardItem {
        column,
        index: int(i),
        id: c.id.as_str().into(),
        title: c.title.as_str().into(),
        icon: glyph(c.icon).into(),
        on: c.enabled,
        state: c.state.as_str().into(),
        exclusive: c.exclusive,
        volume: i32::from(c.volume),
        x: px(r.x),
        y: px(r.y),
        w: px(r.w),
        h: px(r.h),
        channels: ModelRc::new(VecModel::from(channels)),
        ..CardItem::default()
    }
}

/// The meters in the order [`sidebar`] and [`canvas`] number them: one per
/// sidebar row (its loudest channel), then one per channel row.
pub fn slots(rows: &[Row], m: Option<&Main>) -> Vec<Slot> {
    let mut out: Vec<Slot> = rows
        .iter()
        .map(|r| Slot {
            node: r.node.clone(),
            channel: None,
        })
        .collect();
    if let Some(m) = m {
        let channels = m
            .inputs
            .iter()
            .flat_map(|c| &c.channels)
            .chain(m.bus.iter().flat_map(|g| &g.channels))
            .chain(m.listens.iter().flat_map(|c| &c.channels));
        for ch in channels {
            // Bus channels sit on the Junction's own node, numbered from 1.
            out.push(Slot {
                node: ch.meter_node.clone(),
                channel: Some(usize::from(ch.number) - 1),
            });
        }
    }
    out
}
