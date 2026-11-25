// SPDX-License-Identifier: GPL-3.0-or-later

//! F15 (B3) on layer-1 fixtures: Pro Audio detection, channel groups
//! and binding an Input to the Pro Audio node.

use junc_core::graph::{Direction, Graph};
use junc_core::matcher::{Use, resolve};
use junc_core::model::Match;
use junc_core::proaudio::{
    self, ProAudioOffer, UsbChannels, card_name, channel_groups, parse_usb_stream,
};
use std::path::Path;

fn fixture(dir: &str, name: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(dir)
        .join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn graph(name: &str) -> Graph {
    Graph::from_pw_dump(&fixture("graphs", name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// The fixture graph with ALSA's channel counts filled in, as the backend
/// does for USB cards.
fn with_usb(mut g: Graph, stream: &str) -> Graph {
    let c = parse_usb_stream(&fixture("asound", stream));
    for d in g.devices.values_mut() {
        if d.props.get("device.bus") == Some("usb") {
            d.usb_channels = c;
        }
    }
    g
}

const CARD: &str = "alsa_card.usb-JUNC_Test_18in-00";
const STEREO_IN: &str = "alsa_input.usb-JUNC_Test_18in-00.analog-stereo";

#[test]
fn alsa_stream_descriptions_give_the_full_channel_counts() {
    assert_eq!(
        parse_usb_stream(&fixture("asound", "usb-18in-stream0.txt")),
        Some(UsbChannels {
            playback: 20,
            capture: 18
        })
    );
    assert_eq!(
        parse_usb_stream(&fixture("asound", "komplete-audio-2-stream0.txt")),
        Some(UsbChannels {
            playback: 2,
            capture: 2
        })
    );
    assert_eq!(parse_usb_stream("no channels here"), None);
}

#[test]
fn pro_audio_is_offered_only_when_it_adds_inputs() {
    let offer = |g: &Graph| {
        let d = g
            .devices
            .values()
            .find(|d| d.props.get("device.api") == Some("alsa"))
            .expect("card");
        proaudio::offer(g, d)
    };
    // 18-input USB card on its stereo profile: "2 of 18 channels".
    let g = with_usb(graph("usb-18in-stereo.json"), "usb-18in-stream0.txt");
    assert_eq!(
        offer(&g),
        Some(ProAudioOffer {
            now: 2,
            all: Some(18)
        })
    );
    // Already in Pro Audio: nothing to offer.
    let g = with_usb(graph("usb-18in-proaudio.json"), "usb-18in-stream0.txt");
    assert_eq!(offer(&g), None);
    // Komplete Audio 2: Pro Audio exists but gives the same 2 inputs.
    let g = with_usb(
        graph("komplete-audio-2-stereo.json"),
        "komplete-audio-2-stream0.txt",
    );
    assert_eq!(offer(&g), None);
    // A non-USB card: ALSA does not tell, "Pro Audio available".
    let g = graph("snd-aloop-stereo.json");
    assert_eq!(offer(&g), Some(ProAudioOffer { now: 2, all: None }));
}

#[test]
fn channels_come_in_groups_of_eight() {
    assert_eq!(channel_groups(18), [[1, 8], [9, 16], [17, 18]]);
    assert_eq!(channel_groups(8), [[1, 8]]);
    assert_eq!(channel_groups(2), [[1, 2]]);
    assert!(channel_groups(0).is_empty());
    let g = graph("snd-aloop-proaudio.json");
    let d = g.devices.values().next().expect("card");
    assert_eq!(proaudio::capture_channels(&g, d), 32);
}

#[test]
fn the_card_is_found_from_a_node_that_is_gone() {
    // The stereo node does not exist in Pro Audio; its name still leads to
    // the card.
    let pro = graph("usb-18in-proaudio.json");
    assert!(pro.node_by_name(STEREO_IN).is_none());
    assert_eq!(card_name(&pro, STEREO_IN).as_deref(), Some(CARD));
    assert_eq!(card_name(&pro, CARD).as_deref(), Some(CARD));
    let aloop = graph("snd-aloop-proaudio.json");
    assert_eq!(
        card_name(&aloop, "alsa_input.platform-snd_aloop.0.analog-stereo").as_deref(),
        Some("alsa_card.platform-snd_aloop.0")
    );
    assert_eq!(card_name(&aloop, "fake.hwin"), None);
}

#[test]
fn a_pro_audio_input_waits_for_the_profile_then_takes_every_channel() {
    let m = Match {
        device: Some(STEREO_IN.into()),
        profile: Some("pro-audio".into()),
        ..Match::default()
    };
    let stereo = graph("usb-18in-stereo.json");
    assert!(
        resolve(&m, Use::Input, &stereo).is_empty(),
        "waits while the card is not in Pro Audio"
    );
    let pro = graph("usb-18in-proaudio.json");
    let nodes = resolve(&m, Use::Input, &pro);
    assert_eq!(nodes.len(), 1);
    assert_eq!(pro.ports_of(nodes[0], Direction::Out).count(), 18);
    // Without a profile the stereo name simply waits in Pro Audio.
    let plain = Match { profile: None, ..m };
    assert!(resolve(&plain, Use::Input, &pro).is_empty());
}
