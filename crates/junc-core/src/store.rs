// SPDX-License-Identifier: GPL-3.0-or-later

//! The daemon's view of `junctions.toml` (F22, docs/architecture.md §3.12):
//! the current config, a revision counter, the last good text, and writes
//! that keep the user's comments and layout.
//!
//! - A hand edit that validates becomes the config and bumps the revision.
//! - A hand edit that does not validate is reported with file, line and
//!   column; the last good config stays in use.
//! - The daemon's own writes are recognised by content and not re-read.
//! - A change based on an older revision is refused (`E_CONFLICT`), so the
//!   GUI, the CLI, agents and a text editor cannot silently overwrite each
//!   other.
//! - Writes apply only the difference between the old and the new config to
//!   the `toml_edit` document: untouched tables keep their comments,
//!   ordering and formatting. Arrays of tables are matched by `id`.

use crate::config::{self, Loaded};
use crate::diag::{Diagnostic, codes};
use crate::model::Config;
use std::hash::{DefaultHasher, Hash, Hasher};
use toml_edit::{DocumentMut, Item, Table};

/// Error code for a stale revision.
pub const CONFLICT: &str = "E_CONFIG_CONFLICT";

/// What a re-read of the file meant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reload {
    /// Same content as before (or the daemon's own write).
    Unchanged,
    /// A valid edit; the config changed to this revision.
    Changed(u64),
    /// An invalid edit; the last good config stays in use.
    Invalid(Vec<Diagnostic>),
}

/// Current config plus what is needed to edit the file safely.
#[derive(Debug)]
pub struct Store {
    file: String,
    config: Config,
    document: DocumentMut,
    good_text: String,
    last_seen: u64,
    revision: u64,
    error: Option<Vec<Diagnostic>>,
}

