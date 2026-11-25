// SPDX-License-Identifier: GPL-3.0-or-later

//! JUNC daemon: keeps Junctions alive as PipeWire nodes and links.
//!
//! The daemon reconciles the PipeWire graph with `junctions.toml` on every
//! graph snapshot and every config change. Hand edits are picked up live
//! (inotify on the config directory); an invalid edit is reported with file,
//! line and column and the last good config stays in use. Two threads block
//! on file descriptors (PipeWire, inotify) and the main thread on a channel,
//! so an idle daemon costs no wakeups. Logs go to stderr (the journal under
//! systemd), one line per event, English only.
//!
//! State that is not user intent lives in `$XDG_STATE_HOME/junc/`:
//! `state.toml` (saved default devices) and `last-good.toml`.
//!
//! The D-Bus API (docs/dbus.md) is served on the session bus; its calls
//! arrive here as [`Msg::Request`] and run on the main thread like any other
//! event. Owning the bus name also keeps a second daemon from starting.

#![forbid(unsafe_code)]

mod dbus;
mod notify;
mod upgrade;
mod watch;

use dbus::{Edit, Reply, Req, Resp, Responder, Server, Shared};
use junc_core::backend::DefaultKind;
use junc_core::config;
use junc_core::defaults::DefaultsState;
use junc_core::diag::{Diagnostic, codes};
use junc_core::graph::Graph;
use junc_core::model::{Config, DefaultRole, Junction};
use junc_core::plan::{EntityStatus, Plan};
use junc_core::reconcile::{Notice, Reconciler, Resolution};
use junc_core::store::{Reload, Store};
use junc_ipc::{ErrorBody, view};
use junc_pw::{Event, PwBackend};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, mpsc};
use std::time::Instant;

fn usage() -> ExitCode {
    eprintln!("usage: juncd [--config FILE] [--state-dir DIR] [--no-dbus] [--version]");
    ExitCode::from(2)
}

fn log(msg: &str) {
    eprintln!("juncd: {msg}");
}

/// What wakes the main loop.
enum Msg {
    Backend(Event),
    ConfigFile,
    Request(Req, Responder),
    /// The executable on disk was replaced by an upgrade (F20).
    Upgraded,
}

/// Daemon state that is not user intent.
#[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
struct State {
    #[serde(default)]
    defaults: DefaultsState,
}

fn xdg(var: &str, fallback: &str, rest: &str) -> Option<PathBuf> {
    let base = std::env::var_os(var)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback)))?;
    Some(base.join(rest))
}

/// Writes to a temporary file and renames it, so a crash never leaves half a
/// file behind.
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

fn load_state(path: &Path) -> State {
    match std::fs::read_to_string(path) {
        Ok(t) => toml::from_str(&t).unwrap_or_else(|e| {
            log(&format!(
                "ignoring unreadable state {}: {e}",
                path.display()
            ));
            State::default()
        }),
        Err(_) => State::default(),
    }
}

fn save_state(path: &Path, state: &State) {
    match toml::to_string_pretty(state) {
        Ok(t) => {
            if let Err(e) = write_atomic(path, &t) {
                log(&format!("cannot save {}: {e}", path.display()));
            }
        }
        Err(e) => log(&format!("cannot serialize state: {e}")),
    }
}

/// Opens the store from the config file, falling back to the last good
/// config when the file is invalid. Never reconciles against nothing: an
/// empty plan would remove every Junction.
fn open_store(config: &Path, last_good: &Path) -> Result<Store, ExitCode> {
    let name = config.display().to_string();
    let text = match std::fs::read_to_string(config) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log(&format!("no config at {name}; starting with no Junctions"));
            return Ok(Store::empty(&name));
        }
        Err(e) => {
            log(&format!("cannot read {name}: {e}"));
            return Err(ExitCode::from(3));
        }
    };
    match Store::open(&text, &name) {
        Ok(s) => Ok(s),
        Err(diags) => {
            diags
                .iter()
                .for_each(|d| log(&format!("config error: {d}")));
            let good = std::fs::read_to_string(last_good)
                .ok()
                .and_then(|t| Store::open(&t, &name).ok());
            match good {
                Some(mut s) => {
                    log(&format!(
                        "using the last good config from {}",
                        last_good.display()
                    ));
                    s.reload(&text); // remembers the error for status and notices
                    Ok(s)
                }
                None => {
                    log("no last good config to fall back to; fix the file and start again");
                    Err(ExitCode::from(3))
                }
            }
        }
    }
}

