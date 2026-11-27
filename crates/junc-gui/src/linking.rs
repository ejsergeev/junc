// SPDX-License-Identifier: GPL-3.0-or-later

//! Making a link (design boards 13, 14; F4): from a port, by dragging it
//! or with Enter and the arrow keys, to a port it may link to. Plain data
//! on the Main model and its layout, so it is tested without a window.
//! Audio goes Input → Bus in, and Bus out → Listen; a link may start at
//! either end.

use crate::edits::Part;
use crate::layout::{Layout, Point, row_centre};
use crate::main_view::{End, Main};

/// How near a port the pointer snaps to it.
pub const SNAP: f32 = 16.0;

/// Every port on the canvas and where it is, in drawing order.
pub fn ports(m: &Main, l: &Layout) -> Vec<(End, Point)> {
    let mut out = Vec::new();
    for (card, (c, r)) in m.inputs.iter().zip(&l.inputs).enumerate() {
        for (k, ch) in c.channels.iter().enumerate() {
            out.push((
                End::Input {
                    card,
                    ch: ch.number,
                },
                Point {
                    x: r.x + r.w,
                    y: r.y + row_centre(k),
                },
            ));
        }
    }
    let outs = !l.stacked && !l.listens.is_empty();
    for (g, r) in m.bus.iter().zip(&l.bus) {
        for (k, ch) in g.channels.iter().enumerate() {
            let y = r.y + row_centre(k);
            out.push((End::BusIn { ch: ch.number }, Point { x: r.x, y }));
            if outs {
                out.push((End::BusOut { ch: ch.number }, Point { x: r.x + r.w, y }));
            }
        }
    }
    if !l.stacked {
        for (card, (c, r)) in m.listens.iter().zip(&l.listens).enumerate() {
            for (k, ch) in c.channels.iter().enumerate() {
                out.push((
                    End::Listen {
                        card,
                        ch: ch.number,
                    },
                    Point {
                        x: r.x,
                        y: r.y + row_centre(k),
                    },
                ));
            }
        }
    }
    out
}

/// Where port `end` is, if it is drawn.
pub fn at(m: &Main, l: &Layout, end: End) -> Option<Point> {
    ports(m, l)
        .into_iter()
        .find(|(e, _)| *e == end)
        .map(|(_, p)| p)
}

/// Whether `a` and `b` are linked now.
fn linked(m: &Main, a: End, b: End) -> bool {
    m.links
        .iter()
        .any(|k| (k.from == a && k.to == b) || (k.from == b && k.to == a))
}

/// The ports a link from `from` may end at, top to bottom: the other side
/// of the Bus, never one it is linked to already, never a card that is
/// off.
pub fn targets(m: &Main, l: &Layout, from: End) -> Vec<End> {
    let on_input = |card: usize| m.inputs.get(card).is_some_and(|c| c.enabled);
    let on_listen = |card: usize| m.listens.get(card).is_some_and(|c| c.enabled);
    let fits = |to: End| match (from, to) {
        (End::Input { card, .. }, End::BusIn { .. }) => on_input(card),
        (End::BusIn { .. }, End::Input { card, .. }) => on_input(card),
        (End::BusOut { .. }, End::Listen { card, .. })
        | (End::Listen { card, .. }, End::BusOut { .. }) => on_listen(card),
        _ => false,
    };
    let mut out: Vec<(End, Point)> = ports(m, l)
        .into_iter()
        .filter(|(to, _)| fits(*to) && !linked(m, from, *to))
        .collect();
    out.sort_by(|a, b| a.1.y.total_cmp(&b.1.y).then(a.1.x.total_cmp(&b.1.x)));
    out.into_iter().map(|(e, _)| e).collect()
}

/// The target of a link from `from` nearest `p`, within [`SNAP`].
pub fn snap(m: &Main, l: &Layout, from: End, p: Point) -> Option<End> {
    let ok = targets(m, l, from);
    ports(m, l)
        .into_iter()
        .filter(|(e, _)| ok.contains(e))
        .map(|(e, q)| (e, (q.x - p.x).hypot(q.y - p.y)))
        .filter(|(_, d)| *d <= SNAP)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(e, _)| e)
}

