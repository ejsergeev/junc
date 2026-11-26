// SPDX-License-Identifier: GPL-3.0-or-later

//! Level meters for the GUI (docs/architecture.md §7.6), in the GUI process
//! only: the daemon keeps 0 wakeups and the audio path is untouched.
//!
//! One capture stream, `junc-gui.meter`, has one input port per metered
//! channel; its RT callback writes the peak of each channel into atomics,
//! once per graph cycle however many meters there are. The thread links the
//! output ports of each watched node (a sink's are its monitor ports) to
//! those inputs: passive, non-lingering links tagged `junc.role = meter`,
//! which the daemon's planner leaves alone.
//!
//! Watching nothing disconnects from PipeWire: the stream, its links and the
//! RT callback are gone and the thread blocks, so a hidden window costs 0 %
//! CPU and PipeWire runs no extra node.
//!
//! pipewire-rs has no `pw_filter` binding; a stream with one port per
//! channel gives the same single callback per cycle without unsafe code.

use crate::socket;
use pipewire as pw;
use pw::proxy::ProxyT;
use pw::spa;
use pw::types::ObjectType;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

/// Channels one meter stream carries (`SPA_AUDIO_MAX_CHANNELS`); more are
/// not metered.
pub const MAX_CHANNELS: usize = 64;
/// `node.name` of the meter stream.
pub const NODE_NAME: &str = "junc-gui.meter";
/// `SPA_AUDIO_CHANNEL_AUX0`: the stream's channels are AUX0, AUX1, …
const AUX0: u32 = 0x1000;

/// What the GUI and the RT callback share.
struct Levels {
    /// Peak magnitude per stream channel since the last read, as `f32` bits:
    /// for non-negative floats the bit patterns order like the values, so
    /// `fetch_max` keeps the larger without a lock.
    peaks: [AtomicU32; MAX_CHANNELS],
    /// RT callbacks so far (the hidden-window gate counts them).
    callbacks: AtomicU64,
    /// Which stream channels belong to which watched node.
    layout: Mutex<Vec<(String, Vec<usize>)>>,
}

/// What the GUI asks for.
struct Control {
    taps: Mutex<Vec<String>>,
    changed: Condvar,
    stop: AtomicBool,
    /// Wakes the PipeWire loop while connected.
    wake: Mutex<Option<pw::channel::Sender<()>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The meter thread.
pub struct Meter {
    control: Arc<Control>,
    levels: Arc<Levels>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Meter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Meter").finish_non_exhaustive()
    }
}

impl Meter {
    /// Starts the thread; it stays idle (blocked, not connected) until
    /// [`Meter::watch`] names a node.
    pub fn spawn() -> std::io::Result<Self> {
        pw::init();
        let control = Arc::new(Control {
            taps: Mutex::new(Vec::new()),
            changed: Condvar::new(),
            stop: AtomicBool::new(false),
            wake: Mutex::new(None),
        });
        let levels = Arc::new(Levels {
            peaks: std::array::from_fn(|_| AtomicU32::new(0)),
            callbacks: AtomicU64::new(0),
            layout: Mutex::new(Vec::new()),
        });
        let thread = {
            let (c, l) = (control.clone(), levels.clone());
            std::thread::Builder::new()
                .name("junc-meter".into())
                .spawn(move || run(&c, &l))?
        };
        Ok(Self {
            control,
            levels,
            thread: Some(thread),
        })
    }

    /// Meters these nodes (by `node.name`), each with all its output ports.
    /// An empty list pauses: the stream and its links go away.
    pub fn watch(&self, nodes: &[String]) {
        let mut taps: Vec<String> = Vec::with_capacity(nodes.len());
        for n in nodes {
            if !taps.contains(n) {
                taps.push(n.clone());
            }
        }
        *lock(&self.control.taps) = taps;
        self.control.changed.notify_all();
        if let Some(tx) = lock(&self.control.wake).as_ref() {
            let _ = tx.send(());
        }
    }

