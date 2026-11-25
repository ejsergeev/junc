// SPDX-License-Identifier: GPL-3.0-or-later

//! Graph loading and identity matching against recorded `pw-dump` fixtures
//! from the owner's laptop (tests/fixtures/graphs/README.md).

use junc_core::graph::{Direction, Graph, NodeKind};
use junc_core::matcher::{Use, junction_node_name, resolve, resolve_output};
use junc_core::model::Match;
use std::path::Path;

fn graph(name: &str) -> Graph {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/graphs")
        .join(name);
    let json = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    Graph::from_pw_dump(&json).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn names(g: &Graph, ids: &[u32]) -> Vec<String> {
    ids.iter().map(|i| g.nodes[i].name().to_owned()).collect()
}

fn app(binary: Option<&str>, app: Option<&str>, role: Option<&str>) -> Match {
    Match {
        binary: binary.map(Into::into),
        app: app.map(Into::into),
        role: role.map(Into::into),
        ..Match::default()
    }
}

#[test]
fn browser_with_two_streams_matches_both() {
    let g = graph("apps-browser-two-streams.json");
    let ids = resolve(&app(Some("firefox"), None, None), Use::Input, &g);
    assert_eq!(
        names(&g, &ids),
        ["Firefox", "Firefox"],
        "both tabs are captured"
    );
    // App names compare case-insensitively; all given keys must match.
    assert_eq!(
        resolve(&app(None, Some("firefox"), None), Use::Input, &g).len(),
        2
    );
    assert!(
        resolve(
            &app(Some("firefox"), None, Some("Communication")),
            Use::Input,
            &g
        )
        .is_empty()
    );
    assert_eq!(
        names(
            &g,
            &resolve(&app(Some("spotify"), None, None), Use::Input, &g)
        ),
        ["spotify"]
    );
}

#[test]
fn recording_apps_are_cable_targets_not_inputs() {
    let g = graph("apps-browser-two-streams.json");
    let discord = app(Some("Discord"), None, None);
    assert!(resolve(&discord, Use::Input, &g).is_empty());
    assert_eq!(
        names(&g, &resolve(&discord, Use::CableTarget, &g)),
        ["Discord"]
    );
}

#[test]
fn defaults_come_from_metadata() {
    let g = graph("asahi-m1-j293.json");
    // On Asahi the user picked the headphones but the DSP sink is in use.
    assert_eq!(
        g.defaults.sink.as_deref(),
        Some("audio_effect.j293-convolver")
    );
    assert_eq!(
        g.defaults.configured_sink.as_deref(),
        Some("alsa_output.platform-sound.HiFi__Headphones__sink")
    );
    let out = resolve_output("default", &g).expect("default sink visible");
    assert_eq!(g.nodes[&out].name(), "audio_effect.j293-convolver");
}

#[test]
fn asahi_speakers_are_unreachable_and_dsp_streams_are_not_apps() {
    let g = graph("asahi-m1-j293.json");
    // The DSP output stream links to a speaker node the client cannot see.
    let conv = g
        .node_by_name("effect_output.j293-convolver")
        .expect("dsp stream");
    assert!(
        g.links
            .values()
            .any(|l| l.output_node == conv.id && !g.nodes.contains_key(&l.input_node))
    );
    // Only the DSP sink and the headphone jack can be Listen targets.
    let sinks: Vec<&str> = g
        .nodes
        .values()
        .filter(|n| n.kind() == NodeKind::Sink)
        .map(|n| n.name())
        .collect();
    assert_eq!(
        sinks,
        [
            "alsa_output.platform-sound.HiFi__Headphones__sink",
            "audio_effect.j293-convolver"
        ]
    );
    assert!(resolve_output("alsa_output.platform-sound.RawSpeakers", &g).is_none());
    // Filter-chain and loopback streams are plumbing, not apps.
    assert!(
        g.nodes
            .values()
            .filter(|n| n.media_class().starts_with("Stream/"))
            .all(|n| n.is_plumbing())
    );
    assert_eq!(g.nodes.values().filter(|n| n.is_app_stream()).count(), 0);
}

#[test]
fn hardware_inputs_match_by_node_name_then_description() {
    let g = graph("asahi-m1-j293.json");
    let by_name = Match {
        device: Some("alsa_input.platform-sound.HiFi__Headset__source".into()),
        ..Match::default()
    };
    assert_eq!(resolve(&by_name, Use::Input, &g).len(), 1);
    let by_desc = Match {
        device: Some("Headset Microphone".into()),
        ..Match::default()
    };
    assert_eq!(
        names(&g, &resolve(&by_desc, Use::Input, &g)),
        ["alsa_input.platform-sound.HiFi__Headset__source"]
    );
    let missing = Match {
        device: Some("alsa_input.usb-gone".into()),
        ..Match::default()
    };
    assert!(
        resolve(&missing, Use::Input, &g).is_empty(),
        "unplugged device is waiting, not an error"
    );
}

#[test]
fn pro_audio_is_offered_only_when_it_adds_channels_in_principle() {
    let aloop = graph("snd-aloop-stereo.json");
    let dev = aloop.devices.values().next().expect("loopback card");
    assert!(dev.can_switch_to_pro_audio());
    assert_eq!(
        dev.active_profile.as_deref(),
        Some("output:analog-stereo+input:analog-stereo")
    );
    let pro = graph("snd-aloop-proaudio.json");
    let src = pro
        .nodes
        .values()
        .find(|n| n.name().ends_with("pro-input-0"))
        .expect("pro input");
    assert_eq!(pro.ports_of(src.id, Direction::Out).count(), 32);
    // The Komplete Audio 2 has a pro-audio profile too; channel counts come
    // from ALSA (architecture §3.8), here both profiles have 2 channels.
    let k2 = graph("komplete-audio-2-proaudio.json");
    let src = k2
        .nodes
        .values()
        .find(|n| n.kind() == NodeKind::Source)
        .expect("k2 input");
    assert_eq!(k2.ports_of(src.id, Direction::Out).count(), 2);
}

#[test]
fn bluetooth_headsets_expose_a_virtual_mic_in_a2dp() {
    for f in ["bt-airpods-a2dp.json", "bt-pixel-buds-2a-a2dp.json"] {
        let g = graph(f);
        let dev = g
            .devices
            .values()
            .find(|d| d.is_bluetooth())
            .expect("bt device");
        assert_eq!(dev.active_profile.as_deref(), Some("a2dp-sink"), "{f}");
        let mic = g
            .nodes
            .values()
            .find(|n| n.name().starts_with("bluez_input."))
            .expect("virtual mic");
        assert_eq!(
            mic.kind(),
            NodeKind::Source,
            "{f}: listed as a source the user can pick"
        );
    }
}

#[test]
fn junction_node_names() {
    assert_eq!(
        junction_node_name("virtual-mic-for-zoom"),
        "junc.virtual_mic_for_zoom"
    );
}
