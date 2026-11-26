// SPDX-License-Identifier: GPL-3.0-or-later

//! VisibilityService (architecture §7.6): meters run only while the window
//! can be seen. Three signals, each able to hide it on its own:
//!
//! - closed: the window was closed;
//! - Hyprland: its workspace is not shown on any monitor (another
//!   workspace, a hidden special workspace such as a minimized window) or a
//!   fullscreen window covers it — from Hyprland's IPC events, so no
//!   polling;
//! - starved: a frame was asked for and not presented within 500 ms
//!   (Wayland compositors stop frame callbacks to surfaces nobody sees),
//!   the fallback on other compositors.
//!
//! The [`Gate`] pauses the meter from whichever thread notices, so a GUI
//! thread blocked on a frame that never comes cannot keep the meters
//! running.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// A frame not presented this long after it was asked for means hidden.
pub const STARVED: Duration = Duration::from_millis(500);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Why the window may be hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Closed,
    Hyprland,
    Starved,
}

/// What the gate drives: the meter (`watch(&[])` pauses it).
pub trait Meters: Send + Sync {
    fn watch(&self, nodes: &[String]);
}

impl Meters for Arc<junc_pw::meter::Meter> {
    fn watch(&self, nodes: &[String]) {
        junc_pw::meter::Meter::watch(self, nodes);
    }
}

#[derive(Debug, Default)]
struct GateState {
    nodes: Vec<String>,
    closed: bool,
    hyprland: bool,
    starved: bool,
    /// What the meter was last told: running or not.
    running: bool,
}

impl GateState {
    fn visible(&self) -> bool {
        !(self.closed || self.hyprland || self.starved)
    }
}

/// Combines the signals and runs the meter only while visible and there is
/// something to meter.
pub struct Gate {
    state: Mutex<GateState>,
    meters: Box<dyn Meters>,
    /// Called when [`Gate::running`] changes, with its new value (from any
    /// thread).
    on_change: Box<dyn Fn(bool) + Send + Sync>,
}

impl std::fmt::Debug for Gate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gate")
            .field("state", &*lock(&self.state))
            .finish_non_exhaustive()
    }
}

impl Gate {
    pub fn new(meters: Box<dyn Meters>, on_change: impl Fn(bool) + Send + Sync + 'static) -> Self {
        Self {
            state: Mutex::new(GateState::default()),
            meters,
            on_change: Box::new(on_change),
        }
    }

    /// The nodes to meter while visible.
    pub fn set_nodes(&self, nodes: Vec<String>) {
        let mut s = lock(&self.state);
        if s.nodes == nodes {
            return;
        }
        s.nodes = nodes;
        if s.visible() {
            self.meters.watch(&s.nodes);
        }
        self.settle(s);
    }

    /// One signal says hidden (true) or not.
    pub fn set(&self, signal: Signal, hidden: bool) {
        let mut s = lock(&self.state);
        let was = s.visible();
        match signal {
            Signal::Closed => s.closed = hidden,
            Signal::Hyprland => s.hyprland = hidden,
            Signal::Starved => s.starved = hidden,
        }
        let now = s.visible();
        if was != now {
            self.meters.watch(if now { &s.nodes } else { &[] });
        }
        self.settle(s);
    }

    /// Reports a change of [`Gate::running`].
    fn settle(&self, mut s: MutexGuard<'_, GateState>) {
        let running = s.visible() && !s.nodes.is_empty();
        if running != s.running {
            s.running = running;
            drop(s);
            (self.on_change)(running);
        }
    }

    pub fn visible(&self) -> bool {
        lock(&self.state).visible()
    }

    /// Whether the meter runs (visible and something to meter).
    pub fn running(&self) -> bool {
        lock(&self.state).running
    }
}

#[derive(Debug, Default)]
struct FrameState {
    /// The oldest frame asked for and not presented yet.
    asked: Option<Instant>,
    stop: bool,
}

/// Frame-starvation watchdog: [`Frames::asked`] when the GUI changes what
/// it shows, [`Frames::presented`] after each frame; a frame missing for
/// [`STARVED`] marks the window hidden until the next one is presented.
/// Its thread blocks while nothing is asked for, so it wakes nobody while
/// hidden.
#[derive(Debug)]
pub struct Frames {
    state: Mutex<FrameState>,
    cv: Condvar,
}

impl Frames {
    /// Starts the watchdog thread for `gate`.
    pub fn spawn(gate: Arc<Gate>) -> std::io::Result<Arc<Self>> {
        let frames = Arc::new(Self {
            state: Mutex::new(FrameState::default()),
            cv: Condvar::new(),
        });
        let f = frames.clone();
        std::thread::Builder::new()
            .name("junc-frames".into())
            .spawn(move || f.watch(&gate))?;
        Ok(frames)
    }

