// SPDX-License-Identifier: GPL-3.0-or-later

//! JUNC daemon: keeps Junctions alive as PipeWire nodes and links.
//!
//! Alpha shape: reads `junctions.toml` once, then reconciles the PipeWire
//! graph on every snapshot the backend reports. The main thread blocks on a
//! channel, so an idle graph costs no wakeups. Logs go to stderr (the journal
//! under systemd), one line per event, English only. What the daemon must
//! remember between runs but is not user intent (saved default devices) lives
//! in `$XDG_STATE_HOME/junc/state.toml`.
//!
//! Config watching (A9) and D-Bus (A10) come next.

#![forbid(unsafe_code)]

use junc_core::config;
use junc_core::defaults::DefaultsState;
use junc_core::model::Config;
use junc_core::reconcile::{Notice, Reconciler};
use junc_pw::{Event, PwBackend};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Instant;

fn usage() -> ExitCode {
    eprintln!("usage: juncd [--config FILE] [--state FILE] [--version]");
    ExitCode::from(2)
}

fn log(msg: &str) {
    eprintln!("juncd: {msg}");
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

fn load_config(path: &Path) -> Result<Config, ExitCode> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log(&format!(
                "no config at {}; starting with no Junctions",
                path.display()
            ));
            return Ok(Config::default());
        }
        Err(e) => {
            log(&format!("cannot read {}: {e}", path.display()));
            return Err(ExitCode::from(3));
        }
    };
    match config::load(&text, &path.display().to_string()) {
        Ok(l) => {
            for w in &l.warnings {
                log(&w.to_string());
            }
            Ok(l.config)
        }
        Err(diags) => {
            // Never reconcile against a config we could not read: an empty
            // plan would remove every Junction. Last-good handling is A9.
            for d in &diags {
                log(&d.to_string());
            }
            Err(ExitCode::from(3))
        }
    }
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

/// Writes to a temporary file and renames it, so a crash never leaves half a
/// state file.
fn save_state(path: &Path, state: &State) {
    let text = match toml::to_string_pretty(state) {
        Ok(t) => t,
        Err(e) => return log(&format!("cannot serialize state: {e}")),
    };
    let tmp = path.with_extension("toml.tmp");
    let result = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&tmp, text))
        .and_then(|()| std::fs::rename(&tmp, path));
    if let Err(e) = result {
        log(&format!("cannot save state {}: {e}", path.display()));
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

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (mut config_path, mut state_path) = (None, None);
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
            "--state" => match args.next() {
                Some(p) => state_path = Some(PathBuf::from(p)),
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
    let state_path =
        state_path.or_else(|| xdg("XDG_STATE_HOME", ".local/state", "junc/state.toml"));
    let cfg = match load_config(&config_path) {
        Ok(c) => c,
        Err(code) => return code,
    };
    log(&format!(
        "{} Junction(s) from {}",
        cfg.junction.len(),
        config_path.display()
    ));
    let mut state = state_path.as_deref().map(load_state).unwrap_or_default();

    let (tx, rx) = mpsc::channel();
    let backend = match PwBackend::spawn(move |e| {
        let _ = tx.send(e);
    }) {
        Ok(b) => b,
        Err(e) => {
            log(&format!("cannot start the PipeWire thread: {e}"));
            return ExitCode::FAILURE;
        }
    };
    let mut reconciler = Reconciler::with_defaults(state.defaults.clone());
    let started = Instant::now();
    let mut converged = false;
    for event in rx {
        match event {
            Event::Connected => log("connected to PipeWire"),
            Event::Disconnected => {
                log("lost PipeWire; waiting for it to come back");
                // Objects die with PipeWire; only the saved defaults carry over.
                reconciler = Reconciler::with_defaults(reconciler.defaults().clone());
                converged = false;
            }
            Event::ActionFailed { action, error } => {
                log(&format!("action failed: {action}: {error}"))
            }
            Event::Graph(graph) => {
                let step = reconciler.on_graph(&cfg, &graph);
                step.notices.iter().for_each(log_notice);
                if reconciler.defaults() != &state.defaults {
                    state.defaults = reconciler.defaults().clone();
                    if let Some(p) = &state_path {
                        save_state(p, &state);
                    }
                }
                if step.refresh {
                    backend.refresh();
                }
                if step.actions.is_empty() {
                    if !converged {
                        converged = true;
                        log(&format!(
                            "graph complete after {} ms",
                            started.elapsed().as_millis()
                        ));
                    }
                } else {
                    log(&format!("applying {} action(s)", step.actions.len()));
                    for a in &step.actions {
                        if matches!(a, junc_core::backend::Action::SetDefault { .. }) {
                            log(&a.to_string());
                        }
                    }
                    backend.apply(step.actions);
                }
            }
        }
    }
    ExitCode::SUCCESS
}
