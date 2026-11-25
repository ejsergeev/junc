// SPDX-License-Identifier: GPL-3.0-or-later

//! Junctions as system defaults (F16) on the mock backend.

use junc_core::backend::{Action, Backend, DefaultKind, MockBackend};
use junc_core::config;
use junc_core::graph::Graph;
use junc_core::model::Config;
use junc_core::reconcile::{Notice, Reconciler, Step};
use std::path::Path;

fn cfg(default: &str, enabled: bool) -> Config {
    let t = format!(
        "schema = 1\n[[junction]]\nid = \"zoom\"\nname = \"Zoom Mic\"\nenabled = {enabled}\ndefault = \"{default}\"\n"
    );
    config::load(&t, "t.toml").expect("config").config
}

fn backend() -> MockBackend {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/graphs/apps-browser-two-streams.json");
    let mut g = Graph::from_pw_dump(&std::fs::read_to_string(p).expect("fixture")).expect("graph");
    // The user had picked the fake mic and headphones.
    g.defaults.configured_source = Some("fake.hwin".into());
    g.defaults.configured_sink = Some("fake.hp".into());
    MockBackend::new(g)
}

fn run(b: &mut MockBackend, c: &Config, r: &mut Reconciler, rounds: usize) -> Vec<Notice> {
    let mut notices = Vec::new();
    for _ in 0..rounds {
        let s: Step = r.on_graph(c, b.graph());
        for a in &s.actions {
            b.apply(a).unwrap_or_else(|e| panic!("{a}: {e}"));
        }
        notices.extend(s.notices);
    }
    notices
}

fn source(b: &MockBackend) -> Option<String> {
    b.graph().defaults.configured_source.clone()
}

#[test]
fn becomes_default_and_restores_on_disable() {
    let mut b = backend();
    let mut r = Reconciler::new();
    assert!(run(&mut b, &cfg("input", true), &mut r, 4).is_empty());
    assert_eq!(source(&b).as_deref(), Some("junc.zoom"));
    assert_eq!(r.defaults().source.previous, Some(Some("fake.hwin".into())));
    let n = run(&mut b, &cfg("input", false), &mut r, 3);
    assert_eq!(
        n,
        [Notice::DefaultRestored {
            kind: DefaultKind::Source,
            to: Some("fake.hwin".into())
        }]
    );
    assert_eq!(source(&b).as_deref(), Some("fake.hwin"));
    assert_eq!(r.defaults().source, Default::default());
}

#[test]
fn restore_target_missing_clears_the_key() {
    let mut b = backend();
    let mut r = Reconciler::new();
    run(&mut b, &cfg("input", true), &mut r, 4);
    let hwin = b.graph().node_by_name("fake.hwin").expect("node").id;
    b.remove_node(hwin);
    let n = run(&mut b, &cfg("none", true), &mut r, 3);
    assert_eq!(
        n,
        [Notice::DefaultRestored {
            kind: DefaultKind::Source,
            to: None
        }]
    );
    assert_eq!(source(&b), None, "WirePlumber chooses");
}

#[test]
fn deleting_the_junction_restores_too() {
    let mut b = backend();
    let mut r = Reconciler::new();
    run(&mut b, &cfg("input", true), &mut r, 4);
    let n = run(&mut b, &Config::default(), &mut r, 3);
    assert_eq!(
        n,
        [Notice::DefaultRestored {
            kind: DefaultKind::Source,
            to: Some("fake.hwin".into())
        }]
    );
}

#[test]
fn a_user_choice_elsewhere_is_respected() {
    let mut b = backend();
    let mut r = Reconciler::new();
    let c = cfg("input", true);
    run(&mut b, &c, &mut r, 4);
    // The user picks another mic in the desktop's sound settings.
    b.graph_mut().defaults.configured_source = Some("fake.hwin".into());
    let n = run(&mut b, &c, &mut r, 3);
    assert_eq!(
        n,
        [Notice::DefaultTakenOver {
            kind: DefaultKind::Source,
            junction: "zoom".into()
        }]
    );
    assert_eq!(source(&b).as_deref(), Some("fake.hwin"), "not taken back");
    // Turning the Junction off later does not touch the user's choice.
    assert!(run(&mut b, &cfg("input", false), &mut r, 3).is_empty());
    assert_eq!(source(&b).as_deref(), Some("fake.hwin"));
}

#[test]
fn output_default_uses_the_junction_sink() {
    let mut b = backend();
    let mut r = Reconciler::new();
    run(&mut b, &cfg("output", true), &mut r, 4);
    assert_eq!(
        b.graph().defaults.configured_sink.as_deref(),
        Some("junc.zoom.sink")
    );
    let n = run(&mut b, &cfg("none", true), &mut r, 3);
    assert_eq!(
        n,
        [Notice::DefaultRestored {
            kind: DefaultKind::Sink,
            to: Some("fake.hp".into())
        }]
    );
}

