//! juncd — minimal JUNC daemon for the performance baseline (throwaway, but measured).
//!
//! Builds the same graph as ../graph-manual.sh as native PipeWire objects:
//!   junc.bus   : support.null-audio-sink, 4 ch, created server-side via the "adapter" factory (object.linger)
//!   inputs     : links  app(FL,FR) -> bus(FL,FR) for nodes whose application.name starts with JUNCD_APP_PREFIX,
//!                and   fake.hwin:capture_MONO -> bus FL+FR
//!   listen     : --listen=loopback (default) loads libpipewire-module-loopback in-process (like pw-loopback),
//!                --listen=links    links bus monitor ports straight to the output devices (no extra node)
//! Reconnects automatically when PipeWire restarts and re-creates everything.
//! Logs (stderr, one line each, monotonic ms):  EV <ms> <event> k=v ...
use pipewire as pw;
use pw::{properties::properties, types::ObjectType};
use std::{cell::RefCell, collections::HashMap, ffi::CString, rc::Rc, time::Instant};

unsafe extern "C" {
    fn pw_context_load_module(ctx: *mut pw::sys::pw_context, name: *const std::ffi::c_char,
        args: *const std::ffi::c_char, props: *mut pw::sys::pw_properties) -> *mut std::ffi::c_void;
}

#[derive(Clone, Debug)]
struct NodeInfo { name: String, app: String, class: String }
#[derive(Clone, Debug)]
struct PortInfo { node: u32, dir: String, name: String }

#[derive(Default)]
struct State {
    nodes: HashMap<u32, NodeInfo>,
    ports: HashMap<u32, PortInfo>,
    wanted_done: std::collections::HashSet<(u32, u32)>, // (out port, in port) we created
    links: Vec<pw::link::Link>,
    bus: Option<pw::node::Node>,
    app_seen_at: HashMap<u32, f64>,
    graph_complete_logged: bool,
}

fn ms(t0: &Instant) -> f64 { t0.elapsed().as_secs_f64() * 1000.0 }

fn main() {
    let t0 = Instant::now();
    let listen_links = std::env::args().any(|a| a == "--listen=links");
    let prefix = std::env::var("JUNCD_APP_PREFIX").unwrap_or_else(|_| "juncbench-app".into());
    pw::init();
    let mut attempt = 0u32;
    loop {
        match run_once(&t0, listen_links, &prefix) {
            // was connected and lost PipeWire: restart the backoff (bug in first run: attempt never reset,
            // so every reconnect after the first waited the 500 ms maximum)
            Ok(()) => { attempt = 0; }
            Err(e) => { if attempt % 50 == 0 { eprintln!("EV {:.1} connect_failed err={e}", ms(&t0)); } }
        }
        attempt += 1;
        // PipeWire gone (or not up yet): poll every 20 ms for ~2 s (a restart takes ~0.3 s), then every 500 ms.
        // (Exponential backoff was measured to add up to ~0.3 s; an inotify watch on $XDG_RUNTIME_DIR/pipewire-0
        // would make this instant and wakeup-free.)
        let backoff = if attempt < 100 { 20 } else { 500 };
        std::thread::sleep(std::time::Duration::from_millis(backoff));
    }
}

fn run_once(t0: &Instant, listen_links: bool, prefix: &str) -> Result<(), pw::Error> {
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let registry = Rc::new(core.get_registry_rc()?);
    eprintln!("EV {:.1} connected", ms(t0));
    let st = Rc::new(RefCell::new(State::default()));
    let t_connect = ms(t0);

    // --- on disconnect: leave this mainloop so main() reconnects ---
    let ml = mainloop.clone();
    let t0c = *t0;
    let _core_l = core.add_listener_local()
        .error(move |id, _seq, res, msg| {
            eprintln!("EV {:.1} core_error id={id} res={res} msg={msg}", ms(&t0c));
            if id == pw::core::PW_ID_CORE { ml.quit(); }
        })
        .register();

    // --- registry: track nodes/ports, react ---
    let st2 = st.clone();
    let core2 = core.clone();
    let t0r = *t0;
    let pfx = prefix.to_owned();
    let _reg_l = registry.add_listener_local()
        .global(move |g| {
            let Some(p) = g.props else { return };
            match g.type_ {
                ObjectType::Node => {
                    let info = NodeInfo {
                        name: p.get("node.name").unwrap_or("").into(),
                        app: p.get("application.name").unwrap_or("").into(),
                        class: p.get("media.class").unwrap_or("").into(),
                    };
                    let mut s = st2.borrow_mut();
                    if info.app.starts_with(&pfx) {
                        s.app_seen_at.insert(g.id, ms(&t0r));
                        eprintln!("EV {:.1} app_node id={} name={}", ms(&t0r), g.id, info.name);
                    }
                    s.nodes.insert(g.id, info);
                }
                ObjectType::Port => {
                    let node: u32 = p.get("node.id").and_then(|v| v.parse().ok()).unwrap_or(0);
                    let info = PortInfo { node, dir: p.get("port.direction").unwrap_or("").into(), name: p.get("port.name").unwrap_or("").into() };
                    st2.borrow_mut().ports.insert(g.id, info);
                    reconcile(&st2, &core2, &t0r, listen_links, &pfx);
                }
                ObjectType::Link => {
                    let out_node: u32 = p.get("link.output.node").and_then(|v| v.parse().ok()).unwrap_or(0);
                    let s = st2.borrow();
                    if let Some(t) = s.app_seen_at.get(&out_node) {
                        eprintln!("EV {:.1} app_linked node={out_node} relink_ms={:.2}", ms(&t0r), ms(&t0r) - t);
                    }
                }
                _ => {}
            }
        })
        .global_remove({
            let st3 = st.clone();
            move |id| {
                let mut s = st3.borrow_mut();
                s.nodes.remove(&id);
                s.app_seen_at.remove(&id);
                s.ports.remove(&id);
                s.wanted_done.retain(|(a, b)| *a != id && *b != id);
            }
        })
        .register();

    // initial roundtrip so we know whether junc.bus already exists (e.g. after daemon restart)
    roundtrip(&mainloop, &core);
    let have_bus = st.borrow().nodes.values().any(|n| n.name == "junc.bus");
    if !have_bus {
        let bus: pw::node::Node = core.create_object("adapter", &properties! {
            "factory.name" => "support.null-audio-sink",
            "node.name" => "junc.bus",
            "node.description" => "JUNC-Bus",
            "media.class" => "Audio/Sink",
            "audio.position" => "[ FL FR RL RR ]",
            "node.autoconnect" => "false",
            "object.linger" => "true",
        })?;
        st.borrow_mut().bus = Some(bus);
        eprintln!("EV {:.1} bus_created", ms(t0));
    }
    if !listen_links {
        // same as pw-loopback: the loopback lives in our process; WirePlumber links it via target.object
        for out in ["hp", "obs"] {
            let args = format!("{{ node.name=junc.listen.{out} audio.position=[FL FR] \
                capture.props={{ node.name=input.junc.listen.{out} target.object=junc.bus stream.capture.sink=true node.passive=true }} \
                playback.props={{ node.name=output.junc.listen.{out} target.object=fake.{out} }} }}");
            let name = CString::new("libpipewire-module-loopback").unwrap();
            let a = CString::new(args).unwrap();
            let m = unsafe { pw_context_load_module(context.as_raw_ptr(), name.as_ptr(), a.as_ptr(), std::ptr::null_mut()) };
            eprintln!("EV {:.1} loopback_loaded out={out} ok={}", ms(t0), !m.is_null());
        }
    }
    roundtrip(&mainloop, &core);
    reconcile(&st, &core, t0, listen_links, prefix);
    eprintln!("EV {:.1} setup_done since_connect_ms={:.1}", ms(t0), ms(t0) - t_connect);
    mainloop.run();
    eprintln!("EV {:.1} disconnected", ms(t0));
    Ok(())
}

