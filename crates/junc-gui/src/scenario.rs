// SPDX-License-Identifier: GPL-3.0-or-later

//! Scenarios (architecture §7.5, feature `scenarios`): a fixture in
//! `tests/fixtures/scenarios/<name>.toml` holds a config, a mocked graph
//! (`tests/fixtures/graphs/`), the theme and window size of its design
//! board. The mock IPC runs the config against the graph until it settles,
//! as juncd would, and builds the `Describe` snapshot juncd would send.

use crate::actions::{Actions, Command};
use crate::app::App;
use crate::checks;
use crate::edits;
use crate::main_view::End;
use crate::onboarding::Step;
use junc_core::backend::{Backend, MockBackend};
use junc_core::graph::{Graph, Props};
use junc_core::model::Config;
use junc_core::reconcile::Reconciler;
use junc_ipc::view::{self, DefaultsView, Describe};
use serde::Deserialize;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    #[allow(dead_code)] // documents the design board
    board: u32,
    title: String,
    /// Another scenario this one starts from (bases nest): its config,
    /// default output and so on unless given here, and its nodes before
    /// these.
    #[serde(default)]
    base: Option<String>,
    /// Junctions (`"<junction>"`) and Inputs (`"<junction>/<input>"`) to
    /// turn off.
    #[serde(default)]
    off: Vec<String>,
    /// Nodes to leave out of the graph (an unplugged device).
    #[serde(default)]
    unplug: Vec<String>,
    #[serde(default)]
    theme: Option<String>,
    #[serde(default)]
    size: Option<[u32; 2]>,
    /// juncd's state; `down` means juncd is not running.
    #[serde(default)]
    daemon: Option<String>,
    #[serde(default)]
    graph: Option<String>,
    #[serde(default)]
    config: Option<String>,
    /// Nodes added to the graph (apps, devices, outputs).
    #[serde(default)]
    node: Vec<NodeFixture>,
    /// `node.name` of the default output.
    #[serde(default)]
    default_output: Option<String>,
    /// The link under the pointer: `"<Input or Listen id> <a>-><b>"`.
    #[serde(default)]
    hover: Option<String>,
    /// Show Listen off.
    #[serde(default)]
    hide_listen: bool,
    /// The Junction to select (else the first).
    #[serde(default)]
    select: Option<String>,
    /// Onboarding's step: `check` or `templates`.
    #[serde(default)]
    onboarding: Option<String>,
    /// What `junc doctor --json` answers: its checks.
    #[serde(default)]
    check: Vec<CheckFixture>,
    /// The Add Input picker open on the selected Junction.
    #[serde(default)]
    picker: bool,
    /// The Add Listen output picker open on it.
    #[serde(default)]
    listen_picker: bool,
    /// The Add Bus channels popover open on it.
    #[serde(default)]
    add_bus: bool,
    /// The header's name field open (board 26).
    #[serde(default)]
    rename: bool,
    /// The delete confirmation open (board 22).
    #[serde(default)]
    delete: bool,
    /// The selected row's context menu open, its cursor on this item
    /// (board 21).
    menu: Option<i32>,
    /// A New Cable, its From picker open (board 27).
    #[serde(default)]
    new_cable: bool,
    /// The Options panel of this Input card (0-based) open.
    options: Option<i32>,
    /// A link being made, as `"<input> <channel> -> <bus channel>"`, by the
    /// pointer, or with `link_keys` by the keys (boards 13, 14).
    link: Option<String>,
    #[serde(default)]
    link_keys: bool,
}

/// One doctor check (`junc doctor --json`).
#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct CheckFixture {
    id: String,
    status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fix: Option<String>,
}

/// A node of the mocked graph.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeFixture {
    name: String,
    /// `media.class`, e.g. `Stream/Output/Audio`, `Audio/Source`.
    class: String,
    #[serde(default = "two")]
    channels: u8,
    #[serde(default)]
    description: Option<String>,
    /// `application.name` and `application.process.binary` (apps).
    #[serde(default)]
    app: Option<String>,
    #[serde(default)]
    binary: Option<String>,
}

fn two() -> u8 {
    2
}