fn kind_name(k: DefaultKind) -> &'static str {
    match k {
        DefaultKind::Source => "input",
        DefaultKind::Sink => "output",
    }
}

/// Kind and JSON body of a notice, as sent in the `Notice` signal.
fn notice_json(n: &Notice) -> (&'static str, serde_json::Value) {
    match n {
        Notice::LinkOverridden {
            junction,
            reference,
        } => (
            "link_overridden",
            json!({ "junction": junction, "ref": reference }),
        ),
        Notice::DefaultRestored { kind, to } => (
            "default_restored",
            json!({ "kind": kind_name(*kind), "to": to }),
        ),
        Notice::DefaultTakenOver { kind, junction } => (
            "default_taken_over",
            json!({ "kind": kind_name(*kind), "junction": junction }),
        ),
        Notice::ProfileChanged {
            junction,
            input,
            profile,
        } => (
            "profile_changed",
            json!({ "junction": junction, "input": input, "profile": profile }),
        ),
    }
}

fn log_notice(n: &Notice) {
    match n {
        Notice::LinkOverridden {
            junction,
            reference,
        } => {
            log(&format!(
                "notice link_overridden junction={junction} ref={reference}"
            ));
        }
        Notice::DefaultRestored { kind, to } => log(&format!(
            "notice default_restored key={} to={}",
            kind.metadata_key(),
            to.as_deref().unwrap_or("(WirePlumber's choice)")
        )),
        Notice::DefaultTakenOver { kind, junction } => log(&format!(
            "notice default_taken_over key={} junction={junction}",
            kind.metadata_key()
        )),
        Notice::ProfileChanged {
            junction,
            input,
            profile,
        } => log(&format!(
            "notice profile_changed junction={junction} input={input} profile={}",
            profile.as_deref().unwrap_or("(card off, config kept)")
        )),
    }
}

struct Daemon {
    store: Store,
    reconciler: Reconciler,
    state: State,
    state_path: Option<PathBuf>,
    config_path: PathBuf,
    last_good: PathBuf,
    graph: Option<Graph>,
    /// Status of every config entry from the last plan.
    statuses: Vec<EntityStatus>,
    /// `starting`, `connected` or `waiting_for_pipewire`.
    phase: &'static str,
    server: Option<Server>,
    notifier: notify::Notifier,
    started: Instant,
    converged: bool,
}

fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

fn not_found(what: String) -> ErrorBody {
    ErrorBody::new(
        "E_NOT_FOUND",
        what,
        Some("List what exists with `junc describe`."),
    )
}

fn diags(d: &[Diagnostic]) -> ErrorBody {
    ErrorBody::from_diagnostics(d)
}

/// The plan from `old` to `new` as JSON (`junc_ipc::plan::PlanView`).
fn plan_json(old: &Config, new: &Config, p: &Plan, graph: Option<&Graph>) -> String {
    json(&junc_ipc::plan::view(old, new, p, graph))
}

/// What D-Bus clients see: properties and Junction views.
fn shared_of(
    phase: &str,
    store: &Store,
    statuses: &[EntityStatus],
    graph: Option<&Graph>,
) -> Shared {
    Shared {
        state: phase.to_owned(),
        revision: store.revision(),
        config_error: store.error().map_or_else(String::new, |d| {
            json(&d.iter().map(view::Diag::from).collect::<Vec<_>>())
        }),
        junctions: store
            .config()
            .junction
            .iter()
            .map(|j| view::junction(j, statuses, graph))
            .collect(),
        capture_apps: view::capture_apps(statuses, graph),
    }
}

