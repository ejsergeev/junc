// SPDX-License-Identifier: GPL-3.0-or-later

//! `junc preset export|import`, `junc template list|show` and
//! `junc create junction --template` (F22, F24). Templates are built into
//! junc-core; presets are files. Adding one is an ordinary config edit.

use crate::client::Client;
use crate::edit::{self, Result, usage};
use crate::{Done, done, read_input};
use junc_core::preset::{self, BUILTIN, Fill, Preset, SlotKind};
use junc_ipc::ErrorBody;
use junc_ipc::view::Describe;
use junc_l10n::trf;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

/// A built-in template by id, or "did you mean".
fn template(id: &str) -> Result<Preset> {
    preset::builtin(id).ok_or_else(|| {
        ErrorBody::new(
            "E_NOT_FOUND",
            trf("No template '{id}'.", &[("id", &id)]),
            Some(&edit::did_you_mean(
                id,
                BUILTIN.iter().map(|(i, _)| *i),
                "See them with `junc template list`.",
            )),
        )
    })
}

fn template_json(id: &str, p: &Preset) -> Value {
    json!({
        "id": id,
        "name": p.header.name,
        "description": p.header.description,
        "example": p.header.example,
        "icon": p.header.icon,
        "kind": p.junction.kind,
        "slots": p.header.slot,
    })
}

/// `junc template list`.
pub fn list() -> Done {
    let mut items = Vec::new();
    let mut lines = Vec::new();
    for (id, _) in BUILTIN {
        let Some(p) = preset::builtin(id) else {
            continue;
        };
        let slots = p
            .header
            .slot
            .iter()
            .map(|s| format!("--slot {}=<{}>", s.id, s.kind.as_str()))
            .collect::<Vec<_>>()
            .join(" ");
        let mut line = format!(
            "{id:<16} {}: {}",
            p.header.name,
            p.header.description.as_deref().unwrap_or("")
        );
        if !slots.is_empty() {
            line.push_str(&format!("\n{:<16} {slots}", ""));
        }
        lines.push(line);
        items.push(template_json(id, &p));
    }
    Done {
        revision: None,
        data: json!({ "templates": items }),
        text: lines.join("\n"),
    }
}

/// `junc template show T`: the file as shipped.
pub fn show(id: &str) -> Result<Done> {
    let p = template(id)?;
    let text = BUILTIN
        .iter()
        .find(|(i, _)| *i == id)
        .map_or("", |(_, t)| *t);
    let mut data = template_json(id, &p);
    data["file"] = json!(text);
    Ok(Done {
        revision: None,
        data,
        text: text.trim_end().to_owned(),
    })
}

/// `ID=VALUE` pairs from `--slot`.
fn parse_slots(args: &[String]) -> Result<Vec<(String, String)>> {
    args.iter()
        .map(|a| {
            a.split_once('=')
                .filter(|(k, v)| !k.is_empty() && !v.is_empty())
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .ok_or_else(|| {
                    usage(
                        trf("'{a}' is not ID=VALUE.", &[("a", &a)]),
                        "Give each slot as --slot ID=VALUE, e.g. --slot mic=alsa_input.usb-mic.",
                    )
                })
        })
        .collect()
}

/// What to show for a slot value in the Junction's name: the device's
/// description or the app's name, when the graph has them.
fn label(d: Option<&Describe>, kind: SlotKind, value: &str) -> Option<String> {
    let c = &d?.candidates;
    match kind {
        SlotKind::App => c
            .iter()
            .find(|c| c.binary.as_deref() == Some(value))
            .and_then(|c| c.app.clone()),
        _ => c
            .iter()
            .find(|c| c.node == value)
            .map(|c| c.description.clone())
            .filter(|d| !d.is_empty()),
    }
}

/// Fills `p`'s slots from `--slot` arguments.
fn fill(p: &Preset, args: &[String], d: Option<&Describe>) -> Result<junc_core::model::Junction> {
    let fills: BTreeMap<String, Fill> = parse_slots(args)?
        .into_iter()
        .map(|(id, value)| {
            let kind = p.header.slot.iter().find(|s| s.id == id).map(|s| s.kind);
            let label = kind.and_then(|k| label(d, k, &value));
            (id, Fill { value, label })
        })
        .collect();
    p.fill(&fills)
        .map_err(|e| ErrorBody::from_diagnostics(&[e]))
}

/// Adds the filled Junction to the config.
fn add(c: &Client, p: &Preset, slots: &[String], name: Option<&str>, source: &str) -> Result<Done> {
    let d = c.describe().ok();
    let j = fill(p, slots, d.as_ref())?;
    let ((id, notes), rev) = c.edit(|cfg| Ok(edit::add_junction(cfg, j.clone(), name)))?;
    let shown = name.unwrap_or(&j.name);
    let mut text = trf(
        "Created {name} ({id}) from {source}.",
        &[("name", &shown), ("id", &id), ("source", &source)],
    );
    for n in &notes {
        text.push('\n');
        text.push_str(n);
    }
    Ok(done(
        rev,
        json!({ "id": id, "name": shown, "notes": notes }),
        text,
    ))
}

/// `junc create junction NAME --template T --slot …`.
pub fn create(c: &Client, name: &str, id: &str, slots: &[String]) -> Result<Done> {
    let p = template(id)?;
    add(c, &p, slots, Some(name), &p.header.name)
}

/// `junc preset import FILE`.
pub fn import(c: &Client, file: &Path, slots: &[String], name: Option<&str>) -> Result<Done> {
    let text = read_input(file)?;
    let shown = file.display().to_string();
    let p = preset::load(&text, &shown)
        .map_err(|d| ErrorBody::from_diagnostics(&d))?
        .preset;
    add(c, &p, slots, name, &shown)
}

/// `junc preset export NAME [-o FILE]`.
pub fn export(c: &Client, name: &str, output: Option<&Path>) -> Result<Done> {
    let (config, rev) = c.config()?;
    let j = &config.junction[edit::find(&config, name)?];
    let p = Preset::from_junction(j);
    let tied = p.device_specific();
    let mut text = String::new();
    if !tied.is_empty() {
        text.push_str(&format!(
            "# Tied to this computer's devices: {}. Elsewhere they wait for a\n# device with the same name.\n",
            tied.join(", ")
        ));
    }
    text.push_str(&p.to_toml());
    let mut data = json!({ "id": j.id, "device_specific": tied, "preset": text });
    let shown = match output {
        Some(path) => {
            std::fs::write(path, &text).map_err(|e| {
                ErrorBody::new(
                    "E_WRITE_FAILED",
                    trf(
                        "cannot write {f}: {e}",
                        &[("f", &path.display()), ("e", &e)],
                    ),
                    Some("Check the path and that you can write there."),
                )
            })?;
            data["file"] = json!(path);
            let mut msg = trf(
                "Exported {name} to {file}.",
                &[("name", &j.name), ("file", &path.display())],
            );
            if !tied.is_empty() {
                msg.push('\n');
                msg.push_str(&trf(
                    "Tied to this computer's devices: {list}.",
                    &[("list", &tied.join(", "))],
                ));
            }
            msg
        }
        None => text.trim_end().to_owned(),
    };
    Ok(done(rev, data, shown))
}
