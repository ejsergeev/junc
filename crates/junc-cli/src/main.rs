// SPDX-License-Identifier: GPL-3.0-or-later

//! `junc`: command-line interface to the JUNC daemon (docs/architecture.md
//! §5). Every command talks to juncd over D-Bus; nothing here touches
//! PipeWire or the config file directly.
//!
//! With `--json`, every command prints exactly one document,
//! `{"junc": 1, "ok": …, "revision": …, "data" | "error": …}`, and the exit
//! code tells what kind of failure it was (§5.3).

#![forbid(unsafe_code)]

mod client;
mod doctor;
mod edit;
mod human;
mod service;
mod skill;

use clap::Parser;
use client::Client;
use edit::{End, Result, Source, usage};
use junc_cli::args::{
    BusCmd, Cli, Cmd, ConfigCmd, Create, InputCmd, ListenCmd, Role, ServiceCmd, SkillCmd,
    SourceArgs,
};
use junc_ipc::ErrorBody;
use junc_l10n::{tr, trf};
use serde_json::{Value, json};
use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

fn to_source(a: SourceArgs) -> Source {
    match a {
        SourceArgs { app: Some(a), .. } => Source::App(a),
        SourceArgs {
            device: Some(d), ..
        } => Source::Device(d),
        SourceArgs {
            monitor: Some(m), ..
        } => Source::Monitor(m),
        SourceArgs {
            junction: Some(j), ..
        } => Source::Junction(j),
        // clap requires exactly one of the four.
        _ => unreachable!("clap enforces the group"),
    }
}

/// What a successful command reports.
struct Done {
    revision: Option<u64>,
    data: Value,
    text: String,
}

fn done(revision: u64, data: Value, text: String) -> Done {
    Done {
        revision: Some(revision),
        data,
        text,
    }
}

fn percent(text: &str) -> Result<u16> {
    text.trim_end_matches('%').parse::<u16>().map_err(|_| {
        usage(
            trf("'{t}' is not a volume.", &[("t", &text)]),
            "Give a percentage, e.g. 80.",
        )
    })
}

/// Index of the Junction called `name`; its display name goes to `shown`.
fn named(cfg: &junc_core::model::Config, name: &str, shown: &mut String) -> Result<usize> {
    let i = edit::find(cfg, name)?;
    shown.clone_from(&cfg.junction[i].name);
    Ok(i)
}

/// Id and display name of the Junction called `name`.
fn resolve(c: &Client, name: &str) -> Result<(String, String)> {
    let (config, _) = c.config()?;
    let j = &config.junction[edit::find(&config, name)?];
    Ok((j.id.clone(), j.name.clone()))
}

