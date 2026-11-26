// SPDX-License-Identifier: GPL-3.0-or-later

//! Where the routing canvas puts its cards, ports and links (architecture
//! §7.3), in logical pixels from the canvas origin (the top of the first
//! card). Computed here rather than with Slint bindings: link curves need
//! port positions from three columns, and the same function will mirror
//! the geometry for right-to-left languages later.
//!
//! Sizes follow the Spec sheet: card header 33 px, channel rows 28 px with
//! 4 px between them, 10 px body padding, a 28 px footer (Inputs and
//! Listen), 16 px between cards, 120 px link gutters (72 at least). Below
//! 780 px of canvas the Listen column moves under the Bus (design Narrow)
//! and shows where each channel comes from instead of links.

use crate::main_view::{End, Main};

pub const HEADER: f32 = 33.0;
pub const ROW: f32 = 28.0;
pub const ROW_GAP: f32 = 4.0;
pub const PAD: f32 = 10.0;
pub const FOOTER: f32 = 28.0;
pub const CARD_GAP: f32 = 16.0;
pub const GUTTER: f32 = 120.0;
pub const GUTTER_MIN: f32 = 72.0;
/// Narrowest card before the gutters shrink.
pub const CARD_MIN: f32 = 200.0;
/// Below this canvas width Listen stacks under the Bus.
pub const STACK_BELOW: f32 = 780.0;
/// Height of a column heading (title and count) inside the canvas, used
/// for the stacked Listen heading.
pub const HEADING: f32 = 56.0;
/// A link within this distance of the pointer is under it.
pub const HIT: f32 = 5.0;

/// A rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// A link from port to port.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Curve {
    pub from: Point,
    pub to: Point,
}

impl Curve {
    /// The point at `t` (0–1) of the cubic both of whose control points
    /// sit at half the horizontal span (Spec "Link").
    pub fn at(&self, t: f32) -> Point {
        let mid = f32::midpoint(self.from.x, self.to.x);
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        Point {
            x: a * self.from.x + b * mid + c * mid + d * self.to.x,
            y: a * self.from.y + b * self.from.y + c * self.to.y + d * self.to.y,
        }
    }

    /// Distance from `p` to the curve (as 32 straight pieces).
    pub fn distance(&self, p: Point) -> f32 {
        const PIECES: u16 = 32;
        let mut best = f32::INFINITY;
        let mut a = self.from;
        for i in 1..=PIECES {
            let b = self.at(f32::from(i) / f32::from(PIECES));
            best = best.min(segment_distance(p, a, b));
            a = b;
        }
        best
    }
}

fn segment_distance(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = dx * dx + dy * dy;
    let t = if len == 0.0 {
        0.0
    } else {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / len).clamp(0.0, 1.0)
    };
    let (x, y) = (a.x + t * dx, a.y + t * dy);
    ((p.x - x).powi(2) + (p.y - y).powi(2)).sqrt()
}

/// The whole canvas.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub stacked: bool,
    /// Left edge and width of the Inputs, Bus and Listen columns.
    pub columns: [(f32, f32); 3],
    pub inputs: Vec<Rect>,
    pub bus: Vec<Rect>,
    pub listens: Vec<Rect>,
    /// Top of the Listen heading when stacked.
    pub listen_heading: Option<f32>,
    /// One per link of the model, in its order; `None` when it is not
    /// drawn (Listen links while stacked).
    pub curves: Vec<Option<Curve>>,
    pub height: f32,
}

/// Height of a card with `rows` channels.
pub fn card_height(rows: usize, footer: bool) -> f32 {
    // At most 64 channels: exact in f32.
    #[allow(clippy::cast_precision_loss)]
    let n = rows.max(1) as f32;
    HEADER + PAD + n * ROW + (n - 1.0) * ROW_GAP + PAD + if footer { FOOTER } else { 0.0 }
}

/// Centre of row `i` (0-based) from the card's top.
pub fn row_centre(i: usize) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let i = i as f32;
    HEADER + PAD + ROW / 2.0 + i * (ROW + ROW_GAP)
}

fn stack(x: f32, w: f32, top: f32, heights: impl Iterator<Item = f32>) -> (Vec<Rect>, f32) {
    let mut y = top;
    let mut out = Vec::new();
    for h in heights {
        out.push(Rect { x, y, w, h });
        y += h + CARD_GAP;
    }
    let bottom = if out.is_empty() { top } else { y - CARD_GAP };
    (out, bottom)
}

