// SPDX-License-Identifier: GPL-3.0-or-later

//! Golden plans (`junc apply --dry-run --json`) on a recorded graph. The
//! files under tests/golden/plan/ are the expected JSON; `JUNC_BLESS=1`
//! rewrites them after an intended change.

use junc_core::backend::{Backend, MockBackend};
use junc_core::config;
use junc_core::graph::Graph;
use junc_core::model::Config;
use junc_core::plan::{Held, plan};
use junc_ipc::plan::{PlanView, view};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn graph() -> Graph {
    let text = std::fs::read_to_string(root().join("tests/fixtures/graphs/snd-aloop-stereo.json"))
        .expect("fixture");
    Graph::from_pw_dump(&text).expect("graph")
}

fn cfg(text: &str) -> Config {
    config::load(text, "junctions.toml").expect("config").config
}

fn golden(name: &str, v: &PlanView) {
    let path = root().join(format!("tests/golden/plan/{name}.json"));
    let json = serde_json::to_string_pretty(v).expect("json") + "\n";
    if std::env::var_os("JUNC_BLESS").is_some() {
        std::fs::write(&path, &json).expect("write golden");
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        want == json,
        "{} differs (JUNC_BLESS=1 cargo test -p junc-ipc --test plan rewrites it):\n{json}",
        path.display()
    );
}

const EMPTY: &str = "schema = 1\n";

const LINE: &str = r#"schema = 1
[[junction]]
id = "line"
name = "Line In"
default = "input"
[[junction.input]]
id = "aloop"
match = { device = "alsa_input.platform-snd_aloop.0.analog-stereo" }
[[junction.input]]
id = "spotify"
match = { binary = "spotify" }
[[junction.listen]]
id = "out"
target = "alsa_output.platform-snd_aloop.0.analog-stereo"
"#;

#[test]
fn creating_a_junction() {
    let (old, new, g) = (cfg(EMPTY), cfg(LINE), graph());
    let p = plan(&new, &g, &Held::new());
    let v = view(&old, &new, &p, Some(&g));
    assert!(!v.complete, "nodes first, links next round");
    golden("create-junction", &v);
}

#[test]
fn routing_once_the_nodes_exist_and_changing_it() {
    let new = cfg(LINE);
    let mut b = MockBackend::new(graph());
    for round in 0..3 {
        let p = plan(&new, b.graph(), &Held::new());
        if round == 1 {
            // The nodes exist; this round links them.
            golden("route-junction", &view(&new, &new, &p, Some(b.graph())));
        }
        for a in &p.actions {
            b.apply(a).expect("apply");
        }
    }
    // Converged: nothing to do.
    let p = plan(&new, b.graph(), &Held::new());
    let v = view(&new, &new, &p, Some(b.graph()));
    assert_eq!(v.summary, ["No changes"]);
    assert!(v.actions.is_empty());

    let changed = cfg(&LINE
        .replace("[[junction.input]]\nid = \"spotify\"\nmatch = { binary = \"spotify\" }\n", "")
        .replace("default = \"input\"\n", "volume = 80\n")
        .replace("[[junction.listen]]", "[[junction.input]]\nid = \"mon\"\nmatch = { monitor = \"alsa_output.platform-snd_aloop.0.analog-stereo\" }\nenabled = false\n[[junction.listen]]"));
    let p = plan(&changed, b.graph(), &Held::new());
    let v = view(&new, &changed, &p, Some(b.graph()));
    assert!(v.complete);
    golden("change-junction", &v);

    let p = plan(&cfg(EMPTY), b.graph(), &Held::new());
    golden(
        "delete-junction",
        &view(&new, &cfg(EMPTY), &p, Some(b.graph())),
    );
}
