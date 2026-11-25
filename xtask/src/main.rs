// SPDX-License-Identifier: GPL-3.0-or-later

//! Developer tasks, run as `cargo xtask <task>`.
//!
//! - `schema`: regenerate `schema/junc-config-v1.json` from the Rust types.
//! - `skill-ref`: regenerate `skill/junc/reference.md` from the CLI.
//! - `skill-check`: check the agent skill (commands parse, config examples
//!   validate, links resolve, reference is fresh).
//! - `i18n extract|check|build [DIR]`: the message template, catalog
//!   checks, and compiled catalogs (see `i18n.rs`).

#![forbid(unsafe_code)]

mod i18n;
mod skill;

use std::path::PathBuf;
use std::process::ExitCode;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn main() -> ExitCode {
    let task = std::env::args().nth(1).unwrap_or_default();
    match task.as_str() {
        "schema" => {
            let path = repo_root().join("schema/junc-config-v1.json");
            match std::fs::write(&path, junc_core::schema::json_schema()) {
                Ok(()) => {
                    println!("wrote {}", path.display());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("xtask schema: {}: {e}", path.display());
                    ExitCode::FAILURE
                }
            }
        }
        "skill-ref" => {
            let path = skill::dir(&repo_root()).join("reference.md");
            match std::fs::write(&path, skill::reference()) {
                Ok(()) => {
                    println!("wrote {}", path.display());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("xtask skill-ref: {}: {e}", path.display());
                    ExitCode::FAILURE
                }
            }
        }
        "skill-check" => {
            let problems = skill::check(&repo_root());
            for p in &problems {
                eprintln!("{p}");
            }
            if problems.is_empty() {
                println!("skill-check: ok");
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        "i18n" => i18n_task(),
        _ => {
            eprintln!("usage: cargo xtask schema|skill-ref|skill-check|i18n");
            ExitCode::from(2)
        }
    }
}

fn i18n_task() -> ExitCode {
    let root = repo_root();
    let fail = |e: String| {
        eprintln!("xtask i18n: {e}");
        ExitCode::FAILURE
    };
    match std::env::args().nth(2).as_deref() {
        Some("extract") => {
            let path = i18n::po_dir(&root).join("junc.pot");
            match i18n::extract(&root).and_then(|t| {
                std::fs::create_dir_all(i18n::po_dir(&root))
                    .and_then(|()| std::fs::write(&path, t))
                    .map_err(|e| format!("{}: {e}", path.display()))
            }) {
                Ok(()) => {
                    println!("wrote {}", path.display());
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            }
        }
        Some("check") => {
            let problems = i18n::check(&root);
            for p in &problems {
                eprintln!("{p}");
            }
            if problems.is_empty() {
                println!("i18n check: ok");
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Some("build") => {
            let out = std::env::args()
                .nth(3)
                .map_or_else(|| root.join("target/locale"), PathBuf::from);
            match i18n::build(&root, &out) {
                Ok(langs) => {
                    println!("wrote {} ({})", out.display(), langs.join(" "));
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            }
        }
        _ => {
            eprintln!("usage: cargo xtask i18n extract|check|build [DIR]");
            ExitCode::from(2)
        }
    }
}
