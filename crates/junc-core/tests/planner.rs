// SPDX-License-Identifier: GPL-3.0-or-later

//! Planner against the mock backend on recorded graphs (hardware layer 1).

use junc_core::backend::{Action, Backend, MockBackend};
use junc_core::config;
use junc_core::graph::{Direction, Graph, Props};
use junc_core::model::Config;
use junc_core::plan::{Entity, Held, Status, default_map, plan, sink_node_name};
use proptest::prelude::*;
use std::path::Path;

fn graph(name: &str) -> Graph {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/graphs")
        .join(name);
    Graph::from_pw_dump(&std::fs::read_to_string(p).expect("fixture")).expect("pw-dump")
}

fn cfg(toml: &str) -> Config {
    match config::load(toml, "test.toml") {
        Ok(l) => l.config,
        Err(d) => panic!(
            "{}",
            d.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }
}

/// Plans and applies until nothing is left to do. Returns the rounds taken.
fn converge(b: &mut MockBackend, c: &Config, held: &Held) -> usize {
    for round in 1..=5 {
        let p = plan(c, b.graph(), held);
        if p.is_empty() {
            return round - 1;
        }
        for a in &p.actions {
            b.apply(a)
                .unwrap_or_else(|e| panic!("round {round}: {a}: {e}"));
        }
    }
    panic!("did not converge in 5 rounds");
}

fn node_id(b: &MockBackend, name: &str) -> u32 {
    b.graph()
        .node_by_name(name)
        .unwrap_or_else(|| panic!("no node {name}"))
        .id
}

/// (output node name, out channel) -> (input node name, in channel) for JUNC links.
fn junc_links(b: &MockBackend) -> Vec<(String, u32, String, u32)> {
    let g = b.graph();
    let mut v: Vec<_> = g
        .links
        .values()
        .filter(|l| l.is_junc())
        .map(|l| {
            let o = &g.ports[&l.output_port];
            let i = &g.ports[&l.input_port];
            (
                g.nodes[&l.output_node].name().to_owned(),
                o.index + 1,
                g.nodes[&l.input_node].name().to_owned(),
                i.index + 1,
            )
        })
        .collect();
    v.sort();
    v
}

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

#[test]
fn builds_a_junction_and_is_idempotent() {
    let mut b = MockBackend::new(graph("apps-browser-two-streams.json"));
    let c = cfg(ZOOM);
    let rounds = converge(&mut b, &c, &Held::new());
    assert_eq!(rounds, 2, "nodes first, links once their ports exist");
    for n in [
        "junc.zoom",
        "junc.zoom.sink",
        "junc.zoom.in.firefox",
        "junc.zoom.in.mic",
        "junc.zoom.listen.hp",
    ] {
        node_id(&b, n);
    }
    let links = junc_links(&b);
    let from = |src: &str, dst: &str| {
        links
            .iter()
            .filter(|l| l.0 == src && l.2 == dst)
            .map(|l| (l.1, l.3))
            .collect::<Vec<_>>()
    };
    // Both Firefox streams (same node.name) feed the gain node 1:1.
    assert_eq!(
        from("Firefox", "junc.zoom.in.firefox"),
        [(1, 1), (1, 1), (2, 2), (2, 2)]
    );
    // Mono mic feeds both Bus channels.
    assert_eq!(from("fake.hwin", "junc.zoom.in.mic"), [(1, 1), (1, 2)]);
    assert_eq!(from("junc.zoom.in.firefox", "junc.zoom"), [(1, 1), (2, 2)]);
    assert_eq!(from("junc.zoom.sink", "junc.zoom"), [(1, 1), (2, 2)]);
    assert_eq!(from("junc.zoom", "junc.zoom.listen.hp"), [(1, 1), (2, 2)]);
    assert_eq!(
        from("junc.zoom.listen.hp", "fake.hp"),
        [(1, 1), (2, 2)],
        "default sink is fake.hp"
    );
    // Spotify and Discord are untouched.
    assert!(links.iter().all(|l| l.0 != "spotify" && l.2 != "Discord"));
    assert!(plan(&c, b.graph(), &Held::new()).is_empty());
}

#[test]
fn app_restart_relinks_and_missing_apps_wait() {
    let mut b = MockBackend::new(graph("apps-browser-two-streams.json"));
    let c = cfg(ZOOM);
    converge(&mut b, &c, &Held::new());
    let firefox: Vec<u32> = b
        .graph()
        .nodes
        .values()
        .filter(|n| n.name() == "Firefox")
        .map(|n| n.id)
        .collect();
    for id in &firefox {
        b.remove_node(*id);
    }
    let p = plan(&c, b.graph(), &Held::new());
    assert!(
        p.is_empty(),
        "links went away with the app: {:?}",
        p.actions
    );
    let st = p
        .status
        .iter()
        .find(|s| s.entity == Entity::Input("firefox".into()))
        .expect("status");
    assert_eq!(st.status, Status::Waiting);
    // Firefox comes back with a new node id.
    let props: Props = [
        ("media.class", "Stream/Output/Audio"),
        ("node.name", "Firefox"),
        ("application.name", "Firefox"),
        ("application.process.binary", "firefox"),
    ]
    .into_iter()
    .collect();
    b.add_node(props, 2);
    assert_eq!(converge(&mut b, &c, &Held::new()), 1, "one round re-links");
    assert_eq!(
        junc_links(&b).iter().filter(|l| l.0 == "Firefox").count(),
        2
    );
}

#[test]
fn disabling_keeps_nodes_and_drops_routing() {
    let mut b = MockBackend::new(graph("apps-browser-two-streams.json"));
    converge(&mut b, &cfg(ZOOM), &Held::new());
    let off = cfg(&ZOOM.replace(
        "name = \"Zoom Mic\"",
        "name = \"Zoom Mic\"\nenabled = false",
    ));
    converge(&mut b, &off, &Held::new());
    node_id(&b, "junc.zoom");
    node_id(&b, "junc.zoom.sink");
    let links = junc_links(&b);
    assert!(
        links.iter().all(|l| l.0 == "junc.zoom.sink"),
        "only the sink feeds the Bus: {links:?}"
    );
    let st = plan(&off, b.graph(), &Held::new()).status;
    assert!(
        st.iter()
            .filter(|s| s.entity != Entity::Junction)
            .all(|s| s.status == Status::Disabled)
    );
}

#[test]
fn removing_from_config_cleans_up_only_junc_objects() {
    let mut b = MockBackend::new(graph("apps-browser-two-streams.json"));
    let before_nodes = b.graph().nodes.len();
    let before_links = b.graph().links.clone();
    converge(&mut b, &cfg(ZOOM), &Held::new());
    converge(&mut b, &cfg("schema = 1\n"), &Held::new());
    assert_eq!(b.graph().nodes.len(), before_nodes);
    assert_eq!(
        b.graph().links,
        before_links,
        "foreign links survive untouched"
    );
}

#[test]
fn rename_recreates_when_idle_and_waits_while_in_use() {
    let mut b = MockBackend::new(graph("apps-browser-two-streams.json"));
    converge(&mut b, &cfg(ZOOM), &Held::new());
    // Discord starts recording from the Junction.
    let src = node_id(&b, "junc.zoom");
    let discord = node_id(&b, "Discord");
    let out = b
        .graph()
        .ports_of(src, Direction::Out)
        .next()
        .expect("port")
        .id;
    let inp = b
        .graph()
        .ports_of(discord, Direction::In)
        .next()
        .expect("port")
        .id;
    let rec = b.add_foreign_link(out, inp).expect("link");
    let renamed = cfg(&ZOOM.replace("Zoom Mic", "Meeting Mic"));
    let p = plan(&renamed, b.graph(), &Held::new());
    // The source Discord records from waits; the idle sink is recreated now;
    // gain nodes are not renamed at all.
    assert!(
        !p.actions.contains(&Action::DestroyNode { node: src }),
        "{:?}",
        p.actions
    );
    assert_eq!(p.deferred, ["junc.zoom"]);
    let sink = node_id(&b, "junc.zoom.sink");
    assert!(p.actions.contains(&Action::DestroyNode { node: sink }));
    assert_eq!(
        p.actions
            .iter()
            .filter(|a| matches!(a, Action::DestroyNode { .. }))
            .count(),
        1
    );
    // Discord stops; the rename goes through and routing is rebuilt.
    b.remove_link(rec);
    converge(&mut b, &renamed, &Held::new());
    let desc = b
        .graph()
        .node_by_name("junc.zoom")
        .map(|n| n.description().to_owned());
    assert_eq!(desc.as_deref(), Some("Meeting Mic"));
    assert!(plan(&renamed, b.graph(), &Held::new()).deferred.is_empty());
    assert_eq!(
        junc_links(&b).iter().filter(|l| l.0 == "Firefox").count(),
        4
    );
}

#[test]
fn bus_resize_recreates_the_nodes() {
    let mut b = MockBackend::new(graph("apps-browser-two-streams.json"));
    converge(&mut b, &cfg(ZOOM), &Held::new());
    let wide = cfg(&ZOOM.replace(
        "name = \"Zoom Mic\"",
        "name = \"Zoom Mic\"\nbus = { channels = 4 }",
    ));
    converge(&mut b, &wide, &Held::new());
    let src = node_id(&b, "junc.zoom");
    assert_eq!(b.graph().ports_of(src, Direction::In).count(), 4);
}

#[test]
fn held_links_are_not_recreated() {
    let mut b = MockBackend::new(graph("apps-browser-two-streams.json"));
    let c = cfg(ZOOM);
    converge(&mut b, &c, &Held::new());
    let victim = b
        .graph()
        .links
        .values()
        .find(|l| l.props.get("junc.ref") == Some("in/mic/1->2"))
        .expect("link")
        .id;
    b.remove_link(victim); // another tool removed it
    let held: Held = [("zoom".to_owned(), "in/mic/1->2".to_owned())]
        .into_iter()
        .collect();
    assert!(
        plan(&c, b.graph(), &held).is_empty(),
        "held: wait for Restore / Leave as is"
    );
    assert_eq!(
        converge(&mut b, &c, &Held::new()),
        1,
        "Restore recreates it"
    );
}

#[test]
fn foreign_link_on_the_same_ports_is_respected() {
    let mut b = MockBackend::new(graph("apps-browser-two-streams.json"));
    let c = cfg(ZOOM);
    converge(&mut b, &c, &Held::new());
    let l = b
        .graph()
        .links
        .values()
        .find(|l| l.props.get("junc.ref") == Some("in/mic/1->1"))
        .cloned()
        .expect("link");
    b.remove_link(l.id);
    b.add_foreign_link(l.output_port, l.input_port);
    assert!(
        plan(&c, b.graph(), &Held::new()).is_empty(),
        "no duplicate link next to a foreign one"
    );
}

#[test]
fn cable_delivers_to_the_recording_app() {
    let mut b = MockBackend::new(graph("apps-browser-two-streams.json"));
    let c = cfg(r#"
schema = 1
[[junction]]
id = "ff-discord"
name = "Firefox → Discord"
kind = "cable"
[[junction.input]]
id = "firefox"
match = { binary = "firefox" }
[junction.cable]
to = { binary = "Discord" }
"#);
    converge(&mut b, &c, &Held::new());
    let links = junc_links(&b);
    let discord_in = links.iter().filter(|l| l.2 == "Discord").count();
    assert!(discord_in >= 1, "{links:?}");
    assert!(
        links
            .iter()
            .any(|l| l.0 == "junc.ff_discord" && l.2 == "Discord")
    );
    assert!(
        b.graph()
            .node_by_name(&sink_node_name("ff-discord"))
            .is_some()
    );
}

#[test]
fn default_mapping() {
    assert_eq!(default_map(1, 2), [[1, 1], [1, 2]]);
    assert_eq!(default_map(1, 1), [[1, 1]]);
    assert_eq!(default_map(2, 4), [[1, 1], [2, 2]]);
    assert_eq!(default_map(6, 2), [[1, 1], [2, 2]]);
    assert!(default_map(0, 2).is_empty());
}

// --- invariant: never touch what is not ours, always converge ------------

const FIXTURES: [&str; 6] = [
    "apps-browser-two-streams.json",
    "asahi-m1-j293.json",
    "snd-aloop-proaudio.json",
    "komplete-audio-2-stereo.json",
    "bt-airpods-a2dp.json",
    "bt-pixel-buds-2a-hfp.json",
];

fn arb_config(names: Vec<String>) -> impl Strategy<Value = String> {
    let target = proptest::sample::select(names);
    let input = (
        target.clone(),
        any::<bool>(),
        proptest::option::of((1u8..4, 1u8..8)),
    )
        .prop_map(|(t, enabled, link)| {
            let links = link
                .map(|(a, b)| format!("\nlinks = [[{a}, {b}]]"))
                .unwrap_or_default();
            format!("match = {{ device = \"{t}\" }}\nenabled = {enabled}{links}")
        });
    let junction = (
        1u8..9,
        any::<bool>(),
        proptest::collection::vec(input, 0..4),
        any::<bool>(),
    );
    proptest::collection::vec(junction, 1..4).prop_map(|js| {
        let mut s = String::from("schema = 1\n");
        for (n, (ch, enabled, inputs, listen)) in js.into_iter().enumerate() {
            s += &format!("[[junction]]\nid = \"j{n}\"\nname = \"J {n}\"\nenabled = {enabled}\nbus = {{ channels = {ch} }}\n");
            for (k, body) in inputs.into_iter().enumerate() {
                s += &format!("[[junction.input]]\nid = \"i{k}\"\n{body}\n");
            }
            if listen {
                s += "[[junction.listen]]\nid = \"out\"\ntarget = \"default\"\n";
            }
        }
        s
    })
}

fn fixture_and_config() -> impl Strategy<Value = (&'static str, String)> {
    proptest::sample::select(FIXTURES.to_vec()).prop_flat_map(|f| {
        let mut names: Vec<String> = graph(f)
            .nodes
            .values()
            .map(|n| n.name().to_owned())
            .collect();
        names.push("not-plugged-in".into());
        (Just(f), arb_config(names))
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn never_touches_foreign_objects_and_converges((fixture, text) in fixture_and_config()) {
        let g = graph(fixture);
        let foreign_nodes: Vec<u32> = g.nodes.keys().copied().collect();
        let foreign_links = g.links.clone();
        let Ok(loaded) = config::load(&text, "gen.toml") else { return Ok(()) };
        let mut b = MockBackend::new(g);
        // MockBackend refuses foreign changes, so converge() would panic on any.
        converge(&mut b, &loaded.config, &Held::new());
        prop_assert!(plan(&loaded.config, b.graph(), &Held::new()).is_empty());
        for id in &foreign_nodes {
            prop_assert!(b.graph().nodes.contains_key(id), "foreign node {} removed", id);
        }
        for (id, l) in &foreign_links {
            prop_assert_eq!(b.graph().links.get(id), Some(l));
        }
        // Removing everything from the config restores the original graph.
        converge(&mut b, &Config::default(), &Held::new());
        prop_assert_eq!(&b.graph().links, &foreign_links);
        prop_assert_eq!(b.graph().nodes.keys().copied().collect::<Vec<_>>(), foreign_nodes);
    }
}