/// What linking `a` and `b` adds to the config: the Input or Listen output
/// that owns the link and its pair, as the config writes it.
pub fn pair(m: &Main, a: End, b: End) -> Option<(Part, [u8; 2])> {
    match (a, b) {
        (End::Input { card, ch }, End::BusIn { ch: bus })
        | (End::BusIn { ch: bus }, End::Input { card, ch }) => {
            Some((Part::Input(m.inputs.get(card)?.id.clone()), [ch, bus]))
        }
        (End::BusOut { ch: bus }, End::Listen { card, ch })
        | (End::Listen { card, ch }, End::BusOut { ch: bus }) => {
            Some((Part::Listen(m.listens.get(card)?.id.clone()), [bus, ch]))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::layout;
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
    fn a_link_goes_across_the_bus_and_never_twice() {
        let m = board();
        let l = layout(&m, 1092.0, true);
        // Soundux (card 1) channel 1 feeds Bus 3: it may go to the others.
        let from = End::Input { card: 1, ch: 1 };
        let t = targets(&m, &l, from);
        assert!(t.iter().all(|e| matches!(e, End::BusIn { .. })));
        assert!(!t.contains(&End::BusIn { ch: 3 }), "linked already");
        assert!(t.contains(&End::BusIn { ch: 1 }));
        // From the Bus the other way: Inputs.
        let back = targets(&m, &l, End::BusIn { ch: 4 });
        assert!(back.iter().all(|e| matches!(e, End::Input { .. })));
        // Bus out to Listen, and back.
        let out = targets(&m, &l, End::BusOut { ch: 3 });
        assert!(!out.is_empty() && out.iter().all(|e| matches!(e, End::Listen { .. })));
        assert!(
            targets(&m, &l, End::Listen { card: 0, ch: 1 })
                .iter()
                .all(|e| matches!(e, End::BusOut { .. }))
        );
        // Top to bottom.
        let ys: Vec<f32> = t
            .iter()
            .filter_map(|e| at(&m, &l, *e))
            .map(|p| p.y)
            .collect();
        assert!(ys.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn the_pointer_snaps_to_a_target_nearby() {
        let m = board();
        let l = layout(&m, 1092.0, true);
        let from = End::Input { card: 1, ch: 1 };
        let p = at(&m, &l, End::BusIn { ch: 1 }).expect("Bus 1 in");
        let near = Point {
            x: p.x + 5.0,
            y: p.y - 5.0,
        };
        assert_eq!(snap(&m, &l, from, near), Some(End::BusIn { ch: 1 }));
        let far = Point {
            x: p.x + 40.0,
            y: p.y,
        };
        assert_eq!(snap(&m, &l, from, far), None);
        let own = at(&m, &l, End::BusIn { ch: 3 }).expect("Bus 3 in");
        assert_eq!(snap(&m, &l, from, own), None, "linked already");
    }

    #[test]
    fn a_link_belongs_to_its_input_or_listen_output() {
        let m = board();
        assert_eq!(
            pair(&m, End::Input { card: 1, ch: 2 }, End::BusIn { ch: 1 }),
            Some((Part::Input("soundux".into()), [2, 1]))
        );
        assert_eq!(
            pair(&m, End::BusIn { ch: 1 }, End::Input { card: 1, ch: 2 }),
            Some((Part::Input("soundux".into()), [2, 1])),
            "either way round"
        );
        assert_eq!(
            pair(&m, End::Listen { card: 0, ch: 2 }, End::BusOut { ch: 4 }),
            Some((Part::Listen(m.listens[0].id.clone()), [4, 2]))
        );
        assert_eq!(pair(&m, End::BusIn { ch: 1 }, End::BusOut { ch: 1 }), None);
    }
}