impl NodeFixture {
    fn props(&self) -> Props {
        let mut p: Vec<(&str, String)> = vec![
            ("node.name", self.name.clone()),
            ("media.class", self.class.clone()),
        ];
        if let Some(d) = &self.description {
            p.push(("node.description", d.clone()));
        }
        if let Some(a) = &self.app {
            p.push(("application.name", a.clone()));
        }
        if let Some(b) = &self.binary {
            p.push(("application.process.binary", b.clone()));
        }
        p.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
    }
}

/// A loaded scenario.
#[derive(Debug, Clone)]
pub struct Scenario {
    pub name: String,
    pub title: String,
    /// `dark` (Tokyo Night) or `light` (Flexoki Light).
    pub theme: String,
    pub size: (u32, u32),
    /// What juncd would answer; `None` when juncd is down.
    pub describe: Option<Describe>,
    /// The link under the pointer (see the fixture's `hover`).
    pub hover: Option<String>,
    pub hide_listen: bool,
    pub select: Option<String>,
    pub onboarding: Option<Step>,
    pub picker: bool,
    pub listen_picker: bool,
    pub add_bus: bool,
    pub rename: bool,
    pub delete: bool,
    pub menu: Option<i32>,
    pub new_cable: bool,
    pub options: Option<i32>,
    pub link: Option<String>,
    pub link_keys: bool,
    /// What the mock answers to the system check.
    checks: Vec<checks::Row>,
    /// What the mock IPC settles again after each change.
    config: Option<Config>,
    graph: Graph,
    state: String,
}

/// The repository's test fixtures (`$JUNC_FIXTURES` overrides it).
pub fn fixtures() -> PathBuf {
    std::env::var_os("JUNC_FIXTURES")
        .filter(|v| !v.is_empty())
        .map_or_else(
            || PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures")),
            PathBuf::from,
        )
}

/// Reads fixture `name`, with what its `base` (and the base's base) gives.
fn read(dir: &std::path::Path, name: &str, depth: u8) -> Result<Fixture, String> {
    let path = dir.join("scenarios").join(format!("{name}.toml"));
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut f: Fixture = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(base) = f.base.take() {
        if depth > 4 {
            return Err(format!("{name}: bases nest too deep"));
        }
        let b = read(dir, &base, depth + 1)?;
        f.config = f.config.or(b.config);
        f.graph = f.graph.or(b.graph);
        f.default_output = f.default_output.or(b.default_output);
        f.theme = f.theme.or(b.theme);
        f.size = f.size.or(b.size);
        f.daemon = f.daemon.or(b.daemon);
        f.select = f.select.or(b.select);
        f.onboarding = f.onboarding.or(b.onboarding);
        if f.check.is_empty() {
            f.check = b.check;
        }
        // The base's nodes, then this one's.
        f.node = b.node.into_iter().chain(f.node).collect();
    }
    Ok(f)
}

