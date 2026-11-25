// SPDX-License-Identifier: GPL-3.0-or-later

//! Reconcile state machine (§3.5) on the mock backend.

use junc_core::backend::{Action, Backend, MockBackend};
use junc_core::config;
use junc_core::graph::Graph;
use junc_core::model::Config;
use junc_core::reconcile::{Notice, Reconciler, Resolution, Step};
use std::path::Path;

const ZOOM: &str = r#"
schema = 1
[[junction]]
id = "zoom"
name = "Zoom Mic"
[[junction.input]]
id = "firefox"
match = { binary = "firefox" }
[[junction.input]]
id = "mic"
match = { device = "fake.hwin" }
[[junction.listen]]
id = "hp"
target = "default"
"#;

fn setup() -> (MockBackend, Config, Reconciler) {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/graphs/apps-browser-two-streams.json");
    let g = Graph::from_pw_dump(&std::fs::read_to_string(p).expect("fixture")).expect("graph");
    let cfg = config::load(ZOOM, "t.toml").expect("config").config;
    let mut b = MockBackend::new(g);
    let mut r = Reconciler::new();
    for _ in 0..4 {
        step(&mut b, &cfg, &mut r);
    }
    assert!(r.on_graph(&cfg, b.graph()).actions.is_empty(), "converged");
    (b, cfg, r)
}

fn step(b: &mut MockBackend, cfg: &Config, r: &mut Reconciler) -> Step {
    let s = r.on_graph(cfg, b.graph());
    for a in &s.actions {
        b.apply(a).unwrap_or_else(|e| panic!("{a}: {e}"));
    }
    s
}

fn link_by_ref(b: &MockBackend, reference: &str) -> Option<u32> {
    b.graph()
        .links
        .values()
        .find(|l| l.props.get("junc.ref") == Some(reference))
        .map(|l| l.id)
}

fn overridden(junction: &str, reference: &str) -> Notice {
    Notice::LinkOverridden {
        junction: junction.into(),
        reference: reference.into(),
    }
}

#[test]
fn another_tool_removing_a_link_is_not_fought() {
    let (mut b, cfg, mut r) = setup();
    b.remove_link(link_by_ref(&b, "in/mic/1->2").expect("link"));
    let s1 = step(&mut b, &cfg, &mut r);
    assert!(
        s1.actions.is_empty() && s1.notices.is_empty(),
        "held at once, confirmed next round: {s1:?}"
    );
    let s2 = step(&mut b, &cfg, &mut r);
    assert_eq!(s2.notices, [overridden("zoom", "in/mic/1->2")]);
    assert!(s2.actions.is_empty());
    for _ in 0..3 {
        let s = step(&mut b, &cfg, &mut r);
        assert!(
            s.actions.is_empty() && s.notices.is_empty(),
            "one notice, no fighting"
        );
    }
    assert!(
        r.resolve(&cfg, "zoom", "in/mic/1->2", Resolution::Restore)
            .is_none()
    );
    step(&mut b, &cfg, &mut r);
    assert!(link_by_ref(&b, "in/mic/1->2").is_some(), "restored");
}

#[test]
fn leave_as_is_writes_suppressed_and_survives_restarts() {
    let (mut b, cfg, mut r) = setup();
    b.remove_link(link_by_ref(&b, "listen/hp/2->2").expect("link"));
    step(&mut b, &cfg, &mut r);
    step(&mut b, &cfg, &mut r);
    let new_cfg = r
        .resolve(&cfg, "zoom", "listen/hp/2->2", Resolution::LeaveAsIs)
        .expect("config change");
    assert_eq!(new_cfg.junction[0].listen[0].suppressed, [[2, 2]]);
    // A fresh reconciler (daemon restart) with the saved config leaves it out.
    let mut fresh = Reconciler::new();
    let s = fresh.on_graph(&new_cfg, b.graph());
    // Only the volumes a new daemon sets on first sight; no link comes back.
    assert!(
        s.actions
            .iter()
            .all(|a| matches!(a, Action::SetVolume { .. }))
            && s.notices.is_empty(),
        "{s:?}"
    );
}

