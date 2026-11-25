//! pwobs — independent PipeWire graph observer used by the benchmark (same for both graphs).
//! Prints wall-clock-ms events:  OBS <epoch_ms> <event> ...
//!   app_node / app_linked relink_ms=..  : time from app node global to its first link global
//!   complete in_bus=.. out_bus=.. to_outputs=..  : first time the full JUNC graph is present
//! Reconnects forever (so it can time recovery after a PipeWire restart).
use pipewire as pw;
use pw::types::ObjectType;
use std::{cell::RefCell, collections::HashMap, rc::Rc, time::{SystemTime, UNIX_EPOCH}};

fn now() -> f64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64() * 1000.0 }

#[derive(Default)]
struct S { names: HashMap<u32, String>, apps: HashMap<u32, f64>, linked: std::collections::HashSet<u32>,
           links: HashMap<u32, (u32, u32)>, complete: bool, core_complete: bool }

fn main() {
    let prefix = std::env::var("JUNCD_APP_PREFIX").unwrap_or_else(|_| "juncbench-app".into());
    pw::init();
    loop {
        let _ = run(&prefix);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn run(prefix: &str) -> Result<(), pw::Error> {
    let ml = pw::main_loop::MainLoopRc::new(None)?;
    let ctx = pw::context::ContextRc::new(&ml, None)?;
    let core = ctx.connect_rc(None)?;
    let reg = core.get_registry_rc()?;
    println!("OBS {:.1} connected", now());
    let s = Rc::new(RefCell::new(S::default()));
    let ml2 = ml.clone();
    let _c = core.add_listener_local().error(move |id, _, _, _| if id == pw::core::PW_ID_CORE { ml2.quit() }).register();
    let (s1, s2, pfx) = (s.clone(), s.clone(), prefix.to_owned());
    let _r = reg.add_listener_local()
        .global(move |g| {
            let Some(p) = g.props else { return };
            let mut s = s1.borrow_mut();
            match g.type_ {
                ObjectType::Node => {
                    let name = p.get("node.name").unwrap_or("").to_owned();
                    if p.get("application.name").unwrap_or("").starts_with(&pfx) {
                        s.apps.insert(g.id, now());
                        println!("OBS {:.1} app_node id={} name={name}", now(), g.id);
                    }
                    s.names.insert(g.id, name);
                }
                ObjectType::Link => {
                    let o: u32 = p.get("link.output.node").and_then(|v| v.parse().ok()).unwrap_or(0);
                    let i: u32 = p.get("link.input.node").and_then(|v| v.parse().ok()).unwrap_or(0);
                    s.links.insert(g.id, (o, i));
                    if let Some(t) = s.apps.get(&o).copied() {
                        if s.linked.insert(o) { println!("OBS {:.1} app_linked node={o} relink_ms={:.2}", now(), now() - t); }
                    }
                    let name = |id: u32| s.names.get(&id).map(String::as_str).unwrap_or("");
                    let in_bus = s.links.values().filter(|(_, i)| name(*i) == "junc.bus").count();
                    let out_bus = s.links.values().filter(|(o, _)| name(*o) == "junc.bus").count();
                    let to_out = s.links.values().filter(|(_, i)| matches!(name(*i), "fake.hp" | "fake.obs")).count();
                    let hw_in = s.links.values().filter(|(o, i)| name(*o) == "fake.hwin" && name(*i) == "junc.bus").count();
                    if !s.core_complete && hw_in >= 2 && out_bus >= 4 && to_out >= 4 {
                        s.core_complete = true; // bus + hw input + both listen outputs (apps not required)
                        println!("OBS {:.1} core_complete hw_in={hw_in} out_bus={out_bus} to_outputs={to_out}", now());
                    }
                    if !s.complete && in_bus >= 8 && out_bus >= 4 && to_out >= 4 {
                        s.complete = true;
                        println!("OBS {:.1} complete in_bus={in_bus} out_bus={out_bus} to_outputs={to_out}", now());
                    }
                }
                _ => {}
            }
        })
        .global_remove(move |id| { let mut s = s2.borrow_mut(); s.links.remove(&id); s.apps.remove(&id); s.linked.remove(&id); })
        .register();
    ml.run();
    println!("OBS {:.1} disconnected", now());
    Ok(())
}
