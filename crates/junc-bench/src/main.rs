// SPDX-License-Identifier: GPL-3.0-or-later

//! `junc-bench`: daemon performance against the PRD targets, with a
//! regression gate (docs/architecture.md §11.4).
//!
//! ```text
//! junc-bench run [--out FILE] [--quick]       # under scripts/ci/with-pipewire.sh
//! junc-bench compare RESULTS --baseline FILE --thresholds FILE [--inject metric=factor]…
//! ```
//!
//! Results and baselines are `{"arch": "...", "metrics": {name: value}}`.

#![forbid(unsafe_code)]

mod gate;
mod latency;
mod run;

use gate::{Metrics, Thresholds};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Serialize, Deserialize)]
struct Results {
    arch: String,
    metrics: Metrics,
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: junc-bench run [--out FILE] [--quick]\n       \
         junc-bench compare RESULTS --baseline FILE --thresholds FILE [--inject metric=factor]…"
    );
    ExitCode::from(2)
}

fn read<T: for<'de> Deserialize<'de>>(path: &str, toml_file: bool) -> Result<T, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    if toml_file {
        toml::from_str(&text).map_err(|e| format!("{path}: {e}"))
    } else {
        serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))
    }
}

fn run_cmd(args: &[String]) -> Result<bool, String> {
    let mut out = None;
    let mut quick = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = it.next().cloned(),
            "--quick" => quick = true,
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    let bins = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .map(PathBuf::from)
        .ok_or("no binary directory")?;
    let opts = run::Opts {
        bins,
        idle_secs: if quick { 3 } else { 10 },
        churn: if quick { 10 } else { 30 },
        recoveries: if quick { 1 } else { 3 },
        cli_runs: 20,
        unit: PathBuf::from("packaging/systemd/juncd.service"),
    };
    let metrics = run::run(&opts)?;
    let results = Results {
        arch: std::env::consts::ARCH.to_owned(),
        metrics,
    };
    let json = serde_json::to_string_pretty(&results).map_err(|e| e.to_string())? + "\n";
    print!("{json}");
    if let Some(path) = out {
        std::fs::write(&path, &json).map_err(|e| format!("{path}: {e}"))?;
    }
    Ok(true)
}

fn compare_cmd(args: &[String]) -> Result<bool, String> {
    let mut it = args.iter();
    let results_path = it.next().ok_or("missing RESULTS")?;
    let (mut baseline, mut thresholds, mut injects) = (None, None, Vec::new());
    while let Some(a) = it.next() {
        match a.as_str() {
            "--baseline" => baseline = it.next().cloned(),
            "--thresholds" => thresholds = it.next().cloned(),
            "--inject" => injects.extend(it.next().cloned()),
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    let mut results: Results = read(results_path, false)?;
    let limits: Thresholds = read(&thresholds.ok_or("missing --thresholds")?, true)?;
    // No baseline yet (a new architecture): the PRD targets still hold.
    let baseline: Results = match baseline {
        Some(path) if std::path::Path::new(&path).exists() => read(&path, false)?,
        Some(path) => {
            println!("no baseline at {path}: checking the absolute targets only");
            Results {
                arch: results.arch.clone(),
                metrics: Metrics::new(),
            }
        }
        None => return Err("missing --baseline".into()),
    };
    if baseline.arch != results.arch {
        return Err(format!(
            "baseline is for {}, results for {}",
            baseline.arch, results.arch
        ));
    }
    for spec in &injects {
        gate::inject(&mut results.metrics, spec)?;
        println!("injected regression: {spec}");
    }
    let verdicts = gate::check(&results.metrics, &baseline.metrics, &limits);
    for v in &verdicts {
        let unit = limits
            .metrics
            .get(&v.metric)
            .map_or("", |l| l.unit.as_str());
        let show = |x: Option<f64>| x.map_or_else(|| "–".to_owned(), |x| format!("{x:.1}"));
        println!(
            "{} {:<24} {:>12} {:<5} (baseline {}){}",
            if v.ok { "ok  " } else { "FAIL" },
            v.metric,
            show(v.value),
            unit,
            show(v.baseline),
            if v.ok {
                String::new()
            } else {
                format!(": {}", v.why)
            }
        );
    }
    Ok(verdicts.iter().all(|v| v.ok))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("run") => run_cmd(&args[1..]),
        Some("compare") => compare_cmd(&args[1..]),
        _ => return usage(),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => {
            eprintln!("junc-bench: performance gate failed");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("junc-bench: {e}");
            ExitCode::from(3)
        }
    }
}