#[test]
fn an_app_quitting_is_not_an_override() {
    let (mut b, cfg, mut r) = setup();
    // PipeWire removes an app's links first, then its node, sometimes in
    // separate batches.
    let firefox: Vec<u32> = b
        .graph()
        .nodes
        .values()
        .filter(|n| n.name() == "Firefox")
        .map(|n| n.id)
        .collect();
    let links: Vec<u32> = b
        .graph()
        .links
        .values()
        .filter(|l| firefox.contains(&l.output_node))
        .map(|l| l.id)
        .collect();
    for l in links {
        b.remove_link(l);
    }
    let s1 = step(&mut b, &cfg, &mut r);
    assert!(s1.notices.is_empty());
    for id in firefox {
        b.remove_node(id);
    }
    let s2 = step(&mut b, &cfg, &mut r);
    assert!(s2.notices.is_empty(), "nodes gone: not an override");
    assert!(r.held().is_empty());
}

#[test]
fn an_app_restarting_under_a_reused_id_is_relinked() {
    let (mut b, cfg, mut r) = setup();
    // The app quits and starts again between two snapshots, and PipeWire
    // hands the new stream the id the old one had: only `object.serial`
    // tells them apart. Its links vanished with the old stream; that is not
    // the user removing them (F10).
    let firefox: Vec<u32> = b
        .graph()
        .nodes
        .values()
        .filter(|n| n.name() == "Firefox")
        .map(|n| n.id)
        .collect();
    let junc_links = |b: &MockBackend| b.graph().links.values().filter(|l| l.is_junc()).count();
    let before = junc_links(&b);
    b.graph_mut()
        .links
        .retain(|_, l| !firefox.contains(&l.output_node));
    assert!(junc_links(&b) < before, "firefox was linked");
    for id in &firefox {
        let n = b.graph_mut().nodes.get_mut(id).expect("node");
        let serial = n.props.number("object.serial").expect("serial");
        n.props.set("object.serial", (serial + 10_000).to_string());
    }
    for _ in 0..3 {
        let s = step(&mut b, &cfg, &mut r);
        assert!(s.notices.is_empty(), "not an override: {s:?}");
    }
    assert!(r.held().is_empty());
    assert_eq!(junc_links(&b), before, "re-linked");
}

#[test]
fn own_destroys_and_internal_links() {
    let (mut b, _, mut r) = setup();
    // Remapping the mic to channel 1 only makes JUNC destroy in/mic/1->2
    // itself while both nodes stay: no notice, no hold.
    let off = config::load(
        &ZOOM.replace(
            "match = { device = \"fake.hwin\" }",
            "match = { device = \"fake.hwin\" }\nlinks = [[1, 1]]",
        ),
        "t.toml",
    )
    .expect("cfg")
    .config;
    for _ in 0..3 {
        let s = step(&mut b, &off, &mut r);
        assert!(s.notices.is_empty(), "{s:?}");
    }
    assert!(link_by_ref(&b, "in/mic/1->2").is_none());
    assert!(r.held().is_empty());
    // Internal wiring removed by someone else is restored at once.
    b.remove_link(link_by_ref(&b, "sink/1").expect("link"));
    step(&mut b, &off, &mut r);
    assert!(link_by_ref(&b, "sink/1").is_some());
}

#[test]
fn holds_end_with_their_junction() {
    let (mut b, cfg, mut r) = setup();
    b.remove_link(link_by_ref(&b, "in/mic/1->1").expect("link"));
    step(&mut b, &cfg, &mut r);
    step(&mut b, &cfg, &mut r);
    assert_eq!(r.overridden().len(), 1);
    let empty = Config::default();
    step(&mut b, &empty, &mut r);
    assert!(r.overridden().is_empty());
}

const CABLE: &str = r#"
schema = 1
[[junction]]
id = "ff-discord"
name = "Firefox to Discord"
kind = "cable"
[[junction.input]]
id = "firefox"
match = { binary = "firefox" }
[junction.cable]
to = { binary = "Discord" }
"#;

/// Discord's capture streams in the fixture.
fn discord(b: &MockBackend) -> Vec<u32> {
    let m = junc_core::model::Match {
        binary: Some("Discord".into()),
        ..Default::default()
    };
    junc_core::matcher::resolve(&m, junc_core::matcher::Use::CableTarget, b.graph())
}

