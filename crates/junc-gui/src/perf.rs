// SPDX-License-Identifier: GPL-3.0-or-later

//! Timings for `junc-bench gui` (B26, architecture §11.4), off unless
//! `JUNC_GUI_PERF=FILE` is set, so the bench measures the binary that
//! ships. Lines
//! `name=value` are appended to FILE:
//!
//! - `first_frame_ms`: from `JUNC_GUI_PERF_T0` (Unix time in ns, taken by
//!   whoever started the process) to the first frame presented after
//!   juncd's state with a Junction arrived;
//! - `JUNC_GUI_PERF_DRAG_MS` after that frame, a link is dragged from the
//!   first Input port to its first target and back, one step per frame,
//!   and cancelled: `drag_frames`, `drag_render_p95_ms` and
//!   `drag_render_max_ms` (Slint's BeforeRendering to AfterRendering),
//!   `drag_interval_p95_ms` (between presented frames); then `done=1`.
//!
//! Frames come from Slint's rendering notifier, which the software
//! renderer does not offer: there only nothing is written.

use crate::app;
use crate::layout::Point;
use slint::RenderingState;
use std::cell::RefCell;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Steps of the drag, there and back.
const STEPS: usize = 240;

struct Drag {
    a: Point,
    b: Point,
    step: usize,
    renders: Vec<f64>,
    intervals: Vec<f64>,
    last: Option<Instant>,
}

struct Perf {
    out: PathBuf,
    /// Since the Unix epoch, when the process was started.
    t0: Option<Duration>,
    drag_after: Duration,
    live: bool,
    first: bool,
    render: Option<Instant>,
    drag: Option<Drag>,
    /// The steps written so far.
    marked: Vec<String>,
}

thread_local! {
    static PERF: RefCell<Option<Perf>> = const { RefCell::new(None) };
}

fn with<T>(f: impl FnOnce(&mut Perf) -> T) -> Option<T> {
    PERF.with(|p| p.borrow_mut().as_mut().map(f))
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Turns the timings on when `JUNC_GUI_PERF` names a file (GUI thread).
pub fn init() {
    let Some(out) = std::env::var_os("JUNC_GUI_PERF").filter(|v| !v.is_empty()) else {
        return;
    };
    let var = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<u64>().ok());
    let perf = Perf {
        out: PathBuf::from(out),
        t0: var("JUNC_GUI_PERF_T0").map(Duration::from_nanos),
        drag_after: Duration::from_millis(var("JUNC_GUI_PERF_DRAG_MS").unwrap_or(3000)),
        live: false,
        first: false,
        render: None,
        drag: None,
        marked: Vec::new(),
    };
    PERF.with(|p| *p.borrow_mut() = Some(perf));
    mark("main");
}

fn write(out: &PathBuf, line: &str) {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out);
    if let Ok(mut f) = file {
        let _ = writeln!(f, "{line}");
    }
}

/// Writes `<name>_ms`, the time since the process was started, once per
/// name: the steps of the cold start.
pub fn mark(name: &str) {
    with(|p| {
        if let Some(t0) = p.t0
            && let Ok(t) = SystemTime::now().duration_since(UNIX_EPOCH)
            && !p.marked.iter().any(|m| m == name)
        {
            p.marked.push(name.to_owned());
            write(
                &p.out,
                &format!("{name}_ms={:.1}", ms(t.saturating_sub(t0))),
            );
        }
    });
}

/// juncd's state arrived; `junctions` says whether it has any.
pub fn live(junctions: bool) {
    mark("state");
    if junctions {
        with(|p| p.live = true);
        mark("state_with_junctions");
    }
}

/// Slint's rendering notifier calls this for every frame.
pub fn frame(state: &RenderingState) {
    let next = with(|p| {
        let now = Instant::now();
        match state {
            RenderingState::BeforeRendering => {
                p.render = Some(now);
                false
            }
            RenderingState::AfterRendering => {
                if !p.marked.iter().any(|m| m == "frame") {
                    p.marked.push("frame".to_owned());
                    if let (Some(t0), Ok(t)) = (p.t0, SystemTime::now().duration_since(UNIX_EPOCH))
                    {
                        write(&p.out, &format!("frame_ms={:.1}", ms(t.saturating_sub(t0))));
                    }
                }
                if p.live && !p.first {
                    p.first = true;
                    if let Some(t0) = p.t0
                        && let Ok(t) = SystemTime::now().duration_since(UNIX_EPOCH)
                    {
                        write(
                            &p.out,
                            &format!("first_frame_ms={:.1}", ms(t.saturating_sub(t0))),
                        );
                    }
                    slint::Timer::single_shot(p.drag_after, start_drag);
                }
                let Some(d) = p.drag.as_mut() else {
                    return false;
                };
                if let Some(r) = p.render {
                    d.renders.push(ms(now - r));
                }
                if let Some(l) = d.last {
                    d.intervals.push(ms(now - l));
                }
                d.last = Some(now);
                true
            }
            _ => false,
        }
    });
    if next == Some(true) {
        // Not inside the notifier: the step changes the window.
        let _ = slint::invoke_from_event_loop(step);
    }
}

fn start_drag() {
    let Some(a) = app::current() else { return };
    let path = a.borrow().drag_path();
    let Some((from, start, end)) = path else {
        with(|p| write(&p.out, "drag_error=1\ndone=1"));
        return;
    };
    a.borrow_mut().link_start(from, false);
    with(|p| {
        p.drag = Some(Drag {
            a: start,
            b: end,
            step: 0,
            renders: Vec::new(),
            intervals: Vec::new(),
            last: None,
        });
    });
    step();
}

/// Moves the pointer one step further, or ends the drag.
fn step() {
    let Some(a) = app::current() else { return };
    let point = with(|p| {
        let d = p.drag.as_mut()?;
        if d.step >= STEPS {
            return None;
        }
        d.step += 1;
        // There and back: 0 → 1 → 0.
        #[allow(clippy::cast_precision_loss)]
        let t = 1.0 - (2.0 * d.step as f32 / STEPS as f32 - 1.0).abs();
        Some(Point {
            x: d.a.x + (d.b.x - d.a.x) * t,
            y: d.a.y + (d.b.y - d.a.y) * t,
        })
    })
    .flatten();
    let Some(point) = point else {
        a.borrow_mut().link_cancel();
        with(finish);
        return;
    };
    let mut app = a.borrow_mut();
    app.link_drag(point);
    app.request_frame();
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let i = ((v.len() as f64 * p).ceil() as usize).clamp(1, v.len()) - 1;
    v[i]
}

fn finish(p: &mut Perf) {
    let Some(mut d) = p.drag.take() else { return };
    let lines = [
        format!("drag_frames={}", d.renders.len()),
        format!("drag_render_p95_ms={:.2}", percentile(&mut d.renders, 0.95)),
        format!("drag_render_max_ms={:.2}", percentile(&mut d.renders, 1.0)),
        format!(
            "drag_interval_p95_ms={:.2}",
            percentile(&mut d.intervals, 0.95)
        ),
        "done=1".to_owned(),
    ];
    write(&p.out, &lines.join("\n"));
}