#[test]
fn moving_default_between_junctions_keeps_the_original() {
    let two = |zoom: &str, obs: &str| {
        let t = format!(
            "schema = 1\n[[junction]]\nid = \"zoom\"\nname = \"Zoom Mic\"\ndefault = \"{zoom}\"\n[[junction]]\nid = \"obs\"\nname = \"OBS Mix\"\ndefault = \"{obs}\"\n"
        );
        config::load(&t, "t.toml").expect("config").config
    };
    let mut b = backend();
    let mut r = Reconciler::new();
    run(&mut b, &two("input", "none"), &mut r, 4);
    assert_eq!(source(&b).as_deref(), Some("junc.zoom"));
    assert!(run(&mut b, &two("none", "input"), &mut r, 4).is_empty());
    assert_eq!(source(&b).as_deref(), Some("junc.obs"));
    assert_eq!(
        r.defaults().source.previous,
        Some(Some("fake.hwin".into())),
        "the user's mic, not junc.zoom"
    );
    // Neither is the default any more: back to the user's mic.
    let n = run(&mut b, &two("none", "none"), &mut r, 3);
    assert_eq!(
        n,
        [Notice::DefaultRestored {
            kind: DefaultKind::Source,
            to: Some("fake.hwin".into())
        }]
    );
}

#[test]
fn a_stale_snapshot_is_not_a_takeover() {
    let mut b = backend();
    let mut r = Reconciler::new();
    let c = cfg("input", true);
    run(&mut b, &c, &mut r, 3);
    // Planning SetDefault but seeing a snapshot before it lands.
    let mut set = false;
    for _ in 0..3 {
        let s = r.on_graph(&c, b.graph());
        assert!(s.notices.is_empty(), "{:?}", s.notices);
        set |= s
            .actions
            .iter()
            .any(|a| matches!(a, Action::SetDefault { .. }));
    }
    assert!(set || source(&b).as_deref() == Some("junc.zoom"));
}

/// Found on the owner's laptop (A19): the configured mic was the headset
/// jack, unplugged, so WirePlumber used the built-in mic. After the
/// Junction stopped being the default, JUNC put the headset back as
/// configured, but WirePlumber, with the headset unavailable, picked the
/// Junction from its history. JUNC must end with the built-in mic in use.
#[test]
fn restores_the_device_in_use_when_the_configured_one_is_unavailable() {
    let mut b = backend();
    let props = [
        ("media.class", "Audio/Source"),
        ("node.name", "builtin.mic"),
        ("node.description", "Built-in mic"),
    ]
    .into_iter()
    .collect();
    b.add_node(props, 2);
    b.graph_mut().defaults.source = Some("builtin.mic".into()); // in use
    let mut r = Reconciler::new();
    run(&mut b, &cfg("input", true), &mut r, 4);
    assert_eq!(source(&b).as_deref(), Some("junc.zoom"));
    assert_eq!(
        r.defaults().source.previous_in_use.as_deref(),
        Some("builtin.mic")
    );

    // Clearing restores the configured headset...
    let mut notices = run(&mut b, &cfg("none", true), &mut r, 1);
    assert_eq!(source(&b).as_deref(), Some("fake.hwin"));
    // ...which is unavailable: WirePlumber keeps using the Junction.
    b.graph_mut().defaults.source = Some("junc.zoom".into());
    notices.extend(run(&mut b, &cfg("none", true), &mut r, 3));
    assert_eq!(source(&b).as_deref(), Some("builtin.mic"));
    assert_eq!(b.graph().defaults.source.as_deref(), Some("builtin.mic"));
    assert_eq!(
        notices,
        [
            Notice::DefaultRestored {
                kind: DefaultKind::Source,
                to: Some("fake.hwin".into())
            },
            Notice::DefaultRestored {
                kind: DefaultKind::Source,
                to: Some("builtin.mic".into())
            },
        ]
    );
    assert!(
        run(&mut b, &cfg("none", true), &mut r, 2).is_empty(),
        "settled"
    );
}

/// The usual case is unchanged: configured and in use are the same device,
/// one restore, no second step.
#[test]
fn a_normal_restore_takes_one_step() {
    let mut b = backend();
    b.graph_mut().defaults.source = Some("fake.hwin".into());
    let mut r = Reconciler::new();
    run(&mut b, &cfg("input", true), &mut r, 4);
    assert_eq!(r.defaults().source.previous_in_use, None);
    let n = run(&mut b, &cfg("none", true), &mut r, 4);
    assert_eq!(n.len(), 1);
    assert_eq!(source(&b).as_deref(), Some("fake.hwin"));
}