#[test]
fn cable_targets_are_set_respected_and_returned() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/graphs/apps-browser-two-streams.json");
    let g = Graph::from_pw_dump(&std::fs::read_to_string(p).expect("fixture")).expect("graph");
    let cfg = config::load(CABLE, "t.toml").expect("config").config;
    let mut b = MockBackend::new(g);
    let mut r = Reconciler::new();
    for _ in 0..4 {
        step(&mut b, &cfg, &mut r);
    }
    let streams = discord(&b);
    let d = streams[0];
    assert_eq!(
        b.graph().targets.get(&d).map(String::as_str),
        Some("junc.ff_discord")
    );
    assert!(r.on_graph(&cfg, b.graph()).actions.is_empty(), "converged");

    // The user moves Discord to the real mic: held and reported, not fought.
    b.graph_mut().targets.insert(d, "fake.hwin".into());
    let s = step(&mut b, &cfg, &mut r);
    let reference = junc_core::plan::cable_target_ref(b.graph(), d);
    assert_eq!(s.notices, [overridden("ff-discord", &reference)]);
    assert!(s.actions.is_empty(), "{:?}", s.actions);
    assert!(step(&mut b, &cfg, &mut r).actions.is_empty());
    assert_eq!(
        b.graph().targets.get(&d).map(String::as_str),
        Some("fake.hwin")
    );

    // Leave as is keeps holding; Restore sends it back to the Cable.
    assert_eq!(
        r.resolve(&cfg, "ff-discord", &reference, Resolution::LeaveAsIs),
        None
    );
    assert!(r.is_held("ff-discord", &reference));
    assert!(step(&mut b, &cfg, &mut r).actions.is_empty());
    r.resolve(&cfg, "ff-discord", &reference, Resolution::Restore);
    step(&mut b, &cfg, &mut r);
    assert_eq!(
        b.graph().targets.get(&d).map(String::as_str),
        Some("junc.ff_discord")
    );

    // A stream the user sent to a Junction themselves is not JUNC's to undo.
    let other = *b
        .graph()
        .nodes
        .keys()
        .find(|n| !streams.contains(n))
        .expect("node");
    b.graph_mut()
        .targets
        .insert(other, "junc.ff_discord".into());

    // Deleting the Cable returns JUNC's streams to the default, only those.
    let empty = config::load("schema = 1\n", "t.toml")
        .expect("config")
        .config;
    for _ in 0..3 {
        step(&mut b, &empty, &mut r);
    }
    for s in &streams {
        assert_eq!(b.graph().targets.get(s), None, "stream {s} still targeted");
    }
    assert_eq!(
        b.graph().targets.get(&other).map(String::as_str),
        Some("junc.ff_discord")
    );
}

#[test]
fn a_cable_target_restarting_under_a_reused_id_is_sent_again() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/graphs/apps-browser-two-streams.json");
    let g = Graph::from_pw_dump(&std::fs::read_to_string(p).expect("fixture")).expect("graph");
    let cfg = config::load(CABLE, "t.toml").expect("config").config;
    let mut b = MockBackend::new(g);
    let mut r = Reconciler::new();
    for _ in 0..4 {
        step(&mut b, &cfg, &mut r);
    }
    let d = discord(&b)[0];
    // Discord restarts between two snapshots and gets its old id back; the
    // new stream has no target yet. Nobody moved it: no notice, and JUNC
    // sends the new stream to the Cable.
    b.graph_mut().targets.remove(&d);
    let n = b.graph_mut().nodes.get_mut(&d).expect("node");
    let serial = n.props.number("object.serial").expect("serial");
    n.props.set("object.serial", (serial + 10_000).to_string());
    for _ in 0..3 {
        let s = step(&mut b, &cfg, &mut r);
        assert!(s.notices.is_empty(), "not an override: {s:?}");
    }
    assert!(r.held().is_empty());
    assert_eq!(
        b.graph().targets.get(&d).map(String::as_str),
        Some("junc.ff_discord")
    );
}