impl Daemon {
    /// Reconciles and publishes; returns the plan that ran, if any.
    fn reconcile(&mut self, backend: &PwBackend) -> Option<Plan> {
        let graph = self.graph.as_ref()?;
        let mut step = self.reconciler.on_graph(self.store.config(), graph);
        for n in &step.notices {
            log_notice(n);
            let (kind, body) = notice_json(n);
            self.signal(|s| s.notice(kind, &body));
        }
        if self.reconciler.defaults() != &self.state.defaults {
            self.state.defaults = self.reconciler.defaults().clone();
            if let Some(p) = &self.state_path {
                save_state(p, &self.state);
            }
        }
        if step.refresh {
            backend.refresh();
        }
        let new_config = step.config.take();
        let status_changed = step.plan.status != self.statuses;
        self.statuses.clone_from(&step.plan.status);
        self.publish();
        if status_changed {
            self.signal(Server::graph_changed);
        }
        if step.actions.is_empty() {
            if !self.converged {
                self.converged = true;
                log(&format!(
                    "graph complete after {} ms",
                    self.started.elapsed().as_millis()
                ));
                self.notifier.status(&format!(
                    "{} Junction(s) routed",
                    self.store.config().junction.len()
                ));
            }
        } else {
            log(&format!("applying {} action(s)", step.actions.len()));
            for a in &step.actions {
                if matches!(
                    a,
                    junc_core::backend::Action::SetDefault { .. }
                        | junc_core::backend::Action::SetVolume { .. }
                        | junc_core::backend::Action::SetDeviceProfile { .. }
                ) {
                    log(&a.to_string());
                }
            }
            backend.apply(std::mem::take(&mut step.actions));
        }
        if let Some(config) = new_config {
            // The user picked another device profile elsewhere (F15): save
            // it like any edit, which also routes the new config.
            if let Err(e) = self.write(config, backend) {
                log(&format!("cannot save the profile the user picked: {e:?}"));
            }
        }
        Some(step.plan)
    }

    /// Runs `f` on the D-Bus server, logging failures (a client going away
    /// must never stop the daemon).
    fn signal(&mut self, f: impl FnOnce(&mut Server) -> zbus::Result<()>) {
        if let Some(server) = &mut self.server
            && let Err(e) = f(server)
        {
            log(&format!("D-Bus: {e}"));
        }
    }

    fn shared(&self) -> Shared {
        shared_of(self.phase, &self.store, &self.statuses, self.graph.as_ref())
    }

    fn publish(&mut self) {
        let shared = self.shared();
        self.signal(|s| s.publish(shared));
    }