fn hash(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

impl Store {
    /// A store over `text`, read from `file` (used in diagnostics).
    pub fn open(text: &str, file: &str) -> Result<Self, Vec<Diagnostic>> {
        let Loaded {
            config, document, ..
        } = config::load(text, file)?;
        Ok(Self {
            file: file.to_owned(),
            config,
            document,
            good_text: text.to_owned(),
            last_seen: hash(text),
            revision: 1,
            error: None,
        })
    }

    /// An empty store (no file yet).
    pub fn empty(file: &str) -> Self {
        let text = config::to_toml(&Config::default());
        let document = text.parse().unwrap_or_default();
        Self {
            file: file.to_owned(),
            config: Config::default(),
            document,
            good_text: text.clone(),
            last_seen: hash(&text),
            revision: 1,
            error: None,
        }
    }

    /// The config in use.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Revision of the config in use; grows with every accepted change.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The last valid file text (persisted as `last-good.toml`).
    pub fn good_text(&self) -> &str {
        &self.good_text
    }

    /// Diagnostics of the newest file content if it was invalid.
    pub fn error(&self) -> Option<&[Diagnostic]> {
        self.error.as_deref()
    }

    /// Takes the file's current content into account.
    pub fn reload(&mut self, text: &str) -> Reload {
        let h = hash(text);
        if h == self.last_seen {
            return Reload::Unchanged;
        }
        self.last_seen = h;
        match config::load(text, &self.file) {
            Ok(l) => {
                self.error = None;
                if l.config == self.config {
                    // Only comments or layout changed: keep the new text.
                    self.document = l.document;
                    self.good_text = text.to_owned();
                    return Reload::Unchanged;
                }
                self.config = l.config;
                self.document = l.document;
                self.good_text = text.to_owned();
                self.revision += 1;
                Reload::Changed(self.revision)
            }
            Err(d) => {
                self.error = Some(d.clone());
                Reload::Invalid(d)
            }
        }
    }

    /// Applies an edit made through the daemon (D-Bus, CLI). Returns the new
    /// text to write and the new revision. `expected` is the revision the
    /// caller based its change on; `None` skips the check.
    pub fn update(
        &mut self,
        expected: Option<u64>,
        new: Config,
    ) -> Result<(String, u64), Vec<Diagnostic>> {
        if let Some(rev) = expected
            && rev != self.revision
        {
            return Err(vec![
                Diagnostic::error(
                    CONFLICT,
                    format!(
                        "config changed since revision {rev} (now {})",
                        self.revision
                    ),
                )
                .with_hint("Read the config again (junc describe) and redo the change."),
            ]);
        }
        let old = to_value(&self.config)?;
        let target = to_value(&new)?;
        let mut doc = self.document.clone();
        merge_table(doc.as_table_mut(), &old, &target);
        let text = doc.to_string();
        // The written text must load to exactly the requested config.
        let loaded = config::load(&text, &self.file)?;
        if loaded.config != new {
            return Err(vec![Diagnostic::error(
                codes::INVALID_VALUE,
                "internal error: the edited file does not read back as the requested config",
            )]);
        }
        self.config = loaded.config;
        self.document = loaded.document;
        self.good_text.clone_from(&text);
        self.last_seen = hash(&text);
        self.error = None;
        self.revision += 1;
        Ok((text, self.revision))
    }
}

fn to_value(c: &Config) -> Result<toml::Table, Vec<Diagnostic>> {
    toml::Table::try_from(c)
        .map_err(|e| vec![Diagnostic::error(codes::INVALID_VALUE, e.to_string())])
}

/// Applies `old → new` to `doc`, touching only keys whose value changed.
fn merge_table(doc: &mut Table, old: &toml::Table, new: &toml::Table) {
    for key in old.keys() {
        if !new.contains_key(key) {
            doc.remove(key);
        }
    }
    for (key, nv) in new {
        let ov = old.get(key);
        // Unchanged: leave it, present or (as a default) absent.
        if ov == Some(nv) {
            continue;
        }
        match (doc.get_mut(key), ov, nv) {
            (Some(item), Some(toml::Value::Table(ot)), toml::Value::Table(nt)) => {
                if let Some(t) = item.as_table_mut() {
                    merge_table(t, ot, nt);
                } else if let Some(t) = item.as_inline_table_mut() {
                    let mut tmp = t.clone().into_table();
                    merge_table(&mut tmp, ot, nt);
                    *t = tmp.into_inline_table();
                } else {
                    *item = to_item(nv);
                }
            }
            (Some(item), Some(toml::Value::Array(oa)), toml::Value::Array(na))
                if item.is_array_of_tables() && na.iter().all(toml::Value::is_table) =>
            {
                merge_tables_by_id(item, oa, na);
            }
            (Some(item), _, _) => {
                let decor = item.as_value().map(|v| v.decor().clone());
                *item = to_item(nv);
                if let (Some(d), Some(v)) = (decor, item.as_value_mut()) {
                    *v.decor_mut() = d;
                }
            }
            (None, _, toml::Value::Array(na))
                if !na.is_empty() && na.iter().all(toml::Value::is_table) =>
            {
                let mut aot = toml_edit::ArrayOfTables::new();
                for t in na {
                    if let toml::Value::Table(t) = t {
                        aot.push(to_table(t));
                    }
                }
                doc.insert(key, Item::ArrayOfTables(aot));
            }
            (None, _, _) => {
                doc.insert(key, to_item(nv));
            }
        }
    }
}

fn id_of(v: &toml::Value) -> Option<&str> {
    v.get("id").and_then(toml::Value::as_str)
}

/// Arrays of tables (`[[junction]]`, `[[junction.input]]`) matched by `id`,
/// so editing one entry leaves its neighbours' comments alone. The result
/// follows the new order.
fn merge_tables_by_id(item: &mut Item, old: &[toml::Value], new: &[toml::Value]) {
    let Some(aot) = item.as_array_of_tables() else {
        return;
    };
    let mut existing: Vec<(Option<String>, Table)> = aot
        .iter()
        .map(|t| {
            (
                t.get("id").and_then(Item::as_str).map(str::to_owned),
                t.clone(),
            )
        })
        .collect();
    let mut out = toml_edit::ArrayOfTables::new();
    for nv in new {
        let toml::Value::Table(nt) = nv else { continue };
        let id = id_of(nv);
        let pos = existing
            .iter()
            .position(|(eid, _)| eid.as_deref() == id && id.is_some());
        let ov = id.and_then(|id| old.iter().find(|o| id_of(o) == Some(id)));
        match (pos, ov) {
            (Some(p), Some(toml::Value::Table(ot))) => {
                let (_, mut t) = existing.remove(p);
                merge_table(&mut t, ot, nt);
                out.push(t);
            }
            _ => out.push(to_table(nt)),
        }
    }
    *item = Item::ArrayOfTables(out);
}

fn to_table(t: &toml::Table) -> Table {
    let text = toml::to_string(t).unwrap_or_default();
    let mut table = text
        .parse::<DocumentMut>()
        .map(|d| d.as_table().clone())
        .unwrap_or_default();
    inline_subtables(&mut table);
    // Field order as in the model (id first), not "values before tables".
    let order: Vec<&String> = t.keys().collect();
    let pos = |k: &str| {
        order
            .iter()
            .position(|o| o.as_str() == k)
            .unwrap_or(usize::MAX)
    };
    table.sort_values_by(|k1, _, k2, _| pos(k1.get()).cmp(&pos(k2.get())));
    toml_edit::TableLike::fmt(&mut table);
    table
}

/// Writes nested plain tables (`match`, `bus`, `cable`) inline, the way the
/// documented examples do; arrays of tables stay `[[…]]` sections.
fn inline_subtables(t: &mut Table) {
    for (_, item) in t.iter_mut() {
        if let Some(aot) = item.as_array_of_tables_mut() {
            for sub in aot.iter_mut() {
                inline_subtables(sub);
            }
        } else if let Item::Table(sub) = item {
            let mut sub = sub.clone();
            inline_subtables(&mut sub);
            *item = Item::Value(toml_edit::Value::InlineTable(sub.into_inline_table()));
        }
    }
}

fn to_item(v: &toml::Value) -> Item {
    match v {
        toml::Value::Table(t) => {
            // Small tables (match, bus, cable.to) read best inline.
            let mut tmp = to_table(t);
            tmp.set_implicit(false);
            Item::Value(toml_edit::Value::InlineTable(tmp.into_inline_table()))
        }
        other => {
            let text = format!("v = {other}");
            text.parse::<DocumentMut>()
                .ok()
                .and_then(|d| d.get("v").cloned())
                .unwrap_or_else(|| Item::Value(toml_edit::Value::from(other.to_string())))
        }
    }
}