    /// The GUI asked for a frame (changed a property, requested a redraw).
    pub fn asked(&self) {
        let mut s = lock(&self.state);
        if s.asked.is_none() {
            s.asked = Some(Instant::now());
            self.cv.notify_all();
        }
    }

    /// A frame was presented: the window is seen.
    pub fn presented(&self, gate: &Gate) {
        lock(&self.state).asked = None;
        gate.set(Signal::Starved, false);
    }

    /// Ends the watchdog thread.
    pub fn stop(&self) {
        lock(&self.state).stop = true;
        self.cv.notify_all();
    }

    fn watch(&self, gate: &Gate) {
        let mut s = lock(&self.state);
        loop {
            if s.stop {
                return;
            }
            let Some(asked) = s.asked else {
                s = self.cv.wait(s).unwrap_or_else(PoisonError::into_inner);
                continue;
            };
            let left = STARVED.saturating_sub(asked.elapsed());
            if !left.is_zero() {
                s = self
                    .cv
                    .wait_timeout(s, left)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
                continue;
            }
            // Starved: hidden until a frame is presented; nothing more to
            // watch until then.
            s.asked = None;
            drop(s);
            gate.set(Signal::Starved, true);
            s = lock(&self.state);
        }
    }
}

/// Hyprland's IPC sockets for this session, if it runs under Hyprland:
/// `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket{,2}.sock`.
pub fn hyprland_sockets() -> Option<(PathBuf, PathBuf)> {
    let sig = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    let dir = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?)
        .join("hypr")
        .join(sig);
    Some((dir.join(".socket.sock"), dir.join(".socket2.sock")))
}

/// One request on Hyprland's command socket (`j/clients`, `j/monitors`).
fn request(socket: &Path, cmd: &str) -> std::io::Result<String> {
    let mut s = UnixStream::connect(socket)?;
    s.set_read_timeout(Some(Duration::from_secs(1)))?;
    s.write_all(cmd.as_bytes())?;
    let mut out = String::new();
    s.read_to_string(&mut out)?;
    Ok(out)
}

/// Whether the windows of process `pid` can be seen, from Hyprland's
/// `j/clients` and `j/monitors`; `None` while it has no mapped window.
pub fn hyprland_visible(clients: &str, monitors: &str, pid: u32) -> Option<bool> {
    let clients: serde_json::Value = serde_json::from_str(clients).ok()?;
    let monitors: serde_json::Value = serde_json::from_str(monitors).ok()?;
    let id = |v: &serde_json::Value| v["id"].as_i64();
    let shown: Vec<i64> = monitors
        .as_array()?
        .iter()
        .flat_map(|m| [id(&m["activeWorkspace"]), id(&m["specialWorkspace"])])
        .flatten()
        .filter(|&w| w != 0)
        .collect();
    let all = clients.as_array()?;
    let mine: Vec<&serde_json::Value> = all
        .iter()
        .filter(|c| c["pid"].as_u64() == Some(u64::from(pid)) && c["mapped"] == true)
        .collect();
    if mine.is_empty() {
        return None;
    }
    Some(mine.iter().any(|c| {
        let Some(ws) = id(&c["workspace"]) else {
            return false;
        };
        let covered = c["fullscreen"].as_i64().unwrap_or(0) == 0
            && all.iter().any(|o| {
                o["pid"].as_u64() != Some(u64::from(pid))
                    && id(&o["workspace"]) == Some(ws)
                    && o["fullscreen"].as_i64().unwrap_or(0) > 0
            });
        c["hidden"] != true && shown.contains(&ws) && !covered
    }))
}

/// Events that cannot change what is shown where.
fn irrelevant(event: &str) -> bool {
    matches!(
        event,
        "activewindow"
            | "activewindowv2"
            | "activelayout"
            | "submap"
            | "urgent"
            | "windowtitle"
            | "windowtitlev2"
            | "screencast"
            | "bell"
    )
}

