// SPDX-License-Identifier: GPL-3.0-or-later

//! `junc watch` (F23): a stream of events for the bar plugin, the tray and
//! scripts, one per line. It starts with the current state (the apps being
//! captured and every Junction), then prints changes:
//!
//! ```text
//! {"event":"capture","apps":["Spotify"]}
//! {"event":"junction","id":"zoom","name":"Zoom Mic","kind":"junction","enabled":true,"status":"active","default":"input","inputs":3}
//! {"event":"graph","generation":7}
//! {"event":"notice","kind":"link_overridden","body":{…}}
//! {"event":"junction","id":"old","removed":true}
//! {"event":"daemon","running":false}
//! ```

use crate::client::Client;
use crate::edit::Result;
use futures_util::StreamExt;
use futures_util::stream::{BoxStream, select_all};
use junc_core::model::DefaultRole;
use junc_ipc::view::{Describe, JunctionView};
use junc_l10n::{tr, trf};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;

/// What woke the watcher up.
enum Ev {
    Graph(u64),
    Notice(String, String),
    /// Something a fresh describe shows: captured apps, the config.
    Changed,
    /// juncd started (`true`) or went away.
    Daemon(bool),
}

/// The part of a Junction the stream reports.
#[derive(PartialEq)]
struct Seen {
    name: String,
    enabled: bool,
    status: String,
    default: DefaultRole,
    inputs: usize,
}

struct Printer {
    json: bool,
    capture: Option<Vec<String>>,
    junctions: BTreeMap<String, Seen>,
}

fn status_name(j: &JunctionView) -> String {
    serde_json::to_value(j.status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

impl Printer {
    fn emit(&self, event: &Value, text: &str) {
        let mut out = std::io::stdout().lock();
        let _ = if self.json {
            writeln!(out, "{event}")
        } else {
            writeln!(out, "{text}")
        };
        let _ = out.flush();
    }

    /// Prints what changed since the last describe.
    fn refresh(&mut self, d: &Describe) {
        if self.capture.as_ref() != Some(&d.capture_apps) {
            let text = if d.capture_apps.is_empty() {
                tr("capturing: nothing").into_owned()
            } else {
                trf("capturing: {apps}", &[("apps", &d.capture_apps.join(", "))])
            };
            self.emit(
                &json!({ "event": "capture", "apps": d.capture_apps }),
                &text,
            );
            self.capture = Some(d.capture_apps.clone());
        }
        let mut now = BTreeMap::new();
        for j in &d.junctions {
            let seen = Seen {
                name: j.name.clone(),
                enabled: j.enabled,
                status: status_name(j),
                default: j.default,
                inputs: j.inputs.len(),
            };
            if self.junctions.get(&j.id) != Some(&seen) {
                let text = format!(
                    "{} ({}): {}, {}",
                    j.name,
                    j.id,
                    if j.enabled { tr("on") } else { tr("off") },
                    seen.status
                );
                self.emit(
                    &json!({
                        "event": "junction", "id": j.id, "name": j.name, "kind": j.kind,
                        "enabled": j.enabled, "status": seen.status,
                        "default": j.default, "inputs": seen.inputs,
                    }),
                    &text,
                );
            }
            now.insert(j.id.clone(), seen);
        }
        for id in self.junctions.keys().filter(|id| !now.contains_key(*id)) {
            self.emit(
                &json!({ "event": "junction", "id": id, "removed": true }),
                &trf("{id}: removed", &[("id", id)]),
            );
        }
        self.junctions = now;
    }
}

/// Runs until interrupted or the session bus goes away.
pub fn run(c: &Client, json_mode: bool) -> Result<()> {
    let mut p = Printer {
        json: json_mode,
        capture: None,
        junctions: BTreeMap::new(),
    };
    let first = c.describe()?;
    p.refresh(&first);
    zbus::block_on(async {
        let d = c.watching().await?;
        let failed = |e: zbus::Error| junc_ipc::error_body(&e);
        let graph = d
            .receive_graph_changed()
            .await
            .map_err(failed)?
            .map(|s| Ev::Graph(s.args().map_or(0, |a| a.generation)));
        let notice = d.receive_notice().await.map_err(failed)?.map(|s| {
            s.args().map_or(Ev::Changed, |a| {
                Ev::Notice(a.kind.to_owned(), a.body.to_owned())
            })
        });
        let capture = d.receive_capture_apps_changed().await.map(|_| Ev::Changed);
        let revision = d.receive_revision_changed().await.map(|_| Ev::Changed);
        let list = d.receive_junctions_changed().await.map(|_| Ev::Changed);
        let owner = d
            .inner()
            .receive_owner_changed()
            .await
            .map_err(failed)?
            .map(|o| Ev::Daemon(o.is_some()));
        let streams: Vec<BoxStream<'_, Ev>> = vec![
            graph.boxed(),
            notice.boxed(),
            capture.boxed(),
            revision.boxed(),
            list.boxed(),
            owner.boxed(),
        ];
        let mut all = select_all(streams);
        while let Some(ev) = all.next().await {
            match ev {
                Ev::Graph(generation) => p.emit(
                    &json!({ "event": "graph", "generation": generation }),
                    &trf("graph changed ({g})", &[("g", &generation)]),
                ),
                Ev::Notice(kind, body) => {
                    let body: Value = serde_json::from_str(&body).unwrap_or(Value::String(body));
                    let text = format!("notice {kind}: {body}");
                    p.emit(
                        &json!({ "event": "notice", "kind": kind, "body": body }),
                        &text,
                    );
                    continue;
                }
                Ev::Changed => {}
                Ev::Daemon(running) => {
                    p.emit(
                        &json!({ "event": "daemon", "running": running }),
                        &if running {
                            tr("juncd is running")
                        } else {
                            tr("juncd stopped")
                        },
                    );
                    if !running {
                        continue;
                    }
                }
            }
            if let Ok(d) = c.describe_async().await {
                p.refresh(&d);
            }
        }
        Ok(())
    })
}
