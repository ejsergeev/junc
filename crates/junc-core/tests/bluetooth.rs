// SPDX-License-Identifier: GPL-3.0-or-later

//! F18 (B4) on the owner's headset dumps: profile, the call-profile warning
//! for a headset mic in A2DP, and the call codec's quality.

use junc_core::bluetooth::{CallQuality, Mode, headset, mic, mic_switches_profile};
use junc_core::graph::{Graph, Node};
use std::path::Path;

fn graph(name: &str) -> Graph {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/graphs")
        .join(name);
    Graph::from_pw_dump(&std::fs::read_to_string(&p).expect("fixture")).expect("graph")
}

/// The microphone the user would pick: `bluez_input.<address>`.
fn picked_mic(g: &Graph) -> &Node {
    g.nodes
        .values()
        .find(|n| n.name().starts_with("bluez_input.") && !n.media_class().ends_with("/Internal"))
        .expect("headset mic")
}

#[test]
fn a_headset_mic_in_playback_mode_warns_about_the_call_profile() {
    for (file, codec, quality) in [
        ("bt-airpods-a2dp.json", "msbc", CallQuality::Wideband),
        (
            "bt-pixel-buds-2a-a2dp.json",
            "lc3-swb",
            CallQuality::SuperWideband,
        ),
    ] {
        let g = graph(file);
        let m = picked_mic(&g);
        let h = mic(&g, m).expect(file);
        assert_eq!(h.mode, Mode::Playback, "{file}");
        assert_eq!(h.profile.as_deref(), Some("a2dp-sink"), "{file}");
        // Not live yet: the codec comes from the call profile's description.
        assert_eq!(h.call_codec.as_deref(), Some(codec), "{file}");
        assert_eq!(h.call_quality, Some(quality), "{file}");
        assert!(mic_switches_profile(&g, m), "{file}");
    }
}

#[test]
fn a_headset_already_in_a_call_does_not_warn() {
    for (file, codec, quality) in [
        ("bt-airpods-hfp.json", "msbc", CallQuality::Wideband),
        (
            "bt-pixel-buds-2a-hfp.json",
            "lc3-swb",
            CallQuality::SuperWideband,
        ),
    ] {
        let g = graph(file);
        let m = picked_mic(&g);
        let h = mic(&g, m).expect(file);
        assert_eq!(h.mode, Mode::Call, "{file}");
        // Live: the codec comes from the nodes.
        assert_eq!(h.call_codec.as_deref(), Some(codec), "{file}");
        assert_eq!(h.call_quality, Some(quality), "{file}");
        assert!(!mic_switches_profile(&g, m), "{file}");
    }
}

#[test]
fn other_nodes_and_cards_are_not_headsets() {
    let g = graph("asahi-m1-j293.json");
    assert!(g.devices.values().all(|d| headset(&g, d).is_none()));
    assert!(g.nodes.values().all(|n| !mic_switches_profile(&g, n)));
    // The headset's own speaker output is not a mic.
    let g = graph("bt-airpods-a2dp.json");
    let out = g
        .nodes
        .values()
        .find(|n| n.name().starts_with("bluez_output."))
        .expect("output");
    assert!(mic(&g, out).is_none());
    let card = g.devices.values().find(|d| d.is_bluetooth()).expect("card");
    assert_eq!(headset(&g, card).expect("headset").mode, Mode::Playback);
}