/// Follows Hyprland's events and reports whether this process's window can
/// be seen (on a change only). Returns `None` when not under Hyprland.
pub fn follow_hyprland(on_change: impl Fn(bool) + Send + 'static) -> Option<()> {
    let (commands, events) = hyprland_sockets()?;
    let stream = UnixStream::connect(&events).ok()?;
    let pid = std::process::id();
    let query = move || {
        let clients = request(&commands, "j/clients").ok()?;
        let monitors = request(&commands, "j/monitors").ok()?;
        hyprland_visible(&clients, &monitors, pid)
    };
    std::thread::Builder::new()
        .name("junc-hyprland".into())
        .spawn(move || {
            let mut last = None;
            let mut lines = BufReader::new(stream);
            let mut line = String::new();
            loop {
                line.clear();
                match lines.read_line(&mut line) {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {}
                }
                let event = line.split(">>").next().unwrap_or("");
                if irrelevant(event) {
                    continue;
                }
                // A burst of events (a workspace switch sends several) is
                // read before one query.
                while !lines.buffer().is_empty() {
                    line.clear();
                    if lines.read_line(&mut line).is_err() {
                        return;
                    }
                }
                let now = query();
                if now.is_some() && now != last {
                    last = now;
                    on_change(now.unwrap_or(true));
                }
            }
        })
        .ok()?;
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<Vec<String>>>,
    }
    impl Meters for Arc<Fake> {
        fn watch(&self, nodes: &[String]) {
            lock(&self.calls).push(nodes.to_vec());
        }
    }

    fn gate() -> (Gate, Arc<Fake>, Arc<AtomicUsize>) {
        let fake = Arc::new(Fake::default());
        let changes = Arc::new(AtomicUsize::new(0));
        let c = changes.clone();
        let g = Gate::new(Box::new(fake.clone()), move |_| {
            c.fetch_add(1, Ordering::SeqCst);
        });
        (g, fake, changes)
    }

    #[test]
    fn any_signal_hides_and_all_must_clear() {
        let (g, fake, changes) = gate();
        g.set_nodes(vec!["junc.a".into()]);
        assert!(g.running());
        g.set(Signal::Hyprland, true);
        g.set(Signal::Starved, true);
        assert!(!g.visible() && !g.running());
        g.set_nodes(vec!["junc.a".into(), "junc.b".into()]);
        g.set(Signal::Hyprland, false);
        assert!(!g.visible(), "still starved");
        g.set(Signal::Starved, false);
        assert!(g.running());
        let calls = lock(&fake.calls).clone();
        assert_eq!(
            calls,
            vec![
                vec!["junc.a".to_owned()],
                vec![],
                vec!["junc.a".to_owned(), "junc.b".to_owned()]
            ],
            "paused once, resumed once with the nodes set meanwhile"
        );
        assert_eq!(
            changes.load(Ordering::SeqCst),
            3,
            "running, paused, running"
        );
    }

    #[test]
    fn nothing_to_meter_means_not_running() {
        let (g, fake, _) = gate();
        g.set_nodes(Vec::new());
        assert!(g.visible() && !g.running());
        assert!(lock(&fake.calls).is_empty(), "same as before: no call");
    }

    #[test]
    fn a_missing_frame_hides_until_the_next_one() {
        let (g, _, _) = gate();
        let g = Arc::new(g);
        g.set_nodes(vec!["junc.a".into()]);
        let frames = Frames::spawn(g.clone()).expect("thread");
        frames.asked();
        frames.presented(&g);
        std::thread::sleep(STARVED + Duration::from_millis(200));
        assert!(g.visible(), "presented in time");
        frames.asked();
        std::thread::sleep(STARVED + Duration::from_millis(200));
        assert!(!g.visible(), "starved");
        frames.presented(&g);
        assert!(g.visible());
        frames.stop();
    }

    const MONITORS: &str = r#"[{"activeWorkspace":{"id":1},"specialWorkspace":{"id":0}},
        {"activeWorkspace":{"id":4},"specialWorkspace":{"id":-98}}]"#;

    fn client(pid: u32, ws: i64, fullscreen: i64) -> String {
        format!(
            r#"{{"pid":{pid},"mapped":true,"hidden":false,"workspace":{{"id":{ws}}},"fullscreen":{fullscreen}}}"#
        )
    }

    #[test]
    fn hyprland_shows_windows_on_shown_workspaces_only() {
        let at = |ws, others: &str| {
            let clients = format!("[{}{others}]", client(7, ws, 0));
            hyprland_visible(&clients, MONITORS, 7)
        };
        assert_eq!(at(1, ""), Some(true), "active workspace");
        assert_eq!(at(4, ""), Some(true), "active on the other monitor");
        assert_eq!(at(2, ""), Some(false), "another workspace");
        assert_eq!(at(-98, ""), Some(true), "special workspace shown");
        assert_eq!(at(-99, ""), Some(false), "special workspace hidden");
        let cover = format!(",{}", client(8, 1, 2));
        assert_eq!(at(1, &cover), Some(false), "a fullscreen window covers it");
        assert_eq!(hyprland_visible("[]", MONITORS, 7), None, "not mapped yet");
    }
}
