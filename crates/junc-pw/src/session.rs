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
    _proxy: Box<dyn ProxyT>,
    _listeners: Vec<Box<dyn Listener>>,
}

struct State {
    graph: Graph,
    bound: HashMap<u32, Bound>,
    /// Proxies of objects we created, kept until the next sync completes.
    created: Vec<Box<dyn ProxyT>>,
    sync: Option<pw::spa::utils::result::AsyncSeq>,
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
        outcome: Outcome::Running,
    }));

    let _core_listener = {
        let (st, ml, ev) = (state.clone(), mainloop.downgrade(), on_event.clone());
        let (st2, ev2) = (state.clone(), on_event.clone());
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
                    if s.sync != Some(seq) {
                        return;
                    }
                    s.sync = None;
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
                if id == pw::core::PW_ID_CORE {
                    st.borrow_mut().outcome = Outcome::Lost;
                    if let Some(ml) = ml.upgrade() {
                        ml.quit();
                    }
                } else {
                    // An object we asked for could not be made (bad port, …).
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
                g.ports.remove(&id);
                g.links.remove(&id);
                g.devices.remove(&id);
                s.bound.remove(&id);
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
                            if let Some(core) = core2.upgrade() {
                                request_sync(&st2, &core);
                            }
                        }
                    })
                    .register();
                st.borrow_mut().bound.insert(
                    id,
                    Bound {
                        _proxy: Box::new(node),
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
                            if let Some(core) = core2.upgrade() {
                                request_sync(&st2, &core);
                            }
                        }
                    })
                    .register();
                st.borrow_mut().bound.insert(
                    id,
                    Bound {
                        _proxy: Box::new(link),
                        _listeners: vec![Box::new(l)],
                    },
                );
            }
        }
        ObjectType::Device => {
            st.borrow_mut().graph.devices.insert(
                id,
                junc_core::graph::Device {
                    id,
                    props,
                    profiles: Vec::new(),
                    active_profile: None,
                },
            );
        }
        ObjectType::Metadata if props.get("metadata.name") == Some("default") => {
            if let Ok(md) = reg.bind::<pw::metadata::Metadata, _>(g) {
                let (st2, core2) = (st.clone(), core.downgrade());
                let l = md
                    .add_listener_local()
                    .property(move |subject, key, _type, value| {
                        if subject == 0 {
                            set_default(&mut st2.borrow_mut().graph, key, value);
                            if let Some(core) = core2.upgrade() {
                                request_sync(&st2, &core);
                            }
                        }
                        0
                    })
                    .register();
                st.borrow_mut().bound.insert(
                    id,
                    Bound {
                        _proxy: Box::new(md),
                        _listeners: vec![Box::new(l)],
                    },
                );
            }
        }
        _ => return,
    }
    request_sync(st, core);
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
            st.borrow_mut().created.push(Box::new(node));
        }
        Action::CreateLink(spec) => {
            let props: pw::properties::PropertiesBox = spec.props().iter().collect();
            let link: pw::link::Link = core
                .create_object("link-factory", &props)
                .map_err(|e| e.to_string())?;
            st.borrow_mut().created.push(Box::new(link));
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