/// Loads `tests/fixtures/scenarios/<name>.toml`.
pub fn load(name: &str) -> Result<Scenario, String> {
    let dir = fixtures();
    let mut f = read(&dir, name, 0)?;
    f.node.retain(|n| !f.unplug.contains(&n.name));
    let state = f.daemon.unwrap_or_else(|| "connected".to_owned());
    let mut graph = match &f.graph {
        Some(g) => {
            let p = dir.join("graphs").join(g);
            let json = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            Graph::from_pw_dump(&json)?
        }
        None => Graph::default(),
    };
    if !f.node.is_empty() {
        let mut b = MockBackend::new(graph);
        for n in &f.node {
            b.add_node(n.props(), n.channels);
        }
        graph = b.graph().clone();
    }
    if let Some(out) = &f.default_output {
        graph.defaults.sink = Some(out.clone());
    }
    let config = if state == "down" {
        None
    } else {
        let config_text = f.config.unwrap_or_else(|| "schema = 1\n".to_owned());
        Some(
            junc_core::config::load(&config_text, &format!("{name}.toml"))
                .map_err(|d| {
                    format!(
                        "{name}: {}",
                        d.iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("; ")
                    )
                })?
                .config,
        )
    };
    let mut config = config;
    if let Some(c) = config.as_mut() {
        for spec in &f.off {
            let (j, i) = spec.split_once('/').unwrap_or((spec, ""));
            let junction = c
                .junction
                .iter_mut()
                .find(|x| x.id == j)
                .ok_or_else(|| format!("{name}: off {spec:?}: no such Junction"))?;
            if i.is_empty() {
                junction.enabled = false;
                continue;
            }
            let input = junction
                .input
                .iter_mut()
                .find(|x| x.id == i)
                .ok_or_else(|| format!("{name}: off {spec:?}: no such Input"))?;
            input.enabled = false;
        }
    }
    let onboarding = match f.onboarding.as_deref() {
        None => None,
        Some("check") => Some(Step::Check),
        Some("templates") => Some(Step::Templates),
        Some(o) => {
            return Err(format!(
                "{name}: onboarding {o:?} is not check or templates"
            ));
        }
    };
    let doctor = serde_json::to_string(&f.check).map_err(|e| e.to_string())?;
    let checks = checks::parse(&doctor)?;
    Ok(Scenario {
        name: name.to_owned(),
        title: f.title,
        theme: f.theme.unwrap_or_else(|| "dark".to_owned()),
        size: f.size.map_or((1440, 900), |[w, h]| (w, h)),
        describe: config.as_ref().map(|c| settle(c, graph.clone(), &state)),
        hover: f.hover,
        hide_listen: f.hide_listen,
        select: f.select,
        onboarding,
        picker: f.picker,
        listen_picker: f.listen_picker,
        add_bus: f.add_bus,
        rename: f.rename,
        delete: f.delete,
        menu: f.menu,
        new_cable: f.new_cable,
        options: f.options,
        link: f.link,
        link_keys: f.link_keys,
        checks,
        config,
        graph,
        state,
    })
}

/// The scenario's mock IPC: applies the window's requests to the fixture's
/// config and settles it again, as juncd would.
#[derive(Clone)]
pub struct Mock {
    inner: Rc<RefCell<MockState>>,
}

struct MockState {
    config: Option<Config>,
    graph: Graph,
    state: String,
    app: Weak<RefCell<App>>,
    /// The answer to the last request, until [`Mock::flush`] shows it.
    pending: Option<Describe>,
    checks: Vec<checks::Row>,
    /// The system check was asked for and not answered yet.
    checking: bool,
    /// What the window last put on the clipboard.
    copied: Option<String>,
    /// The window asked for juncd's journal.
    logs_opened: bool,
}

impl std::fmt::Debug for Mock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mock").finish_non_exhaustive()
    }
}

impl Mock {
    pub fn new(s: &Scenario) -> Self {
        Self {
            inner: Rc::new(RefCell::new(MockState {
                config: s.config.clone(),
                graph: s.graph.clone(),
                state: s.state.clone(),
                app: Weak::new(),
                pending: None,
                checks: s.checks.clone(),
                checking: false,
                copied: None,
                logs_opened: false,
            })),
        }
    }

    /// The controller to answer.
    pub fn attach(&self, app: &Rc<RefCell<App>>) {
        self.inner.borrow_mut().app = Rc::downgrade(app);
    }

    /// Shows the answer to the last request (requests come while the
    /// controller is busy, so answers wait for this or a zero timer).
    pub fn flush(&self) {
        let (app, d, checks) = {
            let mut s = self.inner.borrow_mut();
            let checks = std::mem::take(&mut s.checking).then(|| s.checks.clone());
            (s.app.upgrade(), s.pending.take(), checks)
        };
        if let (Some(a), Some(d)) = (&app, d) {
            a.borrow_mut().update(Some(d));
        }
        if let (Some(a), Some(rows)) = (&app, checks) {
            a.borrow_mut().set_checks(rows);
        }
    }

    /// What the window last copied.
    pub fn copied(&self) -> Option<String> {
        self.inner.borrow().copied.clone()
    }

    /// The window asked for juncd's journal (board 20).
    pub fn logs_opened(&self) -> bool {
        self.inner.borrow().logs_opened
    }

    /// Changes what the system check answers (a fixed system re-checked).
    pub fn set_checks(&self, rows: Vec<checks::Row>) {
        self.inner.borrow_mut().checks = rows;
    }

    /// The config as the mock holds it now.
    pub fn config(&self) -> Option<Config> {
        self.inner.borrow().config.clone()
    }
}

