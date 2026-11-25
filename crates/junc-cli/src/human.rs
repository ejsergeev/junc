// SPDX-License-Identifier: GPL-3.0-or-later

//! Text for people. Every sentence goes through the l10n functions; tables
//! are aligned by character count.

use junc_ipc::ErrorBody;
use junc_ipc::view::{Describe, InputView, JunctionView, State};
use junc_l10n::{tr, trf, trn};
use std::fmt::Write;

fn state(s: State) -> String {
    match s {
        State::Active => tr("active"),
        State::Waiting => tr("waiting"),
        State::Disabled => tr("off"),
        State::Unknown => tr("unknown"),
    }
    .into_owned()
}

fn role(j: &JunctionView) -> String {
    match j.default {
        junc_core::model::DefaultRole::None => String::new(),
        junc_core::model::DefaultRole::Input => tr("default input").into_owned(),
        junc_core::model::DefaultRole::Output => tr("default output").into_owned(),
    }
}

/// Rows as aligned columns, two spaces apart.
fn table(rows: &[Vec<String>]) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let width: Vec<usize> = (0..cols)
        .map(|c| {
            rows.iter()
                .filter_map(|r| r.get(c))
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    for r in rows {
        let mut line = String::new();
        for (c, cell) in r.iter().enumerate() {
            line.push_str(cell);
            if c + 1 < r.len() {
                line.push_str(&" ".repeat(width[c] - cell.chars().count() + 2));
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// `junc list`.
pub fn list(d: &Describe) -> String {
    if d.junctions.is_empty() {
        return tr("No Junctions yet. Create one: junc create junction \"Zoom Mic\"").into_owned();
    }
    let mut rows = vec![vec![
        tr("NAME").into_owned(),
        tr("ID").into_owned(),
        tr("STATUS").into_owned(),
        tr("INPUTS").into_owned(),
        tr("DEFAULT").into_owned(),
    ]];
    for j in &d.junctions {
        let active = j
            .inputs
            .iter()
            .filter(|i| i.status == State::Active)
            .count();
        rows.push(vec![
            j.name.clone(),
            j.id.clone(),
            if j.enabled {
                state(j.status)
            } else {
                state(State::Disabled)
            },
            format!("{active}/{}", j.inputs.len()),
            role(j),
        ]);
    }
    let mut out = table(&rows);
    if !d.config_error.is_empty() {
        out.push_str(&tr(
            "The config file has errors; the last good config is in use. See: junc status",
        ));
        out.push('\n');
    }
    out
}

/// `junc status` without a name.
pub fn daemon(d: &Describe) -> String {
    let mut out = trf(
        "juncd {v}: {state}, config revision {rev}, {count}.",
        &[
            ("v", &d.daemon_version),
            ("state", &d.state),
            ("rev", &d.revision),
            (
                "count",
                &trn(
                    "{n} Junction",
                    "{n} Junctions",
                    d.junctions.len() as u64,
                    &[],
                ),
            ),
        ],
    );
    out.push('\n');
    let _ = writeln!(
        out,
        "{}",
        trf("Config: {file}", &[("file", &d.config_file)])
    );
    if !d.capture_apps.is_empty() {
        let _ = writeln!(
            out,
            "{}",
            trf("Capturing: {apps}", &[("apps", &d.capture_apps.join(", "))])
        );
    }
    for e in &d.config_error {
        let at = e
            .location
            .as_ref()
            .map(|l| format!("{}:{}:{}: ", l.file, l.line, l.column))
            .unwrap_or_default();
        let _ = writeln!(out, "{at}{} [{}]", e.message, e.code);
        if let Some(h) = &e.hint {
            let _ = writeln!(out, "  {}", trf("hint: {h}", &[("h", h)]));
        }
    }
    out
}

fn source(i: &InputView) -> String {
    let m = &i.matcher;
    let (kind, value) = if let Some(v) = m
        .binary
        .as_ref()
        .or(m.app.as_ref())
        .or(m.node.as_ref())
        .or(m.role.as_ref())
    {
        (tr("app"), v)
    } else if let Some(v) = &m.device {
        (tr("device"), v)
    } else if let Some(v) = &m.monitor {
        (tr("monitor"), v)
    } else if let Some(v) = &m.junction {
        (tr("Junction"), v)
    } else {
        return String::new();
    };
    format!("{kind} {value}")
}

fn pairs(links: &[[u8; 2]]) -> String {
    links
        .iter()
        .map(|[a, b]| format!("{a}→{b}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `junc status NAME`.
pub fn junction(j: &JunctionView) -> String {
    let mut out = String::new();
    let st = if j.enabled {
        state(j.status)
    } else {
        state(State::Disabled)
    };
    let _ = writeln!(out, "{} ({}): {st}", j.name, j.id);
    let mut facts = vec![
        trf("volume {p}%", &[("p", &j.volume)]),
        trn(
            "{n} Bus channel",
            "{n} Bus channels",
            u64::from(j.channels),
            &[],
        ),
        trf("node {node}", &[("node", &j.node)]),
    ];
    let r = role(j);
    if !r.is_empty() {
        facts.insert(0, r);
    }
    let _ = writeln!(out, "  {}", facts.join(", "));
    if let Some(to) = &j.cable_to {
        let shown = to
            .app
            .as_ref()
            .or(to.binary.as_ref())
            .or(to.node.as_ref())
            .cloned()
            .unwrap_or_default();
        let st = j.cable_target.map_or_else(String::new, state);
        let _ = writeln!(
            out,
            "  {}",
            trf("Delivers to {app}: {st}", &[("app", &shown), ("st", &st)])
        );
    }
    if !j.inputs.is_empty() {
        let _ = writeln!(out, "  {}", tr("Inputs:"));
        let rows: Vec<Vec<String>> = j
            .inputs
            .iter()
            .map(|i| {
                vec![
                    format!("    {}", i.id),
                    source(i),
                    state(i.status),
                    format!("{}%", i.volume),
                    pairs(&i.links),
                ]
            })
            .collect();
        out.push_str(&table(&rows));
    }
    if !j.listens.is_empty() {
        let _ = writeln!(out, "  {}", tr("Listen:"));
        let rows: Vec<Vec<String>> = j
            .listens
            .iter()
            .map(|l| {
                vec![
                    format!("    {}", l.id),
                    l.target.clone(),
                    state(l.status),
                    format!("{}%", l.volume),
                    pairs(&l.links),
                ]
            })
            .collect();
        out.push_str(&table(&rows));
    }
    out
}

/// An error for stderr: what, where, how to fix.
pub fn error(e: &ErrorBody) -> String {
    let mut out = format!("junc: {}\n", e.message);
    if let Some(l) = e.diagnostics.first().and_then(|d| d.location.as_ref()) {
        let _ = writeln!(
            out,
            "  {}",
            trf(
                "at {file}:{line}:{col}",
                &[("file", &l.file), ("line", &l.line), ("col", &l.column)]
            )
        );
    }
    for d in e.diagnostics.iter().skip(1) {
        let _ = writeln!(out, "  {}", d.message);
    }
    if let Some(h) = &e.hint {
        let _ = writeln!(out, "  {}", trf("hint: {h}", &[("h", h)]));
    }
    out
}

/// One diagnostic: where, what, how to fix.
pub fn diag(d: &junc_ipc::view::Diag) -> String {
    let at = d
        .location
        .as_ref()
        .map(|l| format!("{}:{}:{}: ", l.file, l.line, l.column))
        .unwrap_or_default();
    let mut out = format!("{at}{}: {} [{}]", d.severity, d.message, d.code);
    if let Some(h) = &d.hint {
        out.push_str("\n  ");
        out.push_str(&trf("hint: {h}", &[("h", h)]));
    }
    out
}

/// A plan from `apply`.
pub fn plan(p: &junc_ipc::plan::PlanView, dry_run: bool) -> String {
    let mut out = if dry_run {
        tr("Dry run; nothing was changed. Applying would:").into_owned()
    } else {
        tr("Applied:").into_owned()
    };
    for s in &p.summary {
        let _ = write!(out, "\n  • {s}");
    }
    if !p.actions.is_empty() {
        out.push('\n');
        out.push_str(&trn(
            "{n} graph action in the first step",
            "{n} graph actions in the first step",
            p.actions.len() as u64,
            &[],
        ));
        if !p.complete {
            out.push_str(&tr("; links follow once the new nodes exist"));
        }
        out.push('.');
    }
    out
}

/// `junc doctor`.
pub fn doctor(checks: &[crate::doctor::Check]) -> String {
    use crate::doctor::Status;
    let rows: Vec<Vec<String>> = checks
        .iter()
        .map(|c| {
            let mark = match c.status {
                Status::Ok => "✓",
                Status::Warn => "!",
                Status::Missing | Status::Down => "✗",
            };
            vec![format!("{mark} {}", c.id), c.detail.clone()]
        })
        .collect();
    let table = table(&rows);
    let mut out = String::new();
    for (line, c) in table.lines().zip(checks) {
        out.push_str(line);
        out.push('\n');
        if let Some(f) = &c.fix
            && c.status != Status::Ok
        {
            let _ = writeln!(out, "    {}", trf("fix: {f}", &[("f", f)]));
        }
    }
    out
}