fn confirm(question: &str) -> bool {
    eprint!("{question} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).is_ok()
        && matches!(answer.trim(), "y" | "Y" | "yes" | "Yes")
}

/// The config file juncd uses, or the default location.
fn config_path(c: Option<&Client>) -> PathBuf {
    if let Some(d) = c.and_then(|c| c.describe().ok()) {
        return PathBuf::from(d.config_file);
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_default()
        .join("junc/junctions.toml")
}

fn read_input(file: &std::path::Path) -> Result<String> {
    let mut text = String::new();
    let r = if file.as_os_str() == "-" {
        std::io::stdin().read_to_string(&mut text).map(|_| ())
    } else {
        std::fs::read_to_string(file).map(|t| text = t)
    };
    r.map(|()| text).map_err(|e| {
        ErrorBody::new(
            "E_USAGE",
            trf("cannot read {f}: {e}", &[("f", &file.display()), ("e", &e)]),
            Some("Check the path."),
        )
    })
}

fn validate(file: &std::path::Path) -> Result<Done> {
    let name = file.display().to_string();
    let text = read_input(file)?;
    match junc_core::config::load(&text, &name) {
        Ok(l) => {
            let warnings: Vec<junc_ipc::view::Diag> =
                l.warnings.iter().map(junc_ipc::view::Diag::from).collect();
            let mut out = trf("{file}: valid", &[("file", &name)]);
            for w in &warnings {
                out.push('\n');
                out.push_str(&human::diag(w));
            }
            Ok(Done {
                revision: None,
                data: json!({ "file": name, "valid": true, "warnings": warnings }),
                text: out,
            })
        }
        Err(d) => Err(ErrorBody::from_diagnostics(&d)),
    }
}

/// Commands that work without juncd.
fn offline(cmd: &Cmd) -> Option<Result<Done>> {
    Some(match cmd {
        Cmd::Schema { schema_version: 1 } => {
            let text = junc_core::schema::json_schema();
            let data = serde_json::from_str(&text).unwrap_or(Value::Null);
            Ok(Done {
                revision: None,
                data,
                text,
            })
        }
        Cmd::Schema { schema_version } => Err(usage(
            trf(
                "There is no config schema version {v}.",
                &[("v", &schema_version)],
            ),
            "This build knows schema version 1.",
        )),
        Cmd::Config(ConfigCmd::Validate { file: Some(f) }) => validate(f),
        Cmd::Service(sub) => {
            if let ServiceCmd::Enable | ServiceCmd::Disable = sub
                && let Err(e) = service::set(matches!(sub, ServiceCmd::Enable))
            {
                return Some(Err(e));
            }
            let client = Client::connect().ok();
            match service::status(client.as_ref()) {
                Ok(st) => {
                    let text = service::describe(&st);
                    Ok(Done {
                        revision: None,
                        data: json!(st),
                        text,
                    })
                }
                Err(e) => Err(e),
            }
        }
        Cmd::Skill(sub) => {
            let run = || -> Result<Vec<skill::Entry>> {
                let dir = skill::skill_dir()?;
                let home = std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_default();
                match sub {
                    SkillCmd::Link => skill::link(&home, &dir),
                    SkillCmd::Unlink => skill::unlink(&home, &dir),
                    SkillCmd::Status => Ok(skill::status(&home, &dir)),
                }
            };
            match run() {
                Ok(entries) => {
                    let text = human::skill(&entries);
                    Ok(Done {
                        revision: None,
                        data: json!({ "folders": entries }),
                        text,
                    })
                }
                Err(e) => Err(e),
            }
        }
        Cmd::Doctor => {
            let client = Client::connect().ok();
            let checks = doctor::checks(client.as_ref());
            let text = human::doctor(&checks);
            Ok(Done {
                revision: None,
                data: json!({ "checks": checks }),
                text,
            })
        }
        _ => return None,
    })
}

fn run(cmd: Cmd, json_mode: bool) -> Result<Done> {
    if let Some(result) = offline(&cmd) {
        return result;
    }
    if let Cmd::Config(sub) = cmd {
        // Use juncd's path when it runs; otherwise the default one.
        let client = Client::connect().ok();
        let path = config_path(client.as_ref());
        return match sub {
            ConfigCmd::Path => Ok(Done {
                revision: None,
                data: json!({ "path": path }),
                text: path.display().to_string(),
            }),
            ConfigCmd::Validate { .. } => validate(&path),
            ConfigCmd::Edit => {
                let editor = std::env::var("VISUAL")
                    .or_else(|_| std::env::var("EDITOR"))
                    .unwrap_or_else(|_| "nano".into());
                let status = std::process::Command::new(&editor).arg(&path).status();
                if !status.is_ok_and(|s| s.success()) {
                    return Err(ErrorBody::new(
                        "E_EDITOR",
                        trf("{editor} did not finish cleanly", &[("editor", &editor)]),
                        Some("Set $EDITOR to your editor."),
                    ));
                }
                validate(&path)
            }
        };
    }
    let c = Client::connect()?;
    match cmd {
        Cmd::Describe => {
            let d = c.describe()?;
            let text = serde_json::to_string_pretty(&d).unwrap_or_default();
            Ok(done(d.revision, json!(d), text))
        }
        Cmd::Import { ids, kind } => import(&c, &ids, kind),
        Cmd::Apply {
            file,
            dry_run,
            expect_revision,
        } => {
            let text = read_input(&file)?;
            if dry_run {
                let (_, rev) = c.get_config()?;
                let plan = c.plan(&text)?;
                let out = human::plan(&plan, true);
                Ok(done(rev, json!({ "applied": false, "plan": plan }), out))
            } else {
                let (rev, plan) = c.apply(&text, expect_revision.unwrap_or(0))?;
                let plan: Value = serde_json::from_str(&plan).unwrap_or(Value::Null);
                let parsed = serde_json::from_value(plan.clone()).ok();
                let out = parsed.map_or_else(String::new, |p| human::plan(&p, false));
                Ok(done(rev, json!({ "applied": true, "plan": plan }), out))
            }
        }
        Cmd::Schema { .. } | Cmd::Doctor | Cmd::Config(_) | Cmd::Service(_) | Cmd::Skill(_) => {
            unreachable!("handled above")
        }
        Cmd::List => {
            let d = c.describe()?;
            let text = human::list(&d);
            Ok(done(d.revision, json!({ "junctions": d.junctions }), text))
        }
        Cmd::Status { name: None } => {
            let d = c.describe()?;
            let text = human::daemon(&d);
            let data = json!({
                "state": d.state, "daemon_version": d.daemon_version,
                "config_file": d.config_file, "config_error": d.config_error,
                "junctions": d.junctions.len(), "capture_apps": d.capture_apps,
                "defaults": d.defaults,
            });
            Ok(done(d.revision, data, text))
        }
        Cmd::Status { name: Some(name) } => {
            let d = c.describe()?;
            let (config, _) = c.config()?;
            let id = &config.junction[edit::find(&config, &name)?].id;
            let j = d.junctions.iter().find(|j| &j.id == id).ok_or_else(|| {
                ErrorBody::new(
                    "E_NOT_FOUND",
                    trf("No Junction called '{name}'.", &[("name", &name)]),
                    None,
                )
            })?;
            Ok(done(
                d.revision,
                json!({ "junction": j }),
                human::junction(j),
            ))
        }
        Cmd::Create(Create::Junction { name }) => {
            let (path, rev) = c.create_junction(&name)?;
            let id = junc_ipc::junction_id(&path).unwrap_or_default();
            let text = trf("Created {name} ({id}).", &[("name", &name), ("id", &id)]);
            Ok(done(rev, json!({ "id": id, "path": path }), text))
        }
        Cmd::Create(Create::Cable {
            from_app,
            to_app,
            name,
        }) => {
            let (id, rev) =
                c.edit(|cfg| Ok(edit::create_cable(cfg, &from_app, &to_app, name.as_deref())))?;
            let text = trf(
                "Created a Cable from {from} to {to} ({id}).",
                &[("from", &from_app), ("to", &to_app), ("id", &id)],
            );
            Ok(done(rev, json!({ "id": id }), text))
        }
        Cmd::Rename { name, new_name } => {
            let (id, old) = resolve(&c, &name)?;
            let rev = c.junction(&id, async |j| j.rename(&new_name).await)?;
            let text = trf(
                "Renamed {old} to {new}.",
                &[("old", &old), ("new", &new_name)],
            );
            Ok(done(rev, json!({ "id": id, "name": new_name }), text))
        }
        Cmd::Duplicate { name, new_name } => {
            let mut shown = String::new();
            let (id, rev) = c.edit(|cfg| {
                let i = named(cfg, &name, &mut shown)?;
                Ok(edit::duplicate(cfg, i, new_name.as_deref()))
            })?;
            let text = trf(
                "Created {id} as a copy of {name}.",
                &[("id", &id), ("name", &shown)],
            );
            Ok(done(rev, json!({ "id": id }), text))
        }
        Cmd::Delete { name, yes } => {
            let (id, display) = resolve(&c, &name)?;
            let interactive = std::io::stdin().is_terminal() && !json_mode;
            if !yes
                && !(interactive
                    && confirm(&trf(
                        "Delete {name}? Its routing stops.",
                        &[("name", &display)],
                    )))
            {
                return Err(ErrorBody::new(
                    "E_NEEDS_CONFIRMATION",
                    trf("Not deleting {name}.", &[("name", &display)]),
                    Some("Add --yes to delete without asking."),
                ));
            }
            let rev = c.junction(&id, async |j| j.delete(true).await)?;
            let text = trf("Deleted {name}.", &[("name", &display)]);
            Ok(done(rev, json!({ "id": id }), text))
        }
        Cmd::Enable { name } => set_enabled(&c, &name, Some(true)),
        Cmd::Disable { name } => set_enabled(&c, &name, Some(false)),
        Cmd::Toggle { name } => set_enabled(&c, &name, None),
        Cmd::Switch { name, role } => {
            let (config, _) = c.config()?;
            let j = &config.junction[edit::find(&config, &name)?];
            let role = role.map_or_else(
                || match j.default {
                    junc_core::model::DefaultRole::Output => "output",
                    _ => "input",
                },
                Role::as_str,
            );
            let (id, display) = (j.id.clone(), j.name.clone());
            c.junction(&id, async |p| p.set_enabled(true).await)?;
            let rev = c.junction(&id, async |p| p.set_default(role).await)?;
            let text = trf(
                "Switched to {name} as the default {role}.",
                &[("name", &display), ("role", &role)],
            );
            Ok(done(
                rev,
                json!({ "id": id, "enabled": true, "default": role }),
                text,
            ))
        }
        Cmd::Default { name, role, clear } => {
            let (id, display) = resolve(&c, &name)?;
            let role = if clear {
                "none"
            } else {
                role.map_or("none", Role::as_str)
            };
            let rev = c.junction(&id, async |j| j.set_default(role).await)?;
            let text = if role == "none" {
                trf(
                    "{name} is no longer a default device.",
                    &[("name", &display)],
                )
            } else {
                trf(
                    "{name} is now the default {role}.",
                    &[("name", &display), ("role", &role)],
                )
            };
            Ok(done(rev, json!({ "id": id, "default": role }), text))
        }
        Cmd::Volume { name, args, listen } => volume(&c, &name, &args, listen.as_deref()),
        Cmd::Input(cmd) => input(&c, cmd),
        Cmd::Bus(BusCmd::Add { name, mono, count }) => {
            let per = if mono { 1 } else { 2 };
            let n = count.saturating_mul(per);
            let mut shown = String::new();
            let (channels, rev) = c.edit(|cfg| {
                let i = named(cfg, &name, &mut shown)?;
                edit::bus_add(cfg, i, n);
                Ok(cfg.junction[i].bus.channels)
            })?;
            let text = trf(
                "{name} now has {n} Bus channels.",
                &[("name", &shown), ("n", &channels)],
            );
            Ok(done(rev, json!({ "channels": channels }), text))
        }
        Cmd::Bus(BusCmd::Remove { name, channel }) => {
            let mut shown = String::new();
            let (channels, rev) = c.edit(|cfg| {
                let i = named(cfg, &name, &mut shown)?;
                edit::bus_remove(cfg, i, channel)?;
                Ok(cfg.junction[i].bus.channels)
            })?;
            let text = trf(
                "{name} now has {n} Bus channels.",
                &[("name", &shown), ("n", &channels)],
            );
            Ok(done(rev, json!({ "channels": channels }), text))
        }
        Cmd::Link { name, from, to } => links(&c, &name, &from, &to, true),
        Cmd::Unlink { name, from, to } => links(&c, &name, &from, &to, false),
        Cmd::Listen(ListenCmd::Add { name, output, id }) => {
            let mut shown = String::new();
            let (id, rev) = c.edit(|cfg| {
                let i = named(cfg, &name, &mut shown)?;
                edit::listen_add(cfg, i, &output, id.as_deref())
            })?;
            let text = trf(
                "{name} now plays to {output} ({id}).",
                &[("name", &shown), ("output", &output), ("id", &id)],
            );
            Ok(done(rev, json!({ "id": id }), text))
        }
        Cmd::Listen(ListenCmd::Remove { name, id }) => {
            let mut shown = String::new();
            let ((), rev) = c.edit(|cfg| {
                let i = named(cfg, &name, &mut shown)?;
                edit::listen_remove(cfg, i, &id)
            })?;
            let text = trf(
                "Removed Listen output {id} from {name}.",
                &[("id", &id), ("name", &shown)],
            );
            Ok(done(rev, json!({ "id": id }), text))
        }
    }
}

fn set_enabled(c: &Client, name: &str, to: Option<bool>) -> Result<Done> {
    let (config, _) = c.config()?;
    let j = &config.junction[edit::find(&config, name)?];
    let on = to.unwrap_or(!j.enabled);
    let (id, display) = (j.id.clone(), j.name.clone());
    let rev = c.junction(&id, async |p| p.set_enabled(on).await)?;
    let text = if on {
        trf("Turned {name} on.", &[("name", &display)])
    } else {
        trf("Turned {name} off.", &[("name", &display)])
    };
    Ok(done(rev, json!({ "id": id, "enabled": on }), text))
}

fn volume(c: &Client, name: &str, args: &[String], listen: Option<&str>) -> Result<Done> {
    match (args, listen) {
        ([p], None) => {
            let p = percent(p)?;
            let (id, display) = resolve(c, name)?;
            let rev = c.junction(&id, async |j| j.set_volume(u32::from(p)).await)?;
            let text = trf("{name} volume {p}%.", &[("name", &display), ("p", &p)]);
            Ok(done(rev, json!({ "id": id, "volume": p }), text))
        }
        ([p], Some(l)) => {
            let p = percent(p)?;
            let mut shown = String::new();
            let ((), rev) = c.edit(|cfg| {
                let i = named(cfg, name, &mut shown)?;
                edit::listen_volume(cfg, i, l, p)
            })?;
            let text = trf("Listen output {id} volume {p}%.", &[("id", &l), ("p", &p)]);
            Ok(done(rev, json!({ "listen": l, "volume": p }), text))
        }
        ([input, p], None) => {
            let p = percent(p)?;
            let mut shown = String::new();
            let ((), rev) = c.edit(|cfg| {
                let i = named(cfg, name, &mut shown)?;
                edit::input_volume(cfg, i, input, p)
            })?;
            let text = trf("Input {id} volume {p}%.", &[("id", input), ("p", &p)]);
            Ok(done(rev, json!({ "input": input, "volume": p }), text))
        }
        _ => Err(usage(
            "Give PERCENT, INPUT PERCENT, or --listen ID PERCENT.".into(),
            "Example: junc volume zoom mic 80",
        )),
    }
}

fn input(c: &Client, cmd: InputCmd) -> Result<Done> {
    match cmd {
        InputCmd::Add {
            name,
            source,
            exclusive,
            pro_audio,
            id,
        } => {
            let source = to_source(source);
            let mut shown = String::new();
            let (id, rev) = c.edit(|cfg| {
                let i = named(cfg, &name, &mut shown)?;
                edit::input_add(cfg, i, &source, exclusive, pro_audio, id.as_deref())
            })?;
            let mut text = trf(
                "Added Input {id} to {name}.",
                &[("id", &id), ("name", &shown)],
            );
            let mut data = json!({ "id": id });
            // F18: a headset mic in A2DP switches the headset to its call
            // profile once it is recorded from.
            if let Source::Device(node) = &source
                && let Some(bt) = c.describe().ok().and_then(|d| {
                    d.candidates
                        .into_iter()
                        .find(|k| &k.node == node)
                        .and_then(|k| k.bluetooth)
                })
                && bt.switches_to_call
            {
                text.push('\n');
                text.push_str(&tr(
                    "Note: this is a Bluetooth headset mic. Using it switches the \
                     headset to its call profile, so playback drops to call quality \
                     while the mic is in use.",
                ));
                data["warning"] = json!("bluetooth_call_profile");
                data["bluetooth"] = json!(bt);
            }
            Ok(done(rev, data, text))
        }
        InputCmd::Remove { name, input } => {
            let mut shown = String::new();
            let ((), rev) = c.edit(|cfg| {
                let i = named(cfg, &name, &mut shown)?;
                edit::input_remove(cfg, i, &input)
            })?;
            let text = trf(
                "Removed Input {id} from {name}.",
                &[("id", &input), ("name", &shown)],
            );
            Ok(done(rev, json!({ "id": input }), text))
        }
        InputCmd::Enable { name, input } => input_enable(c, &name, &input, true),
        InputCmd::Disable { name, input } => input_enable(c, &name, &input, false),
    }
}

fn input_enable(c: &Client, name: &str, input: &str, on: bool) -> Result<Done> {
    let mut shown = String::new();
    let ((), rev) = c.edit(|cfg| {
        let i = named(cfg, name, &mut shown)?;
        edit::input_enable(cfg, i, input, on)
    })?;
    let text = if on {
        trf("Turned Input {id} on.", &[("id", &input)])
    } else {
        trf("Turned Input {id} off.", &[("id", &input)])
    };
    Ok(done(rev, json!({ "id": input, "enabled": on }), text))
}

fn links(c: &Client, name: &str, from: &str, to: &str, add: bool) -> Result<Done> {
    let (from_end, to_end): (End, End) = (from.parse()?, to.parse()?);
    // The mapping in use, for Inputs and Listen outputs on the default one.
    let d = c.describe()?;
    let mut shown = String::new();
    // Set when an entry on the default mapping has no mapping in use yet
    // (its source or output is missing): it now has only the links given.
    let mut unknown_mapping = false;
    let (changed, rev) = c.edit(|cfg| {
        let i = named(cfg, name, &mut shown)?;
        let view = d.junctions.iter().find(|v| v.id == cfg.junction[i].id);
        let entry = match (&from_end, &to_end) {
            (End::Entry(id, _), _) | (_, End::Entry(id, _)) => Some(id),
            _ => None,
        };
        unknown_mapping = view.zip(entry).is_some_and(|(v, id)| {
            v.inputs
                .iter()
                .map(|x| (&x.id, x.custom_links, x.links.is_empty()))
                .chain(
                    v.listens
                        .iter()
                        .map(|x| (&x.id, x.custom_links, x.links.is_empty())),
                )
                .any(|(x, custom, empty)| x == id && !custom && empty)
        });
        if add {
            edit::link(cfg, i, view, &from_end, &to_end)
        } else {
            edit::unlink(cfg, i, view, &from_end, &to_end).map(|()| true)
        }
    })?;
    let text = match (add, changed) {
        (true, true) => trf("Linked {from} → {to}.", &[("from", &from), ("to", &to)]),
        (true, false) => trf(
            "{from} → {to} was already linked.",
            &[("from", &from), ("to", &to)],
        ),
        (false, _) => trf("Unlinked {from} → {to}.", &[("from", &from), ("to", &to)]),
    };
    let text = if unknown_mapping && add {
        format!(
            "{text}\n{}",
            tr(
                "It was not connected, so its default links were unknown: it now has only the links you set."
            )
        )
    } else {
        text
    };
    Ok(done(
        rev,
        json!({ "from": from, "to": to, "linked": add }),
        text,
    ))
}

fn error_json(e: &ErrorBody) -> Value {
    let mut error = json!({ "code": e.code, "message": e.message });
    if let Some(h) = &e.hint {
        error["hint"] = json!(h);
    }
    if let Some(loc) = e.diagnostics.first().and_then(|d| d.location.as_ref()) {
        error["location"] = json!(loc);
    }
    if e.diagnostics.len() > 1 {
        error["diagnostics"] = json!(e.diagnostics);
    }
    json!({ "junc": 1, "ok": false, "error": error })
}

fn print_json(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

fn main() -> ExitCode {
    let wants_json = std::env::args().any(|a| a == "--json");
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if wants_json && e.use_stderr() => {
            let text = e.render().to_string();
            let first = text.lines().next().unwrap_or_default();
            let message = first.strip_prefix("error: ").unwrap_or(first).to_owned();
            let body = ErrorBody::new("E_USAGE", message, Some("See `junc --help`."));
            print_json(&error_json(&body));
            return ExitCode::from(2);
        }
        Err(e) => e.exit(),
    };
    let _lang = junc_l10n::resolve(&junc_l10n::Sources::from_env(cli.lang.as_deref()));
    let _ = cli.no_color; // Output has no colors yet.
    match run(cli.cmd, cli.json) {
        Ok(d) if cli.json => {
            let mut doc = json!({ "junc": 1, "ok": true, "data": d.data });
            if let Some(rev) = d.revision {
                doc["revision"] = json!(rev);
            }
            print_json(&doc);
            ExitCode::SUCCESS
        }
        Ok(d) => {
            if !cli.quiet && !d.text.is_empty() {
                println!("{}", d.text.trim_end());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if cli.json {
                print_json(&error_json(&e));
            } else {
                eprint!("{}", human::error(&e));
            }
            ExitCode::from(junc_cli::errors::exit_code(&e.code))
        }
    }
}

/// `junc import scan` and `junc import ID…` (F13).
fn import(c: &Client, ids: &[String], kind: Option<junc_cli::args::ImportAs>) -> Result<Done> {
    let found = c.scan_importable()?;
    if ids == ["scan"] {
        let text = if found.is_empty() {
            tr("No null sinks or loopbacks to import.").into_owned()
        } else {
            found
                .iter()
                .map(human::importable)
                .collect::<Vec<_>>()
                .join("\n")
        };
        return Ok(Done {
            revision: None,
            data: json!(found),
            text,
        });
    }
    let items = ids
        .iter()
        .map(|id| {
            found.iter().find(|i| &i.id == id).ok_or_else(|| {
                ErrorBody::new(
                    "E_NOT_FOUND",
                    trf("No setup '{id}' to import.", &[("id", id)]),
                    Some("List them with: junc import scan"),
                )
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let (made, rev) = c.edit(|cfg| {
        items
            .iter()
            .map(|item| {
                let as_cable = match kind {
                    Some(k) => k == junc_cli::args::ImportAs::Cable,
                    None => item.suggested == "cable",
                };
                edit::import(cfg, item, as_cable)
            })
            .collect::<Result<Vec<String>>>()
    })?;
    let text = trf(
        "Imported as {ids}. The old setups still run; remove them when the new ones work.",
        &[("ids", &made.join(", "))],
    );
    Ok(done(rev, json!({ "ids": made }), text))
}