impl Actions for Mock {
    fn send(&self, command: Command) {
        {
            let mut guard = self.inner.borrow_mut();
            let s = &mut *guard;
            match &command {
                Command::Check => s.checking = true,
                Command::Copy(text) => s.copied = Some(text.clone()),
                Command::OpenLogs => s.logs_opened = true,
                _ => {}
            }
            if let Some(c) = s.config.as_mut() {
                Self::change(c, &command);
                s.pending = Some(settle(c, s.graph.clone(), &s.state));
            }
        }
        let m = self.clone();
        slint::Timer::single_shot(std::time::Duration::ZERO, move || m.flush());
    }
}

impl Mock {
    /// Applies a request to the mock's config, as juncd would.
    fn change(c: &mut Config, command: &Command) {
        let find = |c: &mut Config, path: &str| {
            let id = junc_ipc::junction_id(path)?;
            c.junction.iter().position(|j| j.id == id)
        };
        match command {
            Command::Enabled { path, on } => {
                if let Some(i) = find(c, path) {
                    c.junction[i].enabled = *on;
                }
            }
            Command::Volume { path, percent } => {
                if let Some(i) = find(c, path) {
                    c.junction[i].volume = *percent;
                }
            }
            Command::Delete { path } => {
                if let Some(i) = find(c, path) {
                    c.junction.remove(i);
                }
            }
            Command::Rename { path, name } => {
                if let Some(i) = find(c, path) {
                    c.junction[i].name.clone_from(name);
                }
            }
            Command::Edit { junction, edit } => {
                edits::apply(c, junction, edit);
            }
            Command::Create { name } => {
                // As juncd's CreateJunction: an id from the name, unique.
                let base = junc_core::config::slugify(name);
                let id = (1..)
                    .map(|n| {
                        if n == 1 {
                            base.clone()
                        } else {
                            format!("{base}-{n}")
                        }
                    })
                    .find(|id| c.junction.iter().all(|j| &j.id != id))
                    .unwrap_or(base);
                let mut t = toml::Table::new();
                t.insert("id".into(), id.into());
                t.insert("name".into(), name.clone().into());
                if let Ok(j) = toml::Value::Table(t).try_into() {
                    c.junction.push(j);
                }
            }
            Command::Check | Command::Copy(_) | Command::OpenLogs => {}
        }
    }
}

/// Runs the reconciler against a mock backend until nothing is left to
/// do (a few steps: nodes first, then their links), then describes it.
fn settle(config: &Config, graph: Graph, state: &str) -> Describe {
    let mut backend = MockBackend::new(graph);
    let mut r = Reconciler::new();
    let mut statuses = Vec::new();
    for _ in 0..8 {
        let step = r.on_graph(config, backend.graph());
        statuses = step.plan.status;
        if step.actions.is_empty() {
            break;
        }
        for a in &step.actions {
            let _ = backend.apply(a);
        }
    }
    let g = backend.graph();
    Describe {
        api_version: junc_ipc::API_VERSION,
        daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
        state: state.to_owned(),
        revision: 1,
        config_file: "~/.config/junc/junctions.toml".to_owned(),
        config_error: Vec::new(),
        junctions: config
            .junction
            .iter()
            .map(|j| view::junction(j, &statuses, Some(g)))
            .collect(),
        capture_apps: view::capture_apps(&statuses, Some(g)),
        defaults: DefaultsView {
            input: g.defaults.source.clone(),
            output: g.defaults.sink.clone(),
        },
        candidates: view::candidates(g),
        devices: view::devices(g),
    }
}

