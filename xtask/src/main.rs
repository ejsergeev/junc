// SPDX-License-Identifier: GPL-3.0-or-later

//! Developer tasks, run as `cargo xtask <task>`.

#![forbid(unsafe_code)]

fn main() {
    let task = std::env::args().nth(1).unwrap_or_default();
    eprintln!("xtask: unknown task '{task}'; none are implemented yet");
    std::process::exit(2);
}