    fn config_file_changed(&mut self, backend: &PwBackend) {
        let path = &self.config_path;
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Removing the file is not an instruction to remove every
                // Junction; deleting them goes through `junc delete`.
                return log("config file removed; keeping the current Junctions");
            }
            Err(e) => return log(&format!("cannot read {}: {e}", path.display())),
        };
        match self.store.reload(&text) {
            Reload::Unchanged => {}
            Reload::Invalid(d) => {
                for x in &d {
                    log(&format!("notice config_error {x}"));
                }
                log("keeping the last good config");
                self.notifier
                    .status("the config file has errors; using the last good config");
                let body =
                    json!({ "diagnostics": d.iter().map(view::Diag::from).collect::<Vec<_>>() });
                self.signal(|s| s.notice("config_error", &body));
                self.publish();
            }
            Reload::Changed(rev) => {
                log(&format!(
                    "config revision {rev}: {} Junction(s)",
                    self.store.config().junction.len()
                ));
                self.save_last_good();
                self.converged = false;
                self.started = Instant::now();
                self.reconcile(backend);
                self.publish();
            }
        }
    }

    fn save_last_good(&self) {
        if let Err(e) = write_atomic(&self.last_good, self.store.good_text()) {
            log(&format!("cannot save {}: {e}", self.last_good.display()));
        }
    }

    /// After the store accepted a change: write the file, then route.
    fn committed(&mut self, backend: &PwBackend) -> Result<Option<Plan>, ErrorBody> {
        let rev = self.store.revision();
        log(&format!(
            "config revision {rev} via D-Bus: {} Junction(s)",
            self.store.config().junction.len()
        ));
        write_atomic(&self.config_path, self.store.good_text()).map_err(|e| {
            ErrorBody::new(
                "E_WRITE_FAILED",
                format!("cannot write {}: {e}", self.config_path.display()),
                Some("Check that the config directory is writable."),
            )
        })?;
        self.save_last_good();
        self.converged = false;
        self.started = Instant::now();
        let plan = self.reconcile(backend);
        self.publish();
        Ok(plan)
    }

    /// Stores `config` (merged into the file, comments kept) and routes it.
    fn write(&mut self, config: Config, backend: &PwBackend) -> Reply {
        self.store.update(None, config).map_err(|d| diags(&d))?;
        self.committed(backend)?;
        Ok(Resp::Revision(self.store.revision()))
    }

    fn describe(&self) -> String {
        let file = self.config_path.display().to_string();
        let info = view::DaemonInfo {
            state: self.phase,
            revision: self.store.revision(),
            config_file: &file,
            config_error: self.store.error(),
        };
        json(&view::describe(
            info,
            self.store.config(),
            &self.statuses,
            self.graph.as_ref(),
        ))
    }

    fn handle(&mut self, req: Req, backend: &PwBackend) -> Reply {
        let file = self.config_path.display().to_string();
        match req {
            Req::Describe => Ok(Resp::Text(self.describe())),
            Req::GetConfig => Ok(Resp::Config(
                self.store.good_text().to_owned(),
                self.store.revision(),
            )),
            Req::Validate(text) => Ok(Resp::Text(json(&view::validate_text(&text, &file)))),
            Req::ScanImportable => {
                let Some(graph) = &self.graph else {
                    return Err(ErrorBody::new(
                        "E_PIPEWIRE_UNAVAILABLE",
                        "PipeWire is not running, so there is nothing to scan",
                        Some("Start PipeWire: systemctl --user start pipewire wireplumber"),
                    ));
                };
                Ok(Resp::Text(json(&junc_core::import::scan(graph))))
            }
            Req::Plan(text) => {
                let loaded = config::load(&text, &file).map_err(|d| diags(&d))?;
                let Some(graph) = &self.graph else {
                    return Err(ErrorBody::new(
                        "E_PIPEWIRE_UNAVAILABLE",
                        "PipeWire is not running, so there is no graph to plan against",
                        Some("Start PipeWire: systemctl --user start pipewire wireplumber"),
                    ));
                };
                let p = junc_core::plan::plan(&loaded.config, graph, &self.reconciler.held());
                Ok(Resp::Text(plan_json(
                    self.store.config(),
                    &loaded.config,
                    &p,
                    Some(graph),
                )))
            }
            Req::Apply { text, expected } => {
                let expected = (expected != 0).then_some(expected);
                let old = self.store.config().clone();
                self.store.replace(expected, &text).map_err(|d| diags(&d))?;
                let plan = self.committed(backend)?.unwrap_or_default();
                Ok(Resp::Applied(
                    self.store.revision(),
                    plan_json(&old, self.store.config(), &plan, self.graph.as_ref()),
                ))
            }
            Req::Create { name } => {
                let mut config = self.store.config().clone();
                let base = config::slugify(&name);
                let id = (1..)
                    .map(|n| {
                        if n == 1 {
                            base.clone()
                        } else {
                            format!("{base}-{n}")
                        }
                    })
                    .find(|id| config.junction.iter().all(|j| &j.id != id))
                    .unwrap_or(base);
                let mut table = toml::Table::new();
                table.insert("id".into(), id.clone().into());
                table.insert("name".into(), name.into());
                let j: Junction = toml::Value::Table(table)
                    .try_into()
                    .map_err(|e| ErrorBody::new(codes::INVALID_VALUE, e.to_string(), None))?;
                config.junction.push(j);
                let Resp::Revision(rev) = self.write(config, backend)? else {
                    unreachable!("write answers with a revision")
                };
                Ok(Resp::Created(junc_ipc::junction_path(&id), rev))
            }
            Req::ResolveLink {
                junction,
                reference,
                action,
            } => {
                let how = match action.as_str() {
                    "restore" => Resolution::Restore,
                    "leave" => Resolution::LeaveAsIs,
                    other => {
                        return Err(ErrorBody::new(
                            codes::INVALID_VALUE,
                            format!("unknown action '{other}'"),
                            Some("Use \"restore\" or \"leave\"."),
                        ));
                    }
                };
                if how == Resolution::LeaveAsIs && !self.reconciler.is_held(&junction, &reference) {
                    return Err(not_found(format!(
                        "no overridden link '{reference}' on '{junction}'"
                    )));
                }
                let config = self.store.config().clone();
                match self.reconciler.resolve(&config, &junction, &reference, how) {
                    Some(new) => self.write(new, backend),
                    // Restore, or a stream target, which is held in memory.
                    None => {
                        self.reconcile(backend);
                        Ok(Resp::Revision(self.store.revision()))
                    }
                }
            }
            Req::Edit { id, edit } => {
                let mut config = self.store.config().clone();
                let Some(pos) = config.junction.iter().position(|j| j.id == id) else {
                    return Err(not_found(format!("no Junction with id '{id}'")));
                };
                let j = &mut config.junction[pos];
                match edit {
                    Edit::Rename(name) => j.name = name,
                    Edit::Enabled(on) => j.enabled = on,
                    Edit::Volume(v) => j.volume = u16::try_from(v).unwrap_or(u16::MAX),
                    Edit::Default(role) => {
                        j.default = match role.as_str() {
                            "none" => DefaultRole::None,
                            "input" => DefaultRole::Input,
                            "output" => DefaultRole::Output,
                            other => {
                                return Err(ErrorBody::new(
                                    codes::INVALID_VALUE,
                                    format!("unknown default role '{other}'"),
                                    Some("Use \"none\", \"input\" or \"output\"."),
                                ));
                            }
                        };
                    }
                    Edit::Delete(confirm) => {
                        if !confirm {
                            return Err(ErrorBody::new(
                                "E_NEEDS_CONFIRMATION",
                                format!("deleting '{id}' removes it and its routing"),
                                Some("Call Delete(true) to confirm."),
                            ));
                        }
                        config.junction.remove(pos);
                    }
                }
                self.write(config, backend)
            }
        }
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (mut config_path, mut state_dir, mut use_dbus) = (None, None, true);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--version" => {
                println!("juncd {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "--config" => match args.next() {
                Some(p) => config_path = Some(PathBuf::from(p)),
                None => return usage(),
            },
            "--state-dir" => match args.next() {
                Some(p) => state_dir = Some(PathBuf::from(p)),
                None => return usage(),
            },
            "--no-dbus" => use_dbus = false,
            _ => return usage(),
        }
    }
    let Some(config_path) =
        config_path.or_else(|| xdg("XDG_CONFIG_HOME", ".config", "junc/junctions.toml"))
    else {
        log("no HOME or XDG_CONFIG_HOME; pass --config");
        return ExitCode::from(2);
    };
    let state_dir = state_dir.or_else(|| xdg("XDG_STATE_HOME", ".local/state", "junc"));
    let state_path = state_dir.as_ref().map(|d| d.join("state.toml"));
    let last_good = state_dir.as_ref().map_or_else(
        || PathBuf::from("/nonexistent"),
        |d| d.join("last-good.toml"),
    );

    if std::env::var_os("JUNC_REEXEC").is_some() {
        log("re-executed after an upgrade; adopting the running Junctions");
    }
    let store = match open_store(&config_path, &last_good) {
        Ok(s) => s,
        Err(code) => return code,
    };
    if store.error().is_none()
        && let Err(e) = write_atomic(&last_good, store.good_text())
    {
        log(&format!("cannot save {}: {e}", last_good.display()));
    }
    log(&format!(
        "{} Junction(s) from {}",
        store.config().junction.len(),
        config_path.display()
    ));
    let state = state_path.as_deref().map(load_state).unwrap_or_default();

    let (tx, rx) = mpsc::channel();
    // The bus name first: a second daemon stops here, before it touches
    // the graph.
    let server = if use_dbus {
        let tx = tx.clone();
        let dispatch: dbus::Dispatch = Arc::new(move |req, reply| {
            // If the main loop is gone, dropping `reply` fails the call.
            let _ = tx.send(Msg::Request(req, reply));
        });
        let initial = shared_of("starting", &store, &[], None);
        match Server::start(dispatch, initial) {
            Ok(s) => Some(s),
            Err(e) => {
                log(&format!(
                    "cannot serve {} on the session bus: {e}",
                    junc_ipc::BUS_NAME
                ));
                return ExitCode::from(4);
            }
        }
    } else {
        None
    };
    let backend = {
        let tx = tx.clone();
        match PwBackend::spawn(move |e| {
            let _ = tx.send(Msg::Backend(e));
        }) {
            Ok(b) => b,
            Err(e) => {
                log(&format!("cannot start the PipeWire thread: {e}"));
                return ExitCode::FAILURE;
            }
        }
    };
    let exe = std::env::current_exe().ok();
    if let Some(exe) = &exe {
        let tx = tx.clone();
        if let Err(e) = upgrade::spawn(exe, move || {
            let _ = tx.send(Msg::Upgraded);
        }) {
            log(&format!(
                "cannot watch {}: {e}; restart juncd after upgrades",
                exe.display()
            ));
        }
    }
    if let Err(e) = watch::spawn(&config_path, move || {
        let _ = tx.send(Msg::ConfigFile);
    }) {
        log(&format!(
            "cannot watch {}: {e}; hand edits need a restart",
            config_path.display()
        ));
    }

    let mut d = Daemon {
        reconciler: Reconciler::with_defaults(state.defaults.clone()),
        store,
        state,
        state_path,
        config_path,
        last_good,
        graph: None,
        statuses: Vec::new(),
        phase: "starting",
        server,
        notifier: notify::Notifier::from_env(),
        started: Instant::now(),
        converged: false,
    };
    d.publish();
    // Ready once D-Bus is served and the config is loaded: callers can talk
    // to the daemon; PipeWire may come later.
    d.notifier.ready(&format!(
        "{} Junction(s), waiting for PipeWire",
        d.store.config().junction.len()
    ));
    for msg in rx {
        match msg {
            Msg::Request(req, reply) => {
                let r = d.handle(req, &backend);
                reply.send(r);
            }
            Msg::ConfigFile => d.config_file_changed(&backend),
            Msg::Backend(Event::Connected) => {
                log("connected to PipeWire");
                // Time the rebuild from here, not from the daemon's start.
                d.started = Instant::now();
            }
            Msg::Backend(Event::Disconnected) => {
                log("lost PipeWire; waiting for it to come back");
                // Objects die with PipeWire; only the saved defaults carry over.
                d.reconciler = Reconciler::with_defaults(d.reconciler.defaults().clone());
                d.graph = None;
                d.statuses.clear();
                d.phase = "waiting_for_pipewire";
                d.converged = false;
                d.notifier.status("waiting for PipeWire");
                d.publish();
                d.signal(Server::graph_changed);
            }
            Msg::Backend(Event::ActionFailed { action, error }) => {
                log(&format!("action failed: {action}: {error}"));
            }
            Msg::Backend(Event::Graph(g)) => {
                d.graph = Some(*g);
                d.phase = "connected";
                d.reconcile(&backend);
            }
            Msg::Upgraded => {
                let Some(exe) = &exe else { continue };
                log("the juncd executable was upgraded; re-executing, audio keeps running");
                if let Some(p) = &d.state_path {
                    save_state(p, &d.state);
                }
                use std::os::unix::process::CommandExt;
                let e = std::process::Command::new(exe)
                    .args(std::env::args_os().skip(1))
                    .env("JUNC_REEXEC", "1")
                    .exec();
                log(&format!(
                    "cannot re-execute {}: {e}; keeping the old daemon",
                    exe.display()
                ));
            }
        }
    }
    ExitCode::SUCCESS
}
