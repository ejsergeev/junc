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

#![forbid(unsafe_code)]

mod watch;

use junc_core::defaults::DefaultsState;
use junc_core::graph::Graph;
use junc_core::reconcile::{Notice, Reconciler};
use junc_core::store::{Reload, Store};
use junc_pw::{Event, PwBackend};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Instant;

fn usage() -> ExitCode {
    eprintln!("usage: juncd [--config FILE] [--state-dir DIR] [--version]");
    ExitCode::from(2)
}

fn log(msg: &str) {
    eprintln!("juncd: {msg}");
}

/// What wakes the main loop.
enum Msg {
    Backend(Event),
    ConfigFile,
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
    }
}

struct Daemon {
    store: Store,
    reconciler: Reconciler,
    state: State,
    state_path: Option<PathBuf>,
    last_good: PathBuf,
    graph: Option<Graph>,
    started: Instant,
    converged: bool,
}

impl Daemon {
    fn reconcile(&mut self, backend: &PwBackend) {
        let Some(graph) = &self.graph else { return };
        let step = self.reconciler.on_graph(self.store.config(), graph);
        step.notices.iter().for_each(log_notice);
        if self.reconciler.defaults() != &self.state.defaults {
            self.state.defaults = self.reconciler.defaults().clone();
            if let Some(p) = &self.state_path {
                save_state(p, &self.state);
            }
        }
        if step.refresh {
            backend.refresh();
        }
        if step.actions.is_empty() {
            if !self.converged {
                self.converged = true;
                log(&format!(
                    "graph complete after {} ms",
                    self.started.elapsed().as_millis()
                ));
            }
            return;
        }
        log(&format!("applying {} action(s)", step.actions.len()));
        for a in &step.actions {
            if matches!(a, junc_core::backend::Action::SetDefault { .. }) {
                log(&a.to_string());
            }
        }
        backend.apply(step.actions);
    }

    fn config_file_changed(&mut self, path: &Path, backend: &PwBackend) {
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
            Reload::Invalid(diags) => {
                for d in &diags {
                    log(&format!("notice config_error {d}"));
                }
                log("keeping the last good config");
            }
            Reload::Changed(rev) => {
                log(&format!(
                    "config revision {rev}: {} Junction(s)",
                    self.store.config().junction.len()
                ));
                if let Err(e) = write_atomic(&self.last_good, self.store.good_text()) {
                    log(&format!("cannot save {}: {e}", self.last_good.display()));
                }
                self.converged = false;
                self.reconcile(backend);
            }
        }
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (mut config_path, mut state_dir) = (None, None);
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
        last_good,
        graph: None,
        started: Instant::now(),
        converged: false,
    };
    for msg in rx {
        match msg {
            Msg::ConfigFile => d.config_file_changed(&config_path, &backend),
            Msg::Backend(Event::Connected) => log("connected to PipeWire"),
            Msg::Backend(Event::Disconnected) => {
                log("lost PipeWire; waiting for it to come back");
                // Objects die with PipeWire; only the saved defaults carry over.
                d.reconciler = Reconciler::with_defaults(d.reconciler.defaults().clone());
                d.graph = None;
                d.converged = false;
            }
            Msg::Backend(Event::ActionFailed { action, error }) => {
                log(&format!("action failed: {action}: {error}"))
            }
            Msg::Backend(Event::Graph(g)) => {
                d.graph = Some(*g);
                d.reconcile(&backend);
            }
        }
    }
    ExitCode::SUCCESS
}
