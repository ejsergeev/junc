// SPDX-License-Identifier: GPL-3.0-or-later

//! MeterService (architecture §7.1, §7.6): every meter on screen, fed by
//! [`junc_pw::meter::Meter`] through the window's ~7 Hz `Timer`, which runs
//! only while the [`Gate`] says the window is seen and something is
//! metered.
//!
//! The window numbers its meters: each is a [`Slot`] (a node's channel, or
//! its loudest channel for a sidebar row) with an index into two float
//! models, levels and peaks, which the window owns so that they read as
//! silence when nothing feeds them.

use crate::meters::{Ballistics, FLOOR, TICK};
use crate::ui::AppWindow;
use crate::visibility::{self, Frames, Gate, Signal};
use slint::{ComponentHandle, Model, VecModel};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// With nothing changing on screen, a frame is still asked for this often
/// so that a hidden window is noticed (frame starvation).
const PROBE: Duration = Duration::from_secs(2);
/// A tick this long after the previous one follows a pause: the bars start
/// again from silence rather than from where they stopped.
const RESUMED: Duration = Duration::from_secs(1);

/// What one meter shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Slot {
    pub node: String,
    /// 0-based channel; `None` for the loudest one.
    pub channel: Option<usize>,
}

/// The level and peak models (dBFS by slot index) the window reads.
#[derive(Clone)]
pub struct Levels {
    pub levels: Rc<VecModel<f32>>,
    pub peaks: Rc<VecModel<f32>>,
}

impl std::fmt::Debug for Levels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Levels")
            .field("slots", &self.levels.row_count())
            .finish()
    }
}

impl Default for Levels {
    fn default() -> Self {
        Self {
            levels: Rc::new(VecModel::default()),
            peaks: Rc::new(VecModel::default()),
        }
    }
}

impl Levels {
    /// `n` silent meters.
    pub fn reset(&self, n: usize) {
        self.levels.set_vec(vec![FLOOR; n]);
        self.peaks.set_vec(vec![FLOOR; n]);
    }

    /// Sets meter `i`; returns whether it changed.
    pub fn set(&self, i: usize, (level, peak): (f32, f32)) -> bool {
        let mut changed = false;
        for (model, v) in [(&self.levels, level), (&self.peaks, peak)] {
            if model
                .row_data(i)
                .is_some_and(|old| (old - v).abs() > f32::EPSILON)
            {
                model.set_row_data(i, v);
                changed = true;
            }
        }
        changed
    }
}

struct Inner {
    slots: Vec<Slot>,
    balls: Vec<Ballistics>,
    asked: Instant,
    ticked: Instant,
}

/// The running service; the meters stop when it is dropped.
pub struct MeterService {
    gate: Arc<Gate>,
    frames: Option<Arc<Frames>>,
    inner: Rc<RefCell<Inner>>,
}

impl std::fmt::Debug for MeterService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeterService")
            .field("gate", &self.gate)
            .finish_non_exhaustive()
    }
}

impl MeterService {
    /// The meters on screen now (the window has already sized its models).
    pub fn set_slots(&self, slots: &[Slot]) {
        let mut inner = self.inner.borrow_mut();
        if inner.slots == slots {
            return;
        }
        inner.slots = slots.to_vec();
        inner.balls = vec![Ballistics::default(); slots.len()];
        drop(inner);
        let mut nodes: Vec<String> = Vec::new();
        for s in slots {
            if !nodes.contains(&s.node) {
                nodes.push(s.node.clone());
            }
        }
        self.gate.set_nodes(nodes);
    }

    /// The gate, for signals from other threads.
    pub fn gate(&self) -> Arc<Gate> {
        self.gate.clone()
    }
}

impl Drop for MeterService {
    fn drop(&mut self) {
        if let Some(f) = &self.frames {
            f.stop();
        }
        self.gate.set(Signal::Closed, true);
    }
}

/// Starts the meters for `window`, writing into `levels`; `None` if the
/// meter thread cannot start (the window then shows silence).
pub fn start(window: &AppWindow, levels: &Levels) -> Option<MeterService> {
    let meter = Arc::new(junc_pw::meter::Meter::spawn().ok()?);
    let weak = window.as_weak();
    let gate = {
        let weak = weak.clone();
        Arc::new(Gate::new(Box::new(meter.clone()), move |running| {
            let _ = weak.upgrade_in_event_loop(move |w| {
                w.set_meters_running(running);
                if !running {
                    // One frame stays asked for: never presented while the
                    // window is hidden (no CPU), presented at once if the
                    // starvation was a false alarm, which resumes the
                    // meters.
                    w.window().request_redraw();
                }
            });
        }))
    };

    // Frame starvation: only where the renderer reports frames (femtovg);
    // otherwise Hyprland and closing are the signals.
    let frames = Frames::spawn(gate.clone()).ok().and_then(|f| {
        let (f2, g2) = (f.clone(), gate.clone());
        let set = window.window().set_rendering_notifier(move |state, _| {
            crate::perf::frame(&state);
            if matches!(state, slint::RenderingState::AfterRendering) {
                f2.presented(&g2);
            }
        });
        if set.is_ok() {
            Some(f)
        } else {
            f.stop();
            None
        }
    });

    {
        let g = gate.clone();
        window.window().on_close_requested(move || {
            g.set(Signal::Closed, true);
            slint::CloseRequestResponse::HideWindow
        });
    }
    {
        let g = gate.clone();
        // Not under Hyprland: no events, the other signals remain.
        let _ = visibility::follow_hyprland(move |visible| g.set(Signal::Hyprland, !visible));
    }

    let inner = Rc::new(RefCell::new(Inner {
        slots: Vec::new(),
        balls: Vec::new(),
        asked: Instant::now(),
        ticked: Instant::now(),
    }));
    let (state, f, lv) = (inner.clone(), frames.clone(), levels.clone());
    window.on_meter_tick(move || {
        let Some(w) = weak.upgrade() else { return };
        let mut st = state.borrow_mut();
        let Inner {
            slots,
            balls,
            asked,
            ticked,
        } = &mut *st;
        if ticked.elapsed() > RESUMED {
            balls.fill(Ballistics::default());
            lv.reset(slots.len());
        }
        *ticked = Instant::now();
        let read: HashMap<String, Vec<f32>> = meter.read().into_iter().collect();
        let mut changed = false;
        for (i, (slot, b)) in slots.iter().zip(balls.iter_mut()).enumerate() {
            let peak = read.get(&slot.node).map_or(0.0, |ch| match slot.channel {
                Some(c) => ch.get(c).copied().unwrap_or(0.0),
                None => ch.iter().copied().fold(0.0, f32::max),
            });
            b.tick(peak, TICK);
            changed |= lv.set(i, b.shown());
        }
        if let Some(f) = &f
            && (changed || asked.elapsed() > PROBE)
        {
            // Explicitly: a changed property of an item not drawn last frame
            // asks for no frame by itself.
            w.window().request_redraw();
            f.asked();
            *asked = Instant::now();
        }
    });
    Some(MeterService {
        gate,
        frames,
        inner,
    })
}
