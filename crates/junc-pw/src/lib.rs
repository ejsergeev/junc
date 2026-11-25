// SPDX-License-Identifier: GPL-3.0-or-later

//! PipeWire implementation of the JUNC backend (docs/architecture.md §3).
//!
//! One thread owns every PipeWire object (they are `!Send`). It keeps a
//! [`junc_core::graph::Graph`] in sync with the registry, applies
//! [`Action`]s, and reports [`Event`]s through a callback. Graph snapshots
//! are sent once per batch of registry changes: after the first change a
//! `core.sync` is issued and the snapshot goes out when it completes, so a
//! burst of changes yields one event and an idle graph yields none (no timers).
//!
//! When PipeWire goes away the thread waits for its socket with inotify and
//! reconnects; objects JUNC created linger in the server and are adopted
//! again by the planner (§3.6).

mod session;
mod socket;

use junc_core::backend::Action;
use junc_core::graph::Graph;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

/// What the backend reports.
#[derive(Debug, Clone)]
pub enum Event {
    /// Connected to PipeWire (again).
    Connected,
    /// Lost PipeWire; waiting for it to come back.
    Disconnected,
    /// The visible graph after a batch of changes.
    Graph(Box<Graph>),
    /// An action failed (for logs and notices; the next plan retries).
    ActionFailed {
        /// The action, as text.
        action: String,
        /// Why.
        error: String,
    },
}

/// Commands to the PipeWire thread.
#[derive(Debug)]
enum Command {
    Apply(Vec<Action>),
    Refresh,
    Shutdown,
}

/// A running backend.
pub struct PwBackend {
    tx: pipewire::channel::Sender<Command>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for PwBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PwBackend").finish_non_exhaustive()
    }
}

impl PwBackend {
    /// Starts the PipeWire thread. `on_event` runs on that thread.
    pub fn spawn(on_event: impl Fn(Event) + Send + 'static) -> std::io::Result<Self> {
        pipewire::init();
        let (tx, rx) = pipewire::channel::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("junc-pw".into())
                .spawn(move || session::run(rx, &stop, on_event))?
        };
        Ok(Self {
            tx,
            stop,
            thread: Some(thread),
        })
    }

    /// Queues actions; they are applied in order on the PipeWire thread.
    pub fn apply(&self, actions: Vec<Action>) {
        let _ = self.tx.send(Command::Apply(actions));
    }

    /// Asks for a graph snapshot even if nothing changed.
    pub fn refresh(&self) {
        let _ = self.tx.send(Command::Refresh);
    }
}

impl Drop for PwBackend {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.tx.send(Command::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
