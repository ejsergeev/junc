// SPDX-License-Identifier: GPL-3.0-or-later

//! F15 (B3) on layer-1 fixtures: Pro Audio detection, channel groups,
//! binding an Input to the Pro Audio node, and profile memory.

use junc_core::backend::{Action, Backend, MockBackend};
use junc_core::config;
use junc_core::graph::{Direction, Graph};
use junc_core::matcher::{Use, resolve};
use junc_core::model::{Config, Match};
use junc_core::proaudio::{
    self, ProAudioOffer, UsbChannels, card_name, channel_groups, parse_usb_stream,
};
use junc_core::reconcile::{Notice, Reconciler, Step};
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

const MIX: &str = r#"
schema = 1
[[junction]]
id = "mix"
name = "Mix"
[[junction.input]]
id = "desk"
match = { device = "alsa_input.usb-JUNC_Test_18in-00.analog-stereo", profile = "pro-audio" }
"#;

fn step(b: &mut MockBackend, cfg: &Config, r: &mut Reconciler) -> Step {
    let s = r.on_graph(cfg, b.graph());
    for a in &s.actions {
        b.apply(a).unwrap_or_else(|e| panic!("{a}: {e}"));
    }
    s
}

fn card_id(g: &Graph) -> u32 {
    g.devices
        .values()
        .find(|d| d.name() == CARD)
        .expect("card")
        .id
}

/// The card switched, as PipeWire shows it afterwards.
fn now_pro(b: &mut MockBackend) {
    *b.graph_mut() = graph("usb-18in-proaudio.json");
}

#[test]
fn the_profile_is_applied_on_plug_kept_and_the_users_choice_remembered() {
    let cfg = config::load(MIX, "t.toml").expect("config").config;
    let mut b = MockBackend::new(graph("usb-18in-stereo.json"));
    let mut r = Reconciler::new();

    // Plugged in on its stereo profile: JUNC switches it, once.
    let s = step(&mut b, &cfg, &mut r);
    let switches: Vec<&Action> = s
        .actions
        .iter()
        .filter(|a| matches!(a, Action::SetDeviceProfile { .. }))
        .collect();
    assert_eq!(switches.len(), 1, "{:?}", s.actions);
    assert!(
        matches!(switches[0], Action::SetDeviceProfile { profile, .. } if profile == "pro-audio")
    );
    let again = r.on_graph(&cfg, b.graph());
    assert!(
        !again
            .actions
            .iter()
            .any(|a| matches!(a, Action::SetDeviceProfile { .. })),
        "not asked twice while the switch is under way"
    );

    // The card comes back in Pro Audio: the Input takes all 18 channels.
    now_pro(&mut b);
    for _ in 0..4 {
        step(&mut b, &cfg, &mut r);
    }
    let gain = b.graph().node_by_name("junc.mix.in.desk").expect("gain").id;
    let into_gain = b
        .graph()
        .links
        .values()
        .filter(|l| l.input_node == gain)
        .count();
    assert_eq!(into_gain, 2, "channels 1–2 of 18 feed the 2-channel Bus");
    assert!(r.on_graph(&cfg, b.graph()).actions.is_empty(), "converged");

    // The user picks the stereo profile elsewhere: JUNC keeps it and saves
    // it, without switching back.
    let id = card_id(b.graph());
    let stereo = "output:analog-stereo+input:analog-stereo";
    b.graph_mut()
        .devices
        .get_mut(&id)
        .expect("card")
        .active_profile = Some(stereo.into());
    let s = step(&mut b, &cfg, &mut r);
    assert_eq!(
        s.notices,
        [Notice::ProfileChanged {
            junction: "mix".into(),
            input: "desk".into(),
            profile: Some(stereo.into()),
        }]
    );
    let saved = s.config.expect("config change");
    assert_eq!(
        saved.junction[0].input[0].matcher.profile.as_deref(),
        Some(stereo)
    );
    assert!(
        !s.actions
            .iter()
            .any(|a| matches!(a, Action::SetDeviceProfile { .. }))
    );
    let s = step(&mut b, &saved, &mut r);
    assert!(s.notices.is_empty() && s.config.is_none(), "{s:?}");

    // Re-plugged (a new card) with the original config: Pro Audio again.
    let mut b = MockBackend::new(graph("usb-18in-stereo.json"));
    let id = card_id(b.graph());
    b.graph_mut()
        .devices
        .get_mut(&id)
        .expect("card")
        .props
        .set("object.serial", "99999");
    let s = step(&mut b, &cfg, &mut r);
    assert!(
        s.actions
            .iter()
            .any(|a| matches!(a, Action::SetDeviceProfile { .. })),
        "{:?}",
        s.actions
    );
}

#[test]
fn a_card_turned_off_is_left_off_and_the_config_kept() {
    let cfg = config::load(MIX, "t.toml").expect("config").config;
    let mut b = MockBackend::new(graph("usb-18in-proaudio.json"));
    let mut r = Reconciler::new();
    for _ in 0..3 {
        step(&mut b, &cfg, &mut r);
    }
    let id = card_id(b.graph());
    b.graph_mut()
        .devices
        .get_mut(&id)
        .expect("card")
        .active_profile = Some("off".into());
    let s = step(&mut b, &cfg, &mut r);
    assert_eq!(
        s.notices,
        [Notice::ProfileChanged {
            junction: "mix".into(),
            input: "desk".into(),
            profile: None,
        }]
    );
    assert!(s.config.is_none());
    for _ in 0..3 {
        let s = step(&mut b, &cfg, &mut r);
        assert!(
            !s.actions
                .iter()
                .any(|a| matches!(a, Action::SetDeviceProfile { .. })),
            "not switched back on"
        );
    }
}

/// UCM cards (the Asahi built-in audio) set up amplifiers and speaker
/// processing; Pro Audio would bypass that. Never offered, never switched,
/// even when a config asks for it.
#[test]
fn ucm_cards_never_get_pro_audio() {
    let g = graph("asahi-m1-j293.json");
    for d in g
        .devices
        .values()
        .filter(|d| d.props.get("device.api") == Some("alsa"))
    {
        assert!(d.uses_ucm(), "{}", d.name());
        assert_eq!(proaudio::offer(&g, d), None, "{}", d.name());
    }
    for name in ["snd-aloop-stereo.json", "usb-18in-stereo.json"] {
        let g = graph(name);
        assert!(g.devices.values().all(|d| !d.uses_ucm()), "{name}");
    }
    let cfg = config::load(
        &MIX.replace(
            "alsa_input.usb-JUNC_Test_18in-00.analog-stereo",
            "alsa_input.platform-sound.HiFi__Headset__source",
        ),
        "t.toml",
    )
    .expect("config")
    .config;
    let mut b = MockBackend::new(graph("asahi-m1-j293.json"));
    let mut r = Reconciler::new();
    for _ in 0..3 {
        let s = step(&mut b, &cfg, &mut r);
        assert!(
            !s.actions
                .iter()
                .any(|a| matches!(a, Action::SetDeviceProfile { .. })),
            "{:?}",
            s.actions
        );
    }
}
