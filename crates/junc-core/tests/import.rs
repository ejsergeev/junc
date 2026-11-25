// SPDX-License-Identifier: GPL-3.0-or-later

//! F13 (B5) on real dumps: what the scan must not offer. The layer-2 test
//! in `junc-cli` covers real `pactl` null sinks and `pw-loopback`.

use junc_core::graph::Graph;
use junc_core::import::scan;
use std::path::Path;

fn graph(name: &str) -> Graph {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/graphs")
        .join(name);
    Graph::from_pw_dump(&std::fs::read_to_string(&p).expect("fixture")).expect("graph")
}

/// The Asahi DSP filter chains, Omarchy's mic (a loopback that presents a
/// virtual source) and a Bluetooth headset's mic loopback are part of the
/// system, not routing setups to import.
#[test]
fn system_loopbacks_and_filter_chains_are_not_offered() {
    for f in [
        "asahi-m1-j293.json",
        "bt-airpods-a2dp.json",
        "bt-pixel-buds-2a-hfp.json",
        "snd-aloop-proaudio.json",
    ] {
        let found = scan(&graph(f));
        assert!(found.is_empty(), "{f}: {found:?}");
    }
}

/// The isolated test session's fake devices are null sinks from a config
/// fragment: offered, marked as such, with nothing connected.
#[test]
fn config_null_sinks_are_offered_as_config() {
    let found = scan(&graph("apps-browser-two-streams.json"));
    assert!(!found.is_empty());
    for i in &found {
        assert_eq!(i.kind, "null_sink");
        assert_eq!(i.origin, "config", "{i:?}");
        assert_eq!(i.suggested, "junction");
    }
}
