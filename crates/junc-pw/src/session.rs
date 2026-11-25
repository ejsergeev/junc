// SPDX-License-Identifier: GPL-3.0-or-later

//! One connection to PipeWire: registry → graph, actions → objects.

use crate::{Command, Event, socket};
use junc_core::backend::Action;
use junc_core::graph::{Direction, Graph, Link, Node, Port, Props};
use pipewire as pw;
use pw::proxy::{Listener, ProxyT};
use pw::types::ObjectType;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Still connected.
    Running,
    /// PipeWire went away.
    Lost,
    /// The owner asked us to stop.
    Shutdown,
}

/// A bound proxy and its listeners; dropping it unbinds.
struct Bound {
    _proxy: Option<Box<dyn ProxyT>>,
    /// Nodes keep their proxy typed, to set their volume.
    node: Option<pw::node::Node>,
    /// Cards keep theirs, to switch profiles (F15).
    device: Option<Rc<pw::device::Device>>,
    _listeners: Vec<Box<dyn Listener>>,
}

/// A create request in flight: creation is asynchronous, so a snapshot taken
/// before the object's global appears would make the planner ask again.
struct Created {
    _proxy: Box<dyn ProxyT>,
    _listener: pw::proxy::ProxyListener,
    /// Global id once the server bound it.
    global: Rc<std::cell::Cell<Option<u32>>>,
    /// Failed, removed, or its global already went away.
    done: Rc<std::cell::Cell<bool>>,
}

impl Created {
    fn pending(&self, g: &Graph) -> bool {
        !self.done.get()
            && self
                .global
                .get()
                .is_none_or(|id| !g.nodes.contains_key(&id) && !g.links.contains_key(&id))
    }
}

struct State {
    graph: Graph,
    bound: HashMap<u32, Bound>,
    /// Objects we asked the server to create and have not seen yet.
    created: Vec<Created>,
    sync: Option<pw::spa::utils::result::AsyncSeq>,
    /// Bound proxies whose first info (with the full props) has not arrived:
    /// a snapshot without it would show JUNC's own objects as untagged.
    awaiting_info: std::collections::HashSet<u32>,
    /// The `default` metadata, to set default devices (F16).
    default_md: Option<(u32, pw::metadata::Metadata)>,
    /// Round trip issued right after binding the metadata: its properties
    /// all arrive before it completes.
    md_sync: Option<pw::spa::utils::result::AsyncSeq>,
    outcome: Outcome,
}

type Shared = Rc<RefCell<State>>;

fn debug() -> bool {
    std::env::var_os("JUNC_PW_DEBUG").is_some()
}
type OnEvent = Rc<dyn Fn(Event)>;

