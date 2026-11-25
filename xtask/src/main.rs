// SPDX-License-Identifier: GPL-3.0-or-later

//! Developer tasks, run as `cargo xtask <task>`.
//!
//! - `schema`: regenerate `schema/junc-config-v1.json` from the Rust types.
//! - `skill-ref`: regenerate `skill/junc/reference.md` from the CLI.
//! - `skill-check`: check the agent skill (commands parse, config examples
//!   validate, links resolve, reference is fresh).

#![forbid(unsafe_code)]

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
        _ => {
            eprintln!("usage: cargo xtask schema|skill-ref|skill-check");
            ExitCode::from(2)
        }
    }
}