/// Create any missing link between ports we care about.
fn reconcile(st: &Rc<RefCell<State>>, core: &pw::core::CoreRc, t0: &Instant, listen_links: bool, prefix: &str) {
    let mut want: Vec<(u32, u32)> = vec![];
    {
        let s = st.borrow();
        let node_by_name = |n: &str| s.nodes.iter().find(|(_, v)| v.name == n).map(|(k, _)| *k);
        let port = |node: u32, dir: &str, name: &str| s.ports.iter()
            .find(|(_, p)| p.node == node && p.dir == dir && p.name == name).map(|(k, _)| *k);
        let Some(bus) = node_by_name("junc.bus") else { return };
        // apps -> bus
        for (id, n) in &s.nodes {
            if n.app.starts_with(prefix) && n.class.starts_with("Stream/Output/Audio") {
                for ch in ["FL", "FR"] {
                    if let (Some(o), Some(i)) = (port(*id, "out", &format!("output_{ch}")), port(bus, "in", &format!("playback_{ch}"))) { want.push((o, i)); }
                }
            }
        }
        // hardware input (mono) -> bus FL + FR
        if let Some(hw) = node_by_name("fake.hwin") {
            for ch in ["FL", "FR"] {
                if let (Some(o), Some(i)) = (port(hw, "out", "capture_MONO"), port(bus, "in", &format!("playback_{ch}"))) { want.push((o, i)); }
            }
        }
        if listen_links {
            for out in ["fake.hp", "fake.obs"] {
                if let Some(dev) = node_by_name(out) {
                    for ch in ["FL", "FR"] {
                        if let (Some(o), Some(i)) = (port(bus, "out", &format!("monitor_{ch}")), port(dev, "in", &format!("playback_{ch}"))) { want.push((o, i)); }
                    }
                }
            }
        }
    }
    let mut s = st.borrow_mut();
    for (o, i) in want {
        if s.wanted_done.contains(&(o, i)) { continue; }
        let (on, inn) = (s.ports[&o].node, s.ports[&i].node);
        match core.create_object::<pw::link::Link>("link-factory", &properties! {
            "link.output.node" => on.to_string(), "link.output.port" => o.to_string(),
            "link.input.node" => inn.to_string(), "link.input.port" => i.to_string(),
            "object.linger" => "false",
        }) {
            Ok(l) => { s.links.push(l); s.wanted_done.insert((o, i)); }
            Err(e) => eprintln!("EV {:.1} link_error {o}->{i} {e}", ms(t0)),
        }
    }
    let n = s.wanted_done.len();
    if !s.graph_complete_logged && n >= if listen_links { 12 } else { 8 } {
        s.graph_complete_logged = true;
        eprintln!("EV {:.1} graph_complete links={n}", ms(t0));
    }
}

fn roundtrip(mainloop: &pw::main_loop::MainLoopRc, core: &pw::core::CoreRc) {
    let done = Rc::new(std::cell::Cell::new(false));
    let pending = core.sync(0).expect("sync");
    let (d, ml) = (done.clone(), mainloop.clone());
    let _l = core.add_listener_local().done(move |id, seq| {
        if id == pw::core::PW_ID_CORE && seq == pending { d.set(true); ml.quit(); }
    }).register();
    while !done.get() { mainloop.run(); }
}