/// F7: volumes go to the Junction source (master), the Input gain nodes and
/// the Listen gain nodes; each is set once, again only when it changes or
/// the node is recreated.
#[test]
fn volumes_are_set_where_they_belong_and_only_when_needed() {
    let (mut b, cfg, mut r) = setup();
    let node = |b: &MockBackend, name: &str| b.graph().node_by_name(name).expect(name).id;
    // First sight sets every JUNC node, at 100 % here.
    for name in [
        "junc.zoom",
        "junc.zoom.in.firefox",
        "junc.zoom.in.mic",
        "junc.zoom.listen.hp",
    ] {
        assert_eq!(b.volume(node(&b, name)), 1.0, "{name}");
    }
    assert!(
        r.on_graph(&cfg, b.graph()).actions.is_empty(),
        "nothing more to set"
    );

    let mut c = cfg.clone();
    c.junction[0].volume = 50;
    c.junction[0].input[0].volume = 70;
    c.junction[0].listen[0].volume = 150;
    let s = step(&mut b, &c, &mut r);
    let set: Vec<String> = s.actions.iter().map(ToString::to_string).collect();
    assert_eq!(set.len(), 3, "{set:?}");
    assert_eq!(b.volume(node(&b, "junc.zoom")), 0.125);
    assert!((b.volume(node(&b, "junc.zoom.in.firefox")) - 0.343).abs() < 1e-6);
    assert_eq!(b.volume(node(&b, "junc.zoom.in.mic")), 1.0, "untouched");
    assert!((b.volume(node(&b, "junc.zoom.listen.hp")) - 3.375).abs() < 1e-6);
    assert!(step(&mut b, &c, &mut r).actions.is_empty());

    // A recreated node (new serial) gets its volume again.
    let id = node(&b, "junc.zoom.in.firefox");
    b.remove_node(id);
    for _ in 0..3 {
        step(&mut b, &c, &mut r);
    }
    assert!((b.volume(node(&b, "junc.zoom.in.firefox")) - 0.343).abs() < 1e-6);
}

/// F11 (B2): an Exclusive Input sends the app's streams to its gain node,
/// holds a move someone else makes, and returns them when turned off.
#[test]
fn exclusive_moves_streams_to_the_gain_node_and_back() {
    let (mut b, cfg, mut r) = setup();
    let mut on = cfg.clone();
    on.junction[0].input[0].exclusive = true;
    for _ in 0..3 {
        step(&mut b, &on, &mut r);
    }
    let firefox: Vec<u32> = junc_core::matcher::resolve(
        &on.junction[0].input[0].matcher,
        junc_core::matcher::Use::Input,
        b.graph(),
    );
    assert!(!firefox.is_empty());
    for s in &firefox {
        assert_eq!(
            b.graph().targets.get(s).map(String::as_str),
            Some("junc.zoom.in.firefox"),
            "stream {s}"
        );
    }
    assert!(r.on_graph(&on, b.graph()).actions.is_empty(), "converged");

    // The user moves one stream to the speakers: held, reported, not fought.
    let s0 = firefox[0];
    b.graph_mut().targets.insert(s0, "fake.hp".into());
    let s = step(&mut b, &on, &mut r);
    let reference = junc_core::plan::stream_target_ref("exclusive", b.graph(), s0);
    assert_eq!(s.notices, [overridden("zoom", &reference)]);
    assert!(s.actions.is_empty(), "{:?}", s.actions);
    assert!(step(&mut b, &on, &mut r).actions.is_empty());

    // Exclusive off: JUNC's other streams go back to the default; the one
    // the user moved stays where they put it. The capture links stay.
    for _ in 0..3 {
        step(&mut b, &cfg, &mut r);
    }
    for s in &firefox[1..] {
        assert_eq!(b.graph().targets.get(s), None, "stream {s}");
    }
    assert_eq!(
        b.graph().targets.get(&s0).map(String::as_str),
        Some("fake.hp")
    );
    assert!(link_by_ref(&b, "in/firefox/1->1").is_some());
}

#[test]
fn exclusive_is_for_app_inputs_only() {
    let text = ZOOM.replace(
        "match = { device = \"fake.hwin\" }",
        "match = { device = \"fake.hwin\" }\nexclusive = true",
    );
    let err = config::load(&text, "t.toml").expect_err("device Input with exclusive");
    assert!(format!("{err:?}").contains("exclusive"), "{err:?}");
}

/// A daemon that restarts does not remember the streams it moved; turning
/// Exclusive off afterwards must still give them back.
#[test]
fn exclusive_off_after_a_restart_returns_the_streams() {
    let (mut b, cfg, mut r) = setup();
    let mut on = cfg.clone();
    on.junction[0].input[0].exclusive = true;
    for _ in 0..3 {
        step(&mut b, &on, &mut r);
    }
    let moved: Vec<u32> = b
        .graph()
        .targets
        .iter()
        .filter(|(_, v)| *v == "junc.zoom.in.firefox")
        .map(|(s, _)| *s)
        .collect();
    assert!(!moved.is_empty());
    let mut fresh = Reconciler::new();
    for _ in 0..3 {
        step(&mut b, &cfg, &mut fresh);
    }
    for s in &moved {
        assert_eq!(b.graph().targets.get(s), None, "stream {s}");
    }
}