    /// Peak magnitude (linear, 1.0 = full scale) of each channel of each
    /// metered node since the previous call. Nodes not found (yet) are left
    /// out.
    pub fn read(&self) -> Vec<(String, Vec<f32>)> {
        lock(&self.levels.layout)
            .iter()
            .map(|(name, slots)| {
                let peaks = slots
                    .iter()
                    .map(|&s| f32::from_bits(self.levels.peaks[s].swap(0, Ordering::Relaxed)))
                    .collect();
                (name.clone(), peaks)
            })
            .collect()
    }

    /// RT callbacks so far.
    pub fn callbacks(&self) -> u64 {
        self.levels.callbacks.load(Ordering::Relaxed)
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        self.control.stop.store(true, Ordering::SeqCst);
        self.control.changed.notify_all();
        if let Some(tx) = lock(&self.control.wake).as_ref() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Thread body: idle while nothing is watched, connected while something is.
fn run(control: &Arc<Control>, levels: &Arc<Levels>) {
    loop {
        {
            let mut taps = lock(&control.taps);
            while taps.is_empty() && !control.stop.load(Ordering::SeqCst) {
                taps = control
                    .changed
                    .wait(taps)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        }
        if control.stop.load(Ordering::SeqCst) || !socket::wait_for_socket(&control.stop) {
            return;
        }
        let lost = session(control, levels);
        lock(&levels.layout).clear();
        if lost {
            // The socket can linger for a moment after the server died.
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}

/// A port as the registry shows it.
#[derive(Debug, Clone)]
struct PortInfo {
    node: u32,
    output: bool,
    index: u32,
    channel: Option<String>,
}

/// A link we asked for.
struct Made {
    _proxy: pw::link::Link,
    _listener: pw::proxy::ProxyListener,
    global: Rc<Cell<Option<u32>>>,
    /// Failed or removed: make it again if still wanted.
    gone: Rc<Cell<bool>>,
}

/// The capture stream and its listener.
struct Capture {
    stream: pw::stream::StreamRc,
    _listener: pw::stream::StreamListener<()>,
    channels: usize,
}

#[derive(Default)]
struct State {
    nodes: HashMap<u32, String>,
    ports: HashMap<u32, PortInfo>,
    capture: Option<Capture>,
    /// Our links by (output port, input port).
    links: HashMap<(u32, u32), Made>,
    sync: Option<spa::utils::result::AsyncSeq>,
    lost: bool,
}

type Shared = Rc<RefCell<State>>;

/// One connection; returns true if PipeWire went away.
fn session(control: &Arc<Control>, levels: &Arc<Levels>) -> bool {
    let Ok(mainloop) = pw::main_loop::MainLoopRc::new(None) else {
        return true;
    };
    let Ok(context) = pw::context::ContextRc::new(&mainloop, None) else {
        return true;
    };
    let Ok(core) = context.connect_rc(None) else {
        return true;
    };
    let Ok(registry) = core.get_registry_rc() else {
        return true;
    };
    let st: Shared = Rc::new(RefCell::new(State::default()));
    let taps: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(lock(&control.taps).clone()));

    let _core_listener = {
        let (st, ml) = (st.clone(), mainloop.downgrade());
        let (st_done, taps, core_w, reg_w, lv) = (
            st.clone(),
            taps.clone(),
            core.downgrade(),
            registry.downgrade(),
            levels.clone(),
        );
        core.add_listener_local()
            .done(move |id, seq| {
                if id != pw::core::PW_ID_CORE || st_done.borrow().sync != Some(seq) {
                    return;
                }
                st_done.borrow_mut().sync = None;
                if let (Some(core), Some(reg)) = (core_w.upgrade(), reg_w.upgrade()) {
                    reconcile(&st_done, &core, &reg, &taps.borrow(), &lv);
                }
            })
            .error(move |id, _seq, res, _msg| {
                if id == pw::core::PW_ID_CORE && res == -libc::EPIPE {
                    st.borrow_mut().lost = true;
                    if let Some(ml) = ml.upgrade() {
                        ml.quit();
                    }
                }
            })
            .register()
    };

    let _registry_listener = {
        let (st_add, core_add, st_rm, core_rm) =
            (st.clone(), core.downgrade(), st.clone(), core.downgrade());
        registry
            .add_listener_local()
            .global(move |g| {
                let props = g.props;
                let get = |k: &str| props.and_then(|p| p.get(k));
                let num = |k: &str| get(k).and_then(|v| v.parse::<u32>().ok());
                let mut s = st_add.borrow_mut();
                match g.type_ {
                    ObjectType::Node => {
                        if let Some(name) = get("node.name") {
                            s.nodes.insert(g.id, name.to_owned());
                        }
                    }
                    ObjectType::Port => {
                        // Audio ports only (a node may also have MIDI or
                        // control ports).
                        if get("format.dsp").is_some_and(|f| !f.contains("audio")) {
                            return;
                        }
                        let Some(node) = num("node.id") else { return };
                        s.ports.insert(
                            g.id,
                            PortInfo {
                                node,
                                output: get("port.direction") == Some("out"),
                                index: num("port.id").unwrap_or(0),
                                channel: get("audio.channel").map(str::to_owned),
                            },
                        );
                    }
                    _ => return,
                }
                drop(s);
                if let Some(core) = core_add.upgrade() {
                    request_sync(&st_add, &core);
                }
            })
            .global_remove(move |id| {
                let mut s = st_rm.borrow_mut();
                s.nodes.remove(&id);
                s.ports.remove(&id);
                for m in s.links.values() {
                    if m.global.get() == Some(id) {
                        m.gone.set(true);
                    }
                }
                drop(s);
                if let Some(core) = core_rm.upgrade() {
                    request_sync(&st_rm, &core);
                }
            })
            .register()
    };

    let (tx, rx) = pw::channel::channel::<()>();
    *lock(&control.wake) = Some(tx);
    let attached = {
        let (st, ml, core, taps) = (st.clone(), mainloop.downgrade(), core.clone(), taps.clone());
        let control = control.clone();
        rx.attach(mainloop.loop_(), move |()| {
            let now = lock(&control.taps).clone();
            if now.is_empty() || control.stop.load(Ordering::SeqCst) {
                if let Some(ml) = ml.upgrade() {
                    ml.quit();
                }
                return;
            }
            *taps.borrow_mut() = now;
            request_sync(&st, &core);
        })
    };
    // Taps may have emptied before the wake sender was in place.
    if lock(&control.taps).is_empty() || control.stop.load(Ordering::SeqCst) {
        *lock(&control.wake) = None;
        let _ = attached.deattach();
        return false;
    }
    request_sync(&st, &core);
    mainloop.run();

    *lock(&control.wake) = None;
    let lost = st.borrow().lost;
    // Links and the stream go before the core they belong to; the server
    // drops whatever is left when the connection closes.
    let capture = {
        let mut s = st.borrow_mut();
        s.links.clear();
        s.capture.take()
    };
    if let Some(c) = capture {
        let _ = c.stream.disconnect();
    }
    let _ = attached.deattach();
    lost
}

/// Asks for a round trip; [`reconcile`] runs when it completes, once per
/// batch of registry changes.
fn request_sync(st: &Shared, core: &pw::core::CoreRc) {
    let mut s = st.borrow_mut();
    if s.sync.is_none()
        && let Ok(seq) = core.sync(0)
    {
        s.sync = Some(seq);
    }
}

/// Which output ports to meter, in stream-channel order, per watched node.
fn layout(s: &State, taps: &[String]) -> Vec<(String, Vec<u32>)> {
    let mut out = Vec::new();
    let mut used = 0;
    for name in taps {
        let Some((&node, _)) = s.nodes.iter().find(|(_, n)| *n == name) else {
            continue;
        };
        let mut ports: Vec<(u32, u32)> = s
            .ports
            .iter()
            .filter(|(_, p)| p.node == node && p.output)
            .map(|(&id, p)| (p.index, id))
            .collect();
        ports.sort_unstable();
        ports.truncate(MAX_CHANNELS - used);
        used += ports.len();
        if !ports.is_empty() {
            out.push((name.clone(), ports.into_iter().map(|(_, id)| id).collect()));
        }
    }
    out
}

/// The stream's input port for channel `k`, once the server has made it.
fn input_port(s: &State, own: u32, k: usize) -> Option<u32> {
    let aux = format!("AUX{k}");
    s.ports
        .iter()
        .find(|(_, p)| p.node == own && !p.output && p.channel.as_deref() == Some(aux.as_str()))
        .or_else(|| {
            s.ports.iter().find(|(_, p)| {
                p.node == own && !p.output && usize::try_from(p.index).ok() == Some(k)
            })
        })
        .map(|(&id, _)| id)
}

/// Brings the stream and its links in line with the watched nodes.
fn reconcile(
    st: &Shared,
    core: &pw::core::CoreRc,
    reg: &pw::registry::RegistryRc,
    taps: &[String],
    levels: &Arc<Levels>,
) {
    let wanted = layout(&st.borrow(), taps);
    let channels: usize = wanted.iter().map(|(_, p)| p.len()).sum();
    if channels == 0 {
        // Nothing to meter yet: no stream, no RT callback.
        let capture = {
            let mut s = st.borrow_mut();
            s.links.clear();
            s.capture.take()
        };
        if let Some(c) = capture {
            let _ = c.stream.disconnect();
        }
        lock(&levels.layout).clear();
        return;
    }
    let grow = st
        .borrow()
        .capture
        .as_ref()
        .is_none_or(|c| c.channels < channels);
    if grow {
        let old = {
            let mut s = st.borrow_mut();
            s.links.clear();
            s.capture.take()
        };
        if let Some(c) = old {
            let _ = c.stream.disconnect();
        }
        match capture(core, channels, levels) {
            Ok(c) => st.borrow_mut().capture = Some(c),
            Err(e) => {
                eprintln!("junc-meter: cannot create the meter stream: {e}");
                return;
            }
        }
    }

    let mut slots = Vec::new();
    let mut desired: Vec<(u32, u32, u32, u32)> = Vec::new();
    {
        let s = st.borrow();
        let own = s.capture.as_ref().map(|c| c.stream.node_id());
        let mut k = 0;
        for (name, ports) in &wanted {
            let mut mine = Vec::new();
            for &out in ports {
                mine.push(k);
                if let (Some(own), Some(p)) = (own, s.ports.get(&out))
                    && own != u32::MAX
                    && let Some(inp) = input_port(&s, own, k)
                {
                    desired.push((p.node, out, own, inp));
                }
                k += 1;
            }
            slots.push((name.clone(), mine));
        }
    }
    *lock(&levels.layout) = slots;

    let mut s = st.borrow_mut();
    s.links.retain(|pair, m| {
        let keep = !m.gone.get() && desired.iter().any(|d| (d.1, d.3) == *pair);
        if !keep && let Some(id) = m.global.get() {
            let _ = reg.destroy_global(id);
        }
        keep
    });
    for (onode, out, inode, inp) in desired {
        if s.links.contains_key(&(out, inp)) {
            continue;
        }
        let props = pw::properties::properties! {
            "link.output.node" => onode.to_string(),
            "link.output.port" => out.to_string(),
            "link.input.node" => inode.to_string(),
            "link.input.port" => inp.to_string(),
            "object.linger" => "false",
            "link.passive" => "true",
            "junc.role" => "meter",
        };
        let Ok(link) = core.create_object::<pw::link::Link>("link-factory", &props) else {
            continue;
        };
        let global = Rc::new(Cell::new(None));
        let gone = Rc::new(Cell::new(false));
        let listener = {
            let (g, gone1, gone2) = (global.clone(), gone.clone(), gone.clone());
            link.upcast_ref()
                .add_listener_local()
                .bound(move |id| g.set(Some(id)))
                .removed(move || gone1.set(true))
                .error(move |_, _, _| gone2.set(true))
                .register()
        };
        s.links.insert(
            (out, inp),
            Made {
                _proxy: link,
                _listener: listener,
                global,
                gone,
            },
        );
    }
}

/// The capture stream with `channels` AUX channels, not linked by the
/// session manager.
fn capture(
    core: &pw::core::CoreRc,
    channels: usize,
    levels: &Arc<Levels>,
) -> Result<Capture, pw::Error> {
    let props = pw::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::NODE_NAME => NODE_NAME,
        *pw::keys::NODE_DESCRIPTION => "JUNC meters",
        // Its links never keep a driver running; it only listens in.
        "node.passive" => "true",
        "stream.dont-remix" => "true",
        "junc.role" => "meter",
    };
    let stream = pw::stream::StreamRc::new(core.clone(), NODE_NAME, props)?;
    let lv = levels.clone();
    let listener = stream
        .add_local_listener_with_user_data(())
        .process(move |stream, ()| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            lv.callbacks.fetch_add(1, Ordering::Relaxed);
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let (offset, size) = (data.chunk().offset() as usize, data.chunk().size() as usize);
            let Some(bytes) = data.data() else { return };
            let Some(bytes) = bytes.get(offset..offset.saturating_add(size)) else {
                return;
            };
            peaks(bytes, channels, &lv.peaks);
        })
        .register()?;

    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    // At most MAX_CHANNELS (64): fits u32.
    #[allow(clippy::cast_possible_truncation)]
    info.set_channels(channels as u32);
    let mut position = [0u32; MAX_CHANNELS];
    for (k, p) in (0u32..).zip(position.iter_mut().take(channels)) {
        *p = AUX0 + k;
    }
    info.set_position(position);
    let obj = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: info.into(),
    };
    let bytes = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(obj),
    )
    .map_err(|_| pw::Error::CreationFailed)?
    .0
    .into_inner();
    let pod = spa::pod::Pod::from_bytes(&bytes).ok_or(pw::Error::CreationFailed)?;
    stream.connect(
        spa::utils::Direction::Input,
        None,
        pw::stream::StreamFlags::MAP_BUFFERS | pw::stream::StreamFlags::RT_PROCESS,
        &mut [pod],
    )?;
    Ok(Capture {
        stream,
        _listener: listener,
        channels,
    })
}