/// Opens scenario `s` in `window`: the controller with the mock IPC, the
/// scenario's snapshot, Show Listen and the hovered link.
pub fn open(
    s: &Scenario,
    window: &crate::ui::AppWindow,
    meters: Option<Rc<crate::meter_service::MeterService>>,
    levels: crate::meter_service::Levels,
) -> (Rc<RefCell<App>>, Mock) {
    use slint::ComponentHandle;
    let mock = Mock::new(s);
    let app = crate::app::install(window, Rc::new(mock.clone()), meters, levels);
    mock.attach(&app);
    window
        .global::<crate::ui::View>()
        .set_show_listen(!s.hide_listen);
    app.borrow_mut().update(s.describe.clone());
    if let Some(id) = &s.select {
        app.borrow_mut().select(id);
    }
    // Board 20: the check juncd's waiting state starts answers at once.
    if s.describe
        .as_ref()
        .is_some_and(|d| d.state == "waiting_for_pipewire")
    {
        mock.flush();
    }
    if s.onboarding.is_some() {
        app.borrow_mut().set_step(s.onboarding);
        // The check answers at once in a fixture.
        mock.flush();
    }
    if s.picker {
        app.borrow_mut().open_picker();
    }
    if s.listen_picker {
        app.borrow_mut().open_listen_picker();
    }
    if s.add_bus {
        app.borrow().open_add_bus();
    }
    if s.rename {
        app.borrow().start_rename();
    }
    if s.delete {
        app.borrow().start_delete();
    }
    if let Some(cursor) = s.menu {
        app.borrow().open_menu(cursor);
    }
    if s.new_cable {
        app.borrow_mut().new_cable();
    }
    if let Some(i) = s.options {
        app.borrow_mut().toggle_options(i);
    }
    (app, mock)
}

/// Puts the pointer on the scenario's hovered link, or starts the link it
/// makes, once the canvas is laid out. Returns whether it found it.
pub fn hover(s: &Scenario, app: &Rc<RefCell<App>>) -> bool {
    if let Some(spec) = &s.link {
        return link(spec, s.link_keys, app);
    }
    let Some(spec) = &s.hover else { return true };
    let Some((owner, pair)) = spec.split_once(' ') else {
        return false;
    };
    let Some((a, b)) = pair.split_once("->") else {
        return false;
    };
    let (Ok(a), Ok(b)) = (a.trim().parse::<u8>(), b.trim().parse::<u8>()) else {
        return false;
    };
    let point = {
        let a_ref = app.borrow();
        let (Some(m), Some(l)) = (a_ref.main(), a_ref.layout()) else {
            return false;
        };
        let Some(i) = m
            .links
            .iter()
            .position(|k| k.owner == owner && k.pair == [a, b])
        else {
            return false;
        };
        let Some(c) = l.curves.get(i).copied().flatten() else {
            return false;
        };
        c.at(0.55)
    };
    app.borrow_mut().hover_at(point);
    true
}

/// A link from an Input's channel to a Bus channel, being made: the
/// pointer on the target, or the target chosen with the keys.
fn link(spec: &str, keys: bool, app: &Rc<RefCell<App>>) -> bool {
    let parse = || -> Option<(End, End)> {
        let (from, bus) = spec.split_once("->")?;
        let (input, ch) = from.trim().split_once(' ')?;
        let a = app.borrow();
        let card = a.main()?.inputs.iter().position(|c| c.id == input)?;
        Some((
            End::Input {
                card,
                ch: ch.trim().parse().ok()?,
            },
            End::BusIn {
                ch: bus.trim().parse().ok()?,
            },
        ))
    };
    let Some((from, to)) = parse() else {
        return false;
    };
    let at = {
        let a = app.borrow();
        let (Some(m), Some(l)) = (a.main(), a.layout()) else {
            return false;
        };
        crate::linking::at(m, l, to)
    };
    let Some(at) = at else { return false };
    let mut a = app.borrow_mut();
    a.link_start(from, keys);
    if keys {
        a.link_target(to);
    } else {
        a.link_drag(at);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_scenario_settles_like_juncd() {
        let s = load("default").expect("scenario");
        assert_eq!((s.theme.as_str(), s.size), ("dark", (1440, 900)));
        let d = s.describe.expect("juncd up");
        let names: Vec<&str> = d.junctions.iter().map(|j| j.name.as_str()).collect();
        assert_eq!(names, ["Virtual Mic for Zoom", "Firefox → OBS"]);
        let status = |i: usize| serde_json::to_value(d.junctions[i].status).expect("json");
        assert_eq!(status(0), "active", "settled, not still being built");
        assert!(
            d.capture_apps.iter().any(|a| a == "Spotify"),
            "{:?}",
            d.capture_apps
        );
        assert!(load("no-such-scenario").is_err());
    }
}
