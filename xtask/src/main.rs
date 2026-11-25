// SPDX-License-Identifier: GPL-3.0-or-later

//! Developer tasks, run as `cargo xtask <task>`.
//!
//! - `schema`: regenerate `schema/junc-config-v1.json` from the Rust types.

#![forbid(unsafe_code)]

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
        _ => {
            eprintln!("usage: cargo xtask schema");
            ExitCode::from(2)
        }
    }
}