/// Thread body: connect, serve, wait for the socket, reconnect.
pub(crate) fn run(
    mut rx: pw::channel::Receiver<Command>,
    stop: &AtomicBool,
    on_event: impl Fn(Event) + 'static,
) {
    let on_event: OnEvent = Rc::new(on_event);
    loop {
        if !socket::wait_for_socket(stop) {
            return;
        }
        let (back, outcome) = serve(rx, &on_event);
        rx = back;
        match outcome {
            Outcome::Shutdown => return,
            Outcome::Running | Outcome::Lost => {
                on_event(Event::Disconnected);
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                // The socket can linger for a moment after the server died.
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    }
}

fn serve(
    rx: pw::channel::Receiver<Command>,
    on_event: &OnEvent,
) -> (pw::channel::Receiver<Command>, Outcome) {
    let Ok(mainloop) = pw::main_loop::MainLoopRc::new(None) else {
        return (rx, Outcome::Lost);
    };
    let Ok(context) = pw::context::ContextRc::new(&mainloop, None) else {
        return (rx, Outcome::Lost);
    };
    let Ok(core) = context.connect_rc(None) else {
        return (rx, Outcome::Lost);
    };
    let Ok(registry) = core.get_registry_rc() else {
        return (rx, Outcome::Lost);
    };

    let state: Shared = Rc::new(RefCell::new(State {
        graph: Graph::default(),
        bound: HashMap::new(),
        created: Vec::new(),
        sync: None,
        awaiting_info: std::collections::HashSet::new(),
        default_md: None,
        md_sync: None,
        outcome: Outcome::Running,
    }));

    let _core_listener = {
        let (st, ml, ev) = (state.clone(), mainloop.downgrade(), on_event.clone());
        let (st2, ev2, core_done) = (state.clone(), on_event.clone(), core.downgrade());
        core.add_listener_local()
            .done(move |id, seq| {
                if debug() {
                    eprintln!(
                        "junc-pw: done id={id} seq={seq:?} pending={:?}",
                        st2.borrow().sync
                    );
                }
                if id != pw::core::PW_ID_CORE {
                    return;
                }
                let snapshot = {
                    let mut s = st2.borrow_mut();
                    if s.md_sync == Some(seq) {
                        s.md_sync = None;
                        if let Some((mid, _)) = &s.default_md {
                            let mid = *mid;
                            s.awaiting_info.remove(&mid);
                        }
                        // A snapshot held for it can go out now.
                        if s.sync.is_none() {
                            drop(s);
                            if let Some(core) = core_done.upgrade() {
                                request_sync(&st2, &core);
                            }
                        }
                        return;
                    }
                    if s.sync != Some(seq) {
                        return;
                    }
                    s.sync = None;
                    if !s.awaiting_info.is_empty() {
                        // Not complete yet; the info listener requests the
                        // next sync once the props are in.
                        return;
                    }
                    if s.created.iter().any(|c| c.pending(&s.graph)) {
                        // Wait for our new objects; their bound/global
                        // events request the next sync.
                        return;
                    }
                    s.created.clear();
                    if debug() {
                        let n = s
                            .graph
                            .nodes
                            .values()
                            .filter(|n| n.name().starts_with("junc"))
                            .count();
                        eprintln!(
                            "junc-pw: snapshot nodes={} junc_named={n}",
                            s.graph.nodes.len()
                        );
                    }
                    s.graph.clone()
                };
                ev2(Event::Graph(Box::new(snapshot)));
            })
            .error(move |id, _seq, res, msg| {
                // Only a broken pipe means the connection is gone; other core
                // errors (e.g. "no global" when destroying an object that just
                // went away) are about a single request.
                if id == pw::core::PW_ID_CORE && res == -libc::EPIPE {
                    st.borrow_mut().outcome = Outcome::Lost;
                    if let Some(ml) = ml.upgrade() {
                        ml.quit();
                    }
                } else {
                    // An object we asked for could not be made (bad port, …).
                    // ENOENT: it, or a port it needed, is already gone;
                    // EEXIST: the link exists. Both settle on the next plan.
                    // EINVAL "unknown port": the app stream it linked just left.
                    if res == -libc::ENOENT
                        || res == -libc::EEXIST
                        || (res == -libc::EINVAL && msg.contains("unknown"))
                    {
                        if debug() {
                            eprintln!("junc-pw: benign error id={id}: {msg}");
                        }
                        return;
                    }
                    ev(Event::ActionFailed {
                        action: format!("object {id}"),
                        error: format!("{msg} ({res})"),
                    });
                }
            })
            .register()
    };

    let _registry_listener = {
        let (st, reg, core_w, st_rm, core_rm) = (
            state.clone(),
            registry.downgrade(),
            core.downgrade(),
            state.clone(),
            core.downgrade(),
        );
        registry
            .add_listener_local()
            .global(move |g| {
                let (Some(reg), Some(core)) = (reg.upgrade(), core_w.upgrade()) else {
                    return;
                };
                on_global(&st, &reg, &core, g);
            })
            .global_remove(move |id| {
                if debug() {
                    eprintln!("junc-pw: global_remove {id}");
                }
                let mut s = st_rm.borrow_mut();
                let g = &mut s.graph;
                g.nodes.remove(&id);
                g.targets.remove(&id);
                g.ports.remove(&id);
                g.links.remove(&id);
                g.devices.remove(&id);
                s.bound.remove(&id);
                for c in &s.created {
                    if c.global.get() == Some(id) {
                        c.done.set(true);
                    }
                }
                s.awaiting_info.remove(&id);
                if s.default_md.as_ref().is_some_and(|(mid, _)| *mid == id) {
                    s.default_md = None;
                }
                drop(s);
                if let Some(core) = core_rm.upgrade() {
                    request_sync(&st_rm, &core);
                }
            })
            .register()
    };

    let attached = {
        let (st, ml, ev, core, reg) = (
            state.clone(),
            mainloop.downgrade(),
            on_event.clone(),
            core.clone(),
            registry.clone(),
        );
        rx.attach(mainloop.loop_(), move |cmd| match cmd {
            Command::Shutdown => {
                st.borrow_mut().outcome = Outcome::Shutdown;
                if let Some(ml) = ml.upgrade() {
                    ml.quit();
                }
            }
            Command::Refresh => request_sync(&st, &core),
            Command::Apply(actions) => {
                for a in &actions {
                    if let Err(e) = apply(&st, &core, &reg, a) {
                        ev(Event::ActionFailed {
                            action: a.to_string(),
                            error: e,
                        });
                    }
                }
                request_sync(&st, &core);
            }
        })
    };

    on_event(Event::Connected);
    request_sync(&state, &core);
    mainloop.run();
    let outcome = state.borrow().outcome;
    // Proxies must go before the core they belong to.
    state.borrow_mut().bound.clear();
    state.borrow_mut().default_md = None;
    state.borrow_mut().created.clear();
    (attached.deattach(), outcome)
}

/// Asks for a round trip; the graph snapshot goes out when it completes.
fn request_sync(st: &Shared, core: &pw::core::CoreRc) {
    let mut s = st.borrow_mut();
    if s.sync.is_none()
        && let Ok(seq) = core.sync(0)
    {
        s.sync = Some(seq);
    }
}

fn to_props(dict: &pw::spa::utils::dict::DictRef) -> Props {
    dict.iter().collect()
}

fn merge(props: &mut Props, dict: &pw::spa::utils::dict::DictRef) {
    for (k, v) in dict.iter() {
        props.set(k, v);
    }
}

fn on_global(
    st: &Shared,
    reg: &pw::registry::RegistryRc,
    core: &pw::core::CoreRc,
    g: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
) {
    let id = g.id;
    let props = g.props.map(to_props).unwrap_or_default();
    if debug() {
        eprintln!(
            "junc-pw: global {id} {:?} {}",
            g.type_,
            props.get("node.name").unwrap_or("")
        );
    }
    match g.type_ {
        ObjectType::Node => {
            st.borrow_mut().graph.nodes.insert(id, Node { id, props });
            // Globals lack junc.* tags and app keys (spike S9): bind for the
            // full info props.
            if let Ok(node) = reg.bind::<pw::node::Node, _>(g) {
                let (st2, core2) = (st.clone(), core.downgrade());
                let l = node
                    .add_listener_local()
                    .info(move |info| {
                        // Only PROPS changes carry the full set; merge, never replace.
                        if info.change_mask().contains(pw::node::NodeChangeMask::PROPS)
                            && let Some(p) = info.props()
                        {
                            if let Some(n) = st2.borrow_mut().graph.nodes.get_mut(&id) {
                                merge(&mut n.props, p);
                            }
                            st2.borrow_mut().awaiting_info.remove(&id);
                            if let Some(core) = core2.upgrade() {
                                request_sync(&st2, &core);
                            }
                        }
                    })
                    .register();
                st.borrow_mut().awaiting_info.insert(id);
                st.borrow_mut().bound.insert(
                    id,
                    Bound {
                        _proxy: None,
                        node: Some(node),
                        device: None,
                        _listeners: vec![Box::new(l)],
                    },
                );
            }
        }
        ObjectType::Port => {
            let Some(node) = props.number("node.id") else {
                return;
            };
            let direction = if props.get("port.direction") == Some("in") {
                Direction::In
            } else {
                Direction::Out
            };
            let port = Port {
                id,
                node,
                direction,
                index: props.number("port.id").unwrap_or(0),
                channel: props.get("audio.channel").map(str::to_owned),
                monitor: props.flag("port.monitor"),
                name: props.get("port.name").unwrap_or("").to_owned(),
            };
            st.borrow_mut().graph.ports.insert(id, port);
        }
        ObjectType::Link => {
            let num = |k| props.number(k);
            let (Some(on), Some(op), Some(inn), Some(ip)) = (
                num("link.output.node"),
                num("link.output.port"),
                num("link.input.node"),
                num("link.input.port"),
            ) else {
                return;
            };
            st.borrow_mut().graph.links.insert(
                id,
                Link {
                    id,
                    output_node: on,
                    output_port: op,
                    input_node: inn,
                    input_port: ip,
                    props: props.clone(),
                },
            );
            if let Ok(link) = reg.bind::<pw::link::Link, _>(g) {
                let (st2, core2) = (st.clone(), core.downgrade());
                let l = link
                    .add_listener_local()
                    .info(move |info| {
                        if info.change_mask().contains(pw::link::LinkChangeMask::PROPS)
                            && let Some(p) = info.props()
                        {
                            if let Some(l) = st2.borrow_mut().graph.links.get_mut(&id) {
                                merge(&mut l.props, p);
                            }
                            st2.borrow_mut().awaiting_info.remove(&id);
                            if let Some(core) = core2.upgrade() {
                                request_sync(&st2, &core);
                            }
                        }
                    })
                    .register();
                st.borrow_mut().awaiting_info.insert(id);
                st.borrow_mut().bound.insert(
                    id,
                    Bound {
                        node: None,
                        device: None,
                        _proxy: Some(Box::new(link)),
                        _listeners: vec![Box::new(l)],
                    },
                );
            }
        }
        ObjectType::Device => {
            let usb_channels = usb_channels(&props);
            st.borrow_mut().graph.devices.insert(
                id,
                junc_core::graph::Device {
                    id,
                    props,
                    profiles: Vec::new(),
                    active_profile: None,
                    usb_channels,
                },
            );
            // Profiles (F15): the list and the active one, kept current.
            if let Ok(device) = reg.bind::<pw::device::Device, _>(g) {
                let device = Rc::new(device);
                let (st2, core2) = (st.clone(), core.downgrade());
                let weak = Rc::downgrade(&device);
                let l = device
                    .add_listener_local()
                    // A subscription does not report a profile switch;
                    // read both params again whenever they change, as
                    // pw-dump does.
                    .info(move |info| {
                        if !info
                            .change_mask()
                            .contains(pw::device::DeviceChangeMask::PARAMS)
                        {
                            return;
                        }
                        let Some(d) = weak.upgrade() else { return };
                        for p in info.params() {
                            let id = p.id();
                            if matches!(
                                id,
                                pw::spa::param::ParamType::EnumProfile
                                    | pw::spa::param::ParamType::Profile
                            ) && p.flags().contains(pw::spa::param::ParamInfoFlags::READ)
                            {
                                d.enum_params(0, Some(id), 0, u32::MAX);
                            }
                        }
                    })
                    .param(move |_seq, kind, index, _next, pod| {
                        let profile = pod.and_then(parse_profile);
                        {
                            let mut s = st2.borrow_mut();
                            let Some(d) = s.graph.devices.get_mut(&id) else {
                                return;
                            };
                            match kind {
                                pw::spa::param::ParamType::EnumProfile => {
                                    if index == 0 {
                                        d.profiles.clear();
                                    }
                                    d.profiles.extend(profile);
                                }
                                pw::spa::param::ParamType::Profile => {
                                    d.active_profile = profile.map(|p| p.name);
                                }
                                _ => return,
                            }
                        }
                        if let Some(core) = core2.upgrade() {
                            request_sync(&st2, &core);
                        }
                    })
                    .register();
                st.borrow_mut().bound.insert(
                    id,
                    Bound {
                        _proxy: None,
                        node: None,
                        device: Some(device),
                        _listeners: vec![Box::new(l)],
                    },
                );
            }
        }
        ObjectType::Metadata if props.get("metadata.name") == Some("default") => {
            if let Ok(md) = reg.bind::<pw::metadata::Metadata, _>(g) {
                let (st2, core2) = (st.clone(), core.downgrade());
                let l = md
                    .add_listener_local()
                    .property(move |subject, key, _type, value| {
                        if subject == 0 {
                            set_default(&mut st2.borrow_mut().graph, key, value);
                        } else {
                            set_target(&mut st2.borrow_mut().graph, subject, key, value);
                        }
                        if let Some(core) = core2.upgrade() {
                            request_sync(&st2, &core);
                        }
                        0
                    })
                    .register();
                let mut s = st.borrow_mut();
                s.default_md = Some((id, md));
                // Defaults read before the properties arrive would look unset.
                s.awaiting_info.insert(id);
                s.md_sync = core.sync(0).ok();
                s.bound.insert(
                    id,
                    Bound {
                        node: None,
                        device: None,
                        _proxy: None,
                        _listeners: vec![Box::new(l)],
                    },
                );
            }
        }
        _ => return,
    }
    request_sync(st, core);
}

/// A stream's `target.object` (spike S11). A clear of the subject (key
/// `None`) removes it too.
fn set_target(g: &mut Graph, subject: u32, key: Option<&str>, value: Option<&str>) {
    match (key, value) {
        (Some("target.object"), Some(v)) => {
            g.targets.insert(subject, v.to_owned());
        }
        (Some("target.object") | None, _) => {
            g.targets.remove(&subject);
        }
        _ => {}
    }
}

fn set_default(g: &mut Graph, key: Option<&str>, value: Option<&str>) {
    let name = value
        .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
        .and_then(|v| {
            v.get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
    let d = &mut g.defaults;
    match key {
        Some("default.audio.sink") => d.sink = name,
        Some("default.audio.source") => d.source = name,
        Some("default.configured.audio.sink") => d.configured_sink = name,
        Some("default.configured.audio.source") => d.configured_source = name,
        // A metadata clear (key None) resets everything.
        None => *d = junc_core::graph::Defaults::default(),
        _ => {}
    }
}

/// Keeps a created object's proxy and follows it until its global shows up.
fn track(st: &Shared, core: &pw::core::CoreRc, proxy: Box<dyn ProxyT>) {
    let global = Rc::new(std::cell::Cell::new(None));
    let done = Rc::new(std::cell::Cell::new(false));
    let wake = |st: &Shared, core: &pw::core::CoreWeak| {
        if let Some(core) = core.upgrade() {
            request_sync(st, &core);
        }
    };
    let listener = {
        let (g, d1, d2) = (global.clone(), done.clone(), done.clone());
        let (st1, st2, st3) = (st.clone(), st.clone(), st.clone());
        let (c1, c2, c3) = (core.downgrade(), core.downgrade(), core.downgrade());
        proxy
            .upcast_ref()
            .add_listener_local()
            .bound(move |id| {
                g.set(Some(id));
                wake(&st1, &c1);
            })
            .removed(move || {
                d1.set(true);
                wake(&st2, &c2);
            })
            .error(move |_seq, _res, _msg| {
                d2.set(true);
                wake(&st3, &c3);
            })
            .register()
    };
    st.borrow_mut().created.push(Created {
        _proxy: proxy,
        _listener: listener,
        global,
        done,
    });
}

fn apply(
    st: &Shared,
    core: &pw::core::CoreRc,
    reg: &pw::registry::RegistryRc,
    a: &Action,
) -> Result<(), String> {
    match a {
        Action::CreateNode(spec) => {
            let props: pw::properties::PropertiesBox = spec.props().iter().collect();
            let node: pw::node::Node = core
                .create_object("adapter", &props)
                .map_err(|e| e.to_string())?;
            track(st, core, Box::new(node));
        }
        Action::CreateLink(spec) => {
            let props: pw::properties::PropertiesBox = spec.props().iter().collect();
            let link: pw::link::Link = core
                .create_object("link-factory", &props)
                .map_err(|e| e.to_string())?;
            track(st, core, Box::new(link));
        }
        Action::SetDefault { kind, name } => {
            let s = st.borrow();
            let Some((_, md)) = &s.default_md else {
                return Err("no default metadata (is WirePlumber running?)".into());
            };
            let value = name
                .as_ref()
                .map(|n| serde_json::json!({ "name": n }).to_string());
            md.set_property(
                0,
                kind.metadata_key(),
                value.as_ref().map(|_| "Spa:String:JSON"),
                value.as_deref(),
            );
        }
        Action::SetVolume {
            node,
            channels,
            percent,
        } => {
            let s = st.borrow();
            let Some(proxy) = s.bound.get(node).and_then(|b| b.node.as_ref()) else {
                return Err(format!("node {node} is not bound"));
            };
            if !s.graph.nodes.get(node).is_some_and(Node::is_junc) {
                return Err(format!("refusing to set the volume of foreign node {node}"));
            }
            let gain = junc_core::model::gain(*percent);
            let pod = volume_pod(*channels, gain)?;
            proxy.set_param(pw::spa::param::ParamType::Props, 0, pod_ref(&pod)?);
        }
        Action::SetDeviceProfile { device, index, .. } => {
            let s = st.borrow();
            let Some(proxy) = s.bound.get(device).and_then(|b| b.device.as_ref()) else {
                return Err(format!("device {device} is not bound"));
            };
            let pod = profile_pod(*index)?;
            proxy.set_param(pw::spa::param::ParamType::Profile, 0, pod_ref(&pod)?);
        }
        Action::SetTarget { node, target, .. } => {
            let s = st.borrow();
            let Some((_, md)) = &s.default_md else {
                return Err("no default metadata (is WirePlumber running?)".into());
            };
            // A node name, as pavucontrol and `pw-metadata` write it.
            md.set_property(*node, "target.object", None, target.as_deref());
        }
        Action::DestroyNode { node: id } | Action::DestroyLink { link: id } => {
            // Defence in depth for F17: the planner never asks for this, but
            // the backend still refuses to destroy anything not tagged ours.
            let ours = {
                let s = st.borrow();
                s.graph
                    .nodes
                    .get(id)
                    .map(Node::is_junc)
                    .or_else(|| s.graph.links.get(id).map(Link::is_junc))
            };
            match ours {
                Some(true) => {
                    reg.destroy_global(*id);
                }
                Some(false) => return Err(format!("refusing to destroy foreign object {id}")),
                None => return Err(format!("object {id} is gone")),
            }
        }
    }
    Ok(())
}

/// `Props` with `channelVolumes` = `gain` on every channel, serialized.
fn volume_pod(channels: u8, gain: f32) -> Result<Vec<u8>, String> {
    use pw::spa::pod::serialize::PodSerializer;
    use pw::spa::pod::{Object, Property, Value, ValueArray};
    let obj = Value::Object(Object {
        type_: pw::spa::sys::SPA_TYPE_OBJECT_Props,
        id: pw::spa::sys::SPA_PARAM_Props,
        properties: vec![Property::new(
            pw::spa::sys::SPA_PROP_channelVolumes,
            Value::ValueArray(ValueArray::Float(vec![gain; usize::from(channels.max(1))])),
        )],
    });
    PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &obj)
        .map(|(c, _)| c.into_inner())
        .map_err(|e| format!("cannot build the volume pod: {e:?}"))
}

fn pod_ref(bytes: &[u8]) -> Result<&pw::spa::pod::Pod, String> {
    pw::spa::pod::Pod::from_bytes(bytes).ok_or_else(|| "invalid volume pod".to_owned())
}

/// A `Profile` param: index and name from `EnumProfile` or `Profile`.
fn parse_profile(pod: &pw::spa::pod::Pod) -> Option<junc_core::graph::Profile> {
    use pw::spa::pod::Value;
    use pw::spa::pod::deserialize::PodDeserializer;
    use pw::spa::sys as spa;
    let (_, Value::Object(obj)) = PodDeserializer::deserialize_any_from(pod.as_bytes()).ok()?
    else {
        return None;
    };
    let mut p = junc_core::graph::Profile {
        index: u32::MAX,
        name: String::new(),
        description: String::new(),
        available: true,
    };
    for prop in obj.properties {
        match (prop.key, prop.value) {
            (spa::SPA_PARAM_PROFILE_index, Value::Int(i)) => p.index = u32::try_from(i).ok()?,
            (spa::SPA_PARAM_PROFILE_name, Value::String(s)) => p.name = s,
            (spa::SPA_PARAM_PROFILE_description, Value::String(s)) => p.description = s,
            (spa::SPA_PARAM_PROFILE_available, Value::Id(a)) => {
                p.available = a.0 != spa::SPA_PARAM_AVAILABILITY_no;
            }
            _ => {}
        }
    }
    (p.index != u32::MAX && !p.name.is_empty()).then_some(p)
}

/// The pod `wpctl set-profile` sends: index, and save so WirePlumber
/// remembers the choice for the card.
fn profile_pod(index: u32) -> Result<Vec<u8>, String> {
    use pw::spa::pod::serialize::PodSerializer;
    use pw::spa::pod::{Object, Property, Value};
    let index = i32::try_from(index).map_err(|_| format!("profile index {index} too large"))?;
    let obj = Value::Object(Object {
        type_: pw::spa::sys::SPA_TYPE_OBJECT_ParamProfile,
        id: pw::spa::sys::SPA_PARAM_Profile,
        properties: vec![
            Property::new(pw::spa::sys::SPA_PARAM_PROFILE_index, Value::Int(index)),
            Property::new(pw::spa::sys::SPA_PARAM_PROFILE_save, Value::Bool(true)),
        ],
    });
    PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &obj)
        .map(|(c, _)| c.into_inner())
        .map_err(|e| format!("cannot build the profile pod: {e:?}"))
}

/// Channel counts ALSA reports for a USB card (F15), read without
/// switching profiles: `/proc/asound/card<N>/stream*`.
fn usb_channels(props: &Props) -> Option<junc_core::proaudio::UsbChannels> {
    if props.get("device.api") != Some("alsa") || props.get("device.bus") != Some("usb") {
        return None;
    }
    let card = props.number("api.alsa.card")?;
    let dir = std::fs::read_dir(format!("/proc/asound/card{card}")).ok()?;
    dir.filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("stream"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|t| junc_core::proaudio::parse_usb_stream(&t))
        .reduce(junc_core::proaudio::UsbChannels::merge)
}