/// Raises each channel's peak to the largest magnitude in an interleaved
/// `f32` block. Lock-free and allocation-free: it runs in the RT thread.
fn peaks(bytes: &[u8], channels: usize, out: &[AtomicU32; MAX_CHANNELS]) {
    if channels == 0 {
        return;
    }
    let mut max = [0f32; MAX_CHANNELS];
    for (i, s) in bytes.as_chunks::<4>().0.iter().enumerate() {
        let v = f32::from_le_bytes(*s).abs();
        let c = i % channels;
        if v > max[c] {
            max[c] = v;
        }
    }
    for (slot, m) in out.iter().zip(max.iter().take(channels)) {
        if m.is_finite() && *m > 0.0 {
            slot.fetch_max(m.to_bits(), Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(frames: &[[f32; 2]]) -> Vec<u8> {
        frames
            .iter()
            .flat_map(|f| f.iter().flat_map(|v| v.to_le_bytes()))
            .collect()
    }

    #[test]
    fn peaks_keep_the_largest_magnitude_per_channel() {
        let out: [AtomicU32; MAX_CHANNELS] = std::array::from_fn(|_| AtomicU32::new(0));
        peaks(&block(&[[0.1, -0.5], [-0.3, 0.2]]), 2, &out);
        peaks(&block(&[[0.2, 0.1]]), 2, &out);
        let get = |c: usize| f32::from_bits(out[c].load(Ordering::Relaxed));
        assert!((get(0) - 0.3).abs() < 1e-6, "{}", get(0));
        assert!((get(1) - 0.5).abs() < 1e-6, "{}", get(1));
        assert!(get(2).abs() < f32::EPSILON);
        peaks(&block(&[[f32::NAN, f32::INFINITY]]), 2, &out);
        assert!(
            (get(1) - 0.5).abs() < 1e-6,
            "non-finite samples are ignored"
        );
    }

    #[test]
    fn layout_follows_the_watch_order_and_port_index() {
        let mut s = State::default();
        s.nodes.insert(10, "junc.a".into());
        s.nodes.insert(20, "junc.b".into());
        for (id, node, output, index) in [
            (101, 10, true, 1),
            (100, 10, true, 0),
            (102, 10, false, 0),
            (200, 20, true, 0),
        ] {
            s.ports.insert(
                id,
                PortInfo {
                    node,
                    output,
                    index,
                    channel: None,
                },
            );
        }
        let l = layout(
            &s,
            &["junc.b".into(), "junc.missing".into(), "junc.a".into()],
        );
        assert_eq!(
            l,
            vec![
                ("junc.b".to_owned(), vec![200]),
                ("junc.a".to_owned(), vec![100, 101])
            ]
        );
    }
}
