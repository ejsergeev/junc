// SPDX-License-Identifier: GPL-3.0-or-later

//! MeterService (architecture §7.1, §7.6): the sidebar meters, fed by
//! [`junc_pw::meter::Meter`] through the window's ~7 Hz `Timer`, which runs
//! only while the [`Gate`] says the window is seen and something is
//! metered.

use crate::meters::{self, Ballistics, TICK};
use crate::ui::{self, AppWindow};
use crate::visibility::{self, Frames, Gate, Signal};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// With nothing changing on screen, a frame is still asked for this often
/// so that a hidden window is noticed (frame starvation).
const PROBE: Duration = Duration::from_secs(2);

/// The running service; the window's meters stop when it is dropped.
#[derive(Debug)]
pub struct MeterService {
    gate: Arc<Gate>,
    frames: Option<Arc<Frames>>,
}

impl MeterService {
    /// The nodes to meter (each Junction's source).
    pub fn set_nodes(&self, nodes: Vec<String>) {
        self.gate.set_nodes(nodes);
    }

    /// The gate, for signals from other threads (the daemon follower).
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

/// Starts the meters for `window`; `None` if the meter thread cannot start
/// (the window then shows no levels).
pub fn start(window: &AppWindow) -> Option<MeterService> {
    let meter = Arc::new(junc_pw::meter::Meter::spawn().ok()?);
    let weak = window.as_weak();
    let gate = Arc::new(Gate::new(Box::new(meter.clone()), move |running| {
        let _ = weak.upgrade_in_event_loop(move |w| {
            w.set_meters_running(running);
            if !running {
                // Shown again, the bars start from silence, not from
                // where they stopped.
                ui::set_levels(&w, &HashMap::new());
                // One frame stays asked for: never presented while the
                // window is hidden (no CPU), presented at once if the
                // starvation was a false alarm, which resumes the meters.
                w.window().request_redraw();
            }
        });
    }));

    // Frame starvation: only where the renderer reports frames (femtovg);
    // otherwise Hyprland and closing are the signals.
    let frames = Frames::spawn(gate.clone()).ok().and_then(|f| {
        let (f2, g2) = (f.clone(), gate.clone());
        let set = window.window().set_rendering_notifier(move |state, _| {
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

    let state: Rc<RefCell<(HashMap<String, Ballistics>, Instant)>> =
        Rc::new(RefCell::new((HashMap::new(), Instant::now())));
    let (weak, f) = (window.as_weak(), frames.clone());
    window.on_meter_tick(move || {
        let Some(w) = weak.upgrade() else { return };
        let mut st = state.borrow_mut();
        let (balls, asked) = &mut *st;
        let mut peaks = meters::loudest(meter.read());
        for (node, b) in balls.iter_mut() {
            b.tick(peaks.remove(node).unwrap_or(0.0), TICK);
        }
        for (node, p) in peaks {
            let mut b = Ballistics::default();
            b.tick(p, TICK);
            balls.insert(node, b);
        }
        let shown: HashMap<String, f32> = balls
            .iter()
            .map(|(n, b)| (n.clone(), b.shown().0))
            .collect();
        let changed = ui::set_levels(&w, &shown);
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
    Some(MeterService { gate, frames })
}
