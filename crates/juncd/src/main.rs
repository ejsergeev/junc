// SPDX-License-Identifier: GPL-3.0-or-later

//! JUNC daemon: keeps Junctions alive as PipeWire nodes and links.
//!
//! Alpha shape (plan A7): reads `junctions.toml` once, then reconciles the
//! PipeWire graph on every snapshot the backend reports. The main thread
//! blocks on a channel, so an idle graph costs no wakeups. Logs go to stderr
//! (the journal under systemd), one line per event, English only.
//!
//! Config watching (A9), D-Bus (A10) and defaults (A8) come next.

#![forbid(unsafe_code)]

use junc_core::config;
use junc_core::model::Config;
use junc_core::reconcile::{Notice, Reconciler};
use junc_pw::{Event, PwBackend};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Instant;

fn usage() -> ExitCode {
    eprintln!("usage: juncd [--config FILE] [--version]");
    ExitCode::from(2)
}

/// `$XDG_CONFIG_HOME/junc/junctions.toml`, else `~/.config/junc/junctions.toml`.
fn default_config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("junc/junctions.toml"))
}

fn load(path: &PathBuf) -> Result<Config, ExitCode> {
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

fn log(msg: &str) {
    eprintln!("juncd: {msg}");
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut path = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--version" => {
                println!("juncd {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "--config" => match args.next() {
                Some(p) => path = Some(PathBuf::from(p)),
                None => return usage(),
            },
            _ => return usage(),
        }
    }
    let Some(path) = path.or_else(default_config_path) else {
        log("no HOME or XDG_CONFIG_HOME; pass --config");
        return ExitCode::from(2);
    };
    let cfg = match load(&path) {
        Ok(c) => c,
        Err(code) => return code,
    };
    log(&format!(
        "{} Junction(s) from {}",
        cfg.junction.len(),
        path.display()
    ));

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
    let mut reconciler = Reconciler::new();
    let started = Instant::now();
    let mut first_converged = false;
    for event in rx {
        match event {
            Event::Connected => log("connected to PipeWire"),
            Event::Disconnected => {
                log("lost PipeWire; waiting for it to come back");
                // Objects die with PipeWire; nothing JUNC knew is current.
                reconciler = Reconciler::new();
                first_converged = false;
            }
            Event::ActionFailed { action, error } => {
                log(&format!("action failed: {action}: {error}"))
            }
            Event::Graph(graph) => {
                let step = reconciler.on_graph(&cfg, &graph);
                for n in &step.notices {
                    match n {
                        Notice::LinkOverridden {
                            junction,
                            reference,
                        } => {
                            log(&format!(
                                "notice link_overridden junction={junction} ref={reference}"
                            ));
                        }
                    }
                }
                if step.actions.is_empty() {
                    if !first_converged {
                        first_converged = true;
                        log(&format!(
                            "graph complete after {} ms",
                            started.elapsed().as_millis()
                        ));
                    }
                } else {
                    log(&format!("applying {} action(s)", step.actions.len()));
                    backend.apply(step.actions);
                }
            }
        }
    }
    ExitCode::SUCCESS
}