/// Lays out `m` on a canvas `width` wide; without `listen` (Show Listen
/// off) the Listen column is left out and Inputs and Bus share the width.
pub fn layout(m: &Main, width: f32, listen: bool) -> Layout {
    let stacked = listen && width < STACK_BELOW;
    let cols = if stacked || !listen { 2.0 } else { 3.0 };
    let gutter = ((width - cols * CARD_MIN) / (cols - 1.0)).clamp(GUTTER_MIN, GUTTER);
    let w = ((width - (cols - 1.0) * gutter) / cols).max(0.0);
    let x = |i: f32| i * (w + gutter);
    let columns = if stacked || !listen {
        [(x(0.0), w), (x(1.0), w), (x(1.0), w)]
    } else {
        [(x(0.0), w), (x(1.0), w), (x(2.0), w)]
    };
    let (inputs, in_bottom) = stack(
        columns[0].0,
        w,
        0.0,
        m.inputs.iter().map(|c| card_height(c.channels.len(), true)),
    );
    let (bus, bus_bottom) = stack(
        columns[1].0,
        w,
        0.0,
        m.bus.iter().map(|g| card_height(g.channels.len(), false)),
    );
    let listen_top = if stacked {
        bus_bottom + CARD_GAP + HEADING
    } else {
        0.0
    };
    let (listens, listen_bottom) = stack(
        columns[2].0,
        w,
        listen_top,
        m.listens
            .iter()
            .filter(|_| listen)
            .map(|c| card_height(c.channels.len(), true)),
    );

    let row_of = |rects: &[Rect], card: usize, ch: u8| -> Option<f32> {
        rects
            .get(card)
            .map(|r| r.y + row_centre(usize::from(ch).saturating_sub(1)))
    };
    let bus_row = |ch: u8| -> Option<(Rect, f32)> {
        m.bus.iter().zip(&bus).find_map(|(g, r)| {
            let i = g.channels.iter().position(|c| c.number == ch)?;
            Some((*r, r.y + row_centre(i)))
        })
    };
    let point = |end: End| -> Option<Point> {
        match end {
            End::Input { card, ch } => {
                let r = inputs.get(card)?;
                Some(Point {
                    x: r.x + r.w,
                    y: row_of(&inputs, card, ch)?,
                })
            }
            End::BusIn { ch } => bus_row(ch).map(|(r, y)| Point { x: r.x, y }),
            End::BusOut { ch } => bus_row(ch).map(|(r, y)| Point { x: r.x + r.w, y }),
            End::Listen { card, ch } => {
                let r = listens.get(card)?;
                Some(Point {
                    x: r.x,
                    y: row_of(&listens, card, ch)?,
                })
            }
        }
    };
    let curves = m
        .links
        .iter()
        .map(|l| {
            if (stacked || !listen) && matches!(l.to, End::Listen { .. }) {
                return None;
            }
            Some(Curve {
                from: point(l.from)?,
                to: point(l.to)?,
            })
        })
        .collect();
    Layout {
        stacked,
        columns,
        inputs,
        bus,
        listens,
        listen_heading: stacked.then_some(bus_bottom + CARD_GAP),
        curves,
        height: in_bottom.max(bus_bottom).max(listen_bottom),
    }
}

impl Layout {
    /// The link under the pointer at `p`, if any: the nearest within
    /// [`HIT`].
    pub fn hit(&self, p: Point) -> Option<usize> {
        self.curves
            .iter()
            .enumerate()
            .filter_map(|(i, c)| Some((i, c.as_ref()?.distance(p))))
            .filter(|(_, d)| *d <= HIT)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::main_view::main;

    fn board() -> Main {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/golden/describe/full.json"
        ))
        .expect("golden");
        let v: serde_json::Value = serde_json::from_str(&text).expect("json");
        let d = serde_json::from_value(v["data"].clone()).expect("describe");
        main(&d, "virtual-mic-for-zoom").expect("junction")
    }

    #[test]
    fn cards_and_ports_sit_where_the_design_puts_them() {
        // Design board 1: a 1092 px canvas (1440 window − 300 sidebar − 2 ×
        // 18 padding − 12 frame): three 284 px columns, 120 px gutters.
        let l = layout(&board(), 1092.0, true);
        assert!(!l.stacked);
        assert_eq!(l.columns[1], (404.0, 284.0));
        let spotify = l.inputs[0];
        assert!(
            (spotify.h - 141.0).abs() < 0.01,
            "two rows and a footer: {}",
            spotify.h
        );
        assert!((l.inputs[1].y - 157.0).abs() < 0.01);
        assert!((row_centre(0) - 57.0).abs() < 0.01 && (row_centre(1) - 89.0).abs() < 0.01);
        let first = l.curves[0].expect("Spotify 1 → Ch 1");
        assert_eq!(first.from, Point { x: 284.0, y: 57.0 });
        assert_eq!(first.to, Point { x: 404.0, y: 57.0 });
        assert!((l.bus[1].h - 113.0).abs() < 0.01, "no footer");
        // Inputs: 141 + 16 + 141 + 16 + 109 (the mono mic) is the tallest.
        assert!((l.height - 423.0).abs() < 0.01, "{}", l.height);
    }

    #[test]
    fn narrow_canvases_stack_listen_without_links() {
        let l = layout(&board(), 600.0, true);
        assert!(l.stacked);
        assert_eq!(l.columns[1], l.columns[2], "Listen under the Bus");
        let heading = l.listen_heading.expect("heading");
        assert!(l.listens[0].y > heading);
        let m = board();
        for (link, c) in m.links.iter().zip(&l.curves) {
            assert_eq!(c.is_none(), matches!(link.to, End::Listen { .. }));
        }
        let gutter = l.columns[1].0 - l.columns[0].1;
        assert!((GUTTER_MIN..=GUTTER).contains(&gutter), "{gutter}");
    }

    #[test]
    fn without_listen_two_columns_share_the_width() {
        let l = layout(&board(), 1092.0, false);
        assert!(!l.stacked && l.listens.is_empty());
        assert!(l.columns[0].1 > 400.0, "{:?}", l.columns);
        let m = board();
        for (link, c) in m.links.iter().zip(&l.curves) {
            assert_eq!(c.is_none(), matches!(link.to, End::Listen { .. }));
        }
    }

    #[test]
    fn the_pointer_finds_the_nearest_link() {
        let l = layout(&board(), 1092.0, true);
        let c = l.curves[0].expect("curve");
        let mid = c.at(0.5);
        assert_eq!(l.hit(mid), Some(0));
        assert_eq!(
            l.hit(Point {
                x: mid.x,
                y: mid.y + 40.0
            }),
            None
        );
        assert!(c.distance(c.from) < 0.01);
    }
}
