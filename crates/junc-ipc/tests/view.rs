// SPDX-License-Identifier: GPL-3.0-or-later

//! JSON views: statuses and the channel links in use, from a recorded graph.

use junc_core::config;
use junc_core::graph::Graph;
use junc_core::plan::{Entity, EntityStatus, Status};
use junc_ipc::view::{self, State};

const CONFIG: &str = r#"schema = 1
[[junction]]
id = "mix"
name = "Mix"
bus = { channels = 4 }
[[junction.input]]
id = "line"
match = { device = "alsa_input.platform-snd_aloop.0.analog-stereo" }
[[junction.input]]
id = "custom"
match = { device = "alsa_input.platform-snd_aloop.0.analog-stereo" }
links = [[1, 3], [2, 4]]
[[junction.input]]
id = "gone"
match = { device = "alsa_input.usb-not-here" }
name = "USB Microphone"
[[junction.listen]]
id = "out"
target = "alsa_output.platform-snd_aloop.0.analog-stereo"
"#;

#[test]
fn views_report_statuses_and_links_in_use() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/graphs/snd-aloop-stereo.json"
    );
    let graph =
        Graph::from_pw_dump(&std::fs::read_to_string(path).expect("fixture")).expect("graph");
    let cfg = config::load(CONFIG, "junctions.toml")
        .expect("config")
        .config;
    let line_in = graph
        .node_by_name("alsa_input.platform-snd_aloop.0.analog-stereo")
        .expect("node")
        .id;
    let st = |entity, status, nodes| EntityStatus {
        junction: "mix".into(),
        entity,
        status,
        nodes,
    };
    // What the planner reports once the Junction's own nodes exist.
    let statuses = [
        st(Entity::Junction, Status::Active, vec![]),
        st(Entity::Input("line".into()), Status::Active, vec![line_in]),
        st(
            Entity::Input("custom".into()),
            Status::Active,
            vec![line_in],
        ),
        st(Entity::Input("gone".into()), Status::Waiting, vec![]),
        st(Entity::Listen("out".into()), Status::Active, vec![]),
    ];
    let j = view::junction(&cfg.junction[0], &statuses, Some(&graph));
    assert_eq!(j.status, State::Active);

    let line = &j.inputs[0];
    assert_eq!(line.status, State::Active);
    assert_eq!(
        line.matched,
        ["alsa_input.platform-snd_aloop.0.analog-stereo"]
    );
    assert_eq!(line.links, [[1, 1], [2, 2]], "stereo into a 4-channel bus");
    assert!(!line.custom_links);

    assert_eq!(j.inputs[1].links, [[1, 3], [2, 4]]);
    assert!(j.inputs[1].custom_links);

    assert_eq!(j.inputs[2].status, State::Waiting);
    assert_eq!(
        j.inputs[2].name.as_deref(),
        Some("USB Microphone"),
        "the config's name, shown while the device is away"
    );
    assert_eq!(line.name, None);
    assert!(
        j.inputs[2].links.is_empty(),
        "unknown until the device appears"
    );

    assert_eq!(j.listens[0].links, [[1, 1], [2, 2]]);
    assert_eq!(j.path, "/io/github/ejsergeev/Junc1/junction/mix");

    let json = serde_json::to_value(&j).expect("json");
    assert_eq!(json["inputs"][0]["match"]["device"], line.matched[0]);
}

/// F18 (B4): describe marks a headset mic in A2DP so agents and the picker
/// can warn before adding it; the card shows its profile and call quality.
#[test]
fn bluetooth_mics_carry_the_call_profile_warning() {
    let load = |f: &str| {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/graphs")
            .join(f);
        Graph::from_pw_dump(&std::fs::read_to_string(p).expect("fixture")).expect("graph")
    };
    let g = load("bt-pixel-buds-2a-a2dp.json");
    let c = view::candidates(&g);
    let mic = c
        .iter()
        .find(|k| k.node == "bluez_input.00:00:5E:00:53:02")
        .expect("mic candidate");
    let bt = mic.bluetooth.as_ref().expect("bluetooth");
    assert!(bt.switches_to_call);
    assert_eq!(bt.mode, "playback");
    assert_eq!(bt.call_quality.as_deref(), Some("super-wideband"));
    assert!(
        c.iter()
            .filter(|k| k.node != mic.node)
            .all(|k| k.bluetooth.is_none()),
        "only the mic carries it"
    );
    let d = view::devices(&g);
    let card = d.iter().find(|d| d.api == "bluez5").expect("card");
    let bt = card.bluetooth.as_ref().expect("bluetooth");
    assert_eq!(bt.profile.as_deref(), Some("a2dp-sink"));
    assert!(!bt.switches_to_call, "the card itself is not a mic");

    let g = load("bt-airpods-hfp.json");
    let mic = view::candidates(&g)
        .into_iter()
        .find(|k| k.node == "bluez_input.00:00:5E:00:53:01")
        .expect("mic candidate");
    let bt = mic.bluetooth.expect("bluetooth");
    assert!(!bt.switches_to_call, "already in a call");
    assert_eq!(bt.call_codec.as_deref(), Some("msbc"));
}

/// The device side of a loopback or filter chain is what users pick: the
/// Asahi speakers and mic behind their DSP, Omarchy's mic. Their stream
/// sides are plumbing and stay hidden.
#[test]
fn candidates_list_the_device_side_of_loopbacks_and_filter_chains() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/graphs/asahi-m1-j293.json");
    let g = Graph::from_pw_dump(&std::fs::read_to_string(p).expect("fixture")).expect("graph");
    let names: Vec<String> = view::candidates(&g).into_iter().map(|c| c.node).collect();
    for shown in [
        "omarchy_asahi_mic",
        "audio_effect.j293-convolver",
        "effect_output.j293-mic",
    ] {
        assert!(
            names.iter().any(|n| n == shown),
            "{shown} missing: {names:?}"
        );
    }
    for hidden in [
        "input.omarchy_asahi_mic",
        "audio_effect.j293-mic",
        "effect_output.j293-convolver",
    ] {
        assert!(!names.iter().any(|n| n == hidden), "{hidden} listed");
    }
}
