// SPDX-License-Identifier: GPL-3.0-or-later

//! `cargo xtask i18n extract|check|plugin|build` (docs/architecture.md §8).
//!
//! - `extract` writes `po/cli/junc.pot` from every `tr`, `trf` and `trn`
//!   call in the crates' sources (GNU xgettext ≥ 0.24 reads Rust).
//! - `check` fails on a stale template, on malformed catalogs, on catalogs
//!   that miss a message of the template, on placeholders that differ from
//!   the English, and on a wrong number of plural forms. Messages that are
//!   not translated yet never fail it.
//! - `build [DIR]` compiles `po/cli/<lang>.po` into
//!   `DIR/<lang>/LC_MESSAGES/junc.mo` (default `target/locale`), with
//!   junc-l10n's own writer, so no gettext tools are needed for it.
//! - `plugin` writes the Omarchy bar plugin's JSON catalogs from
//!   `po/plugin/<lang>.po` (see the end of this file).
//!
//! The GUI's messages are the `@tr(…)` of `crates/junc-gui/ui/` (the
//! component gallery's demo text excepted): `extract` also writes
//! `po/gui/junc-gui.pot` with its own reader of Slint string literals, and
//! `check` checks `po/gui/<lang>.po` the same way. They have no msgctxt:
//! the GUI is built without Slint's default context, so a message such as
//! "Delete" is translated once. Slint bundles them into the binary
//! (`crates/junc-gui/build.rs`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the CLI's catalogs are.
pub fn po_dir(root: &Path) -> PathBuf {
    root.join("po/cli")
}

/// Where the GUI's catalogs are.
pub fn gui_dir(root: &Path) -> PathBuf {
    root.join("po/gui")
}

/// Reads a Slint string literal at the start of `s` (after `"`): its value
/// and the rest after the closing quote.
fn slint_string(s: &str) -> Option<(String, &str)> {
    let mut out = String::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, &s[i + 1..])),
            '\\' => match chars.next()?.1 {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'u' => {
                    // \u{XXXX}
                    let rest = &s[i + 2..];
                    let end = rest.find('}')?;
                    let code = u32::from_str_radix(rest.get(1..end)?, 16).ok()?;
                    out.push(char::from_u32(code)?);
                    for _ in 0..=end {
                        chars.next();
                    }
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

/// The `@tr` messages of one Slint source: `(msgid, plural, line)`.
pub fn slint_messages(text: &str) -> Result<Vec<(String, Option<String>, usize)>, String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("@tr(") {
        let line = text[..text.len() - rest.len() + at].matches('\n').count() + 1;
        let after = rest[at + 4..].trim_start();
        let lit = after
            .strip_prefix('"')
            .ok_or_else(|| format!("line {line}: @tr without a string literal"))?;
        let (msgid, tail) =
            slint_string(lit).ok_or_else(|| format!("line {line}: unterminated string"))?;
        let tail = tail.trim_start();
        if tail.starts_with("=>") {
            return Err(format!("line {line}: @tr contexts are not used in JUNC"));
        }
        let mut plural = None;
        let mut tail = tail;
        if let Some(p) = tail.strip_prefix('|') {
            let p = p
                .trim_start()
                .strip_prefix('"')
                .ok_or_else(|| format!("line {line}: plural without a string literal"))?;
            let (text, t) =
                slint_string(p).ok_or_else(|| format!("line {line}: unterminated string"))?;
            plural = Some(text);
            tail = t;
        }
        out.push((msgid, plural, line));
        rest = tail;
    }
    Ok(out)
}

/// A PO string: quoted, escaped, on one line.
fn po_string(s: &str) -> String {
    let mut out = String::from('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The GUI's template: every `@tr` under `crates/junc-gui/ui/` but the
/// gallery, by file, each message once (with every file it is in).
pub fn extract_gui(root: &Path) -> Result<String, String> {
    let ls = Command::new("git")
        .args(["ls-files", "crates/junc-gui/ui"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    let files: Vec<String> = String::from_utf8_lossy(&ls.stdout)
        .lines()
        .filter(|f| f.ends_with(".slint") && !f.ends_with("/gallery.slint"))
        .map(str::to_owned)
        .collect();
    // msgid → (plural, files), in order of first appearance.
    let mut order: Vec<String> = Vec::new();
    let mut seen: BTreeMap<String, (Option<String>, Vec<String>)> = BTreeMap::new();
    for f in &files {
        let text = std::fs::read_to_string(root.join(f)).map_err(|e| format!("{f}: {e}"))?;
        for (id, plural, _) in slint_messages(&text).map_err(|e| format!("{f}: {e}"))? {
            let entry = seen.entry(id.clone()).or_insert_with(|| {
                order.push(id.clone());
                (plural.clone(), Vec::new())
            });
            if entry.0 != plural {
                return Err(format!("{f}: {id:?} has two plural forms"));
            }
            if !entry.1.contains(f) {
                entry.1.push(f.clone());
            }
        }
    }
    let mut out = String::from(
        "# The GUI's messages, from cargo xtask i18n extract. Do not edit.\n\
         msgid \"\"\n\
         msgstr \"\"\n\
         \"Project-Id-Version: junc-gui\\n\"\n\
         \"Content-Type: text/plain; charset=UTF-8\\n\"\n",
    );
    for id in &order {
        let (plural, files) = &seen[id];
        out.push('\n');
        out.push_str(&format!("#: {}\n", files.join(" ")));
        out.push_str(&format!("msgid {}\n", po_string(id)));
        match plural {
            Some(p) => {
                out.push_str(&format!("msgid_plural {}\n", po_string(p)));
                out.push_str("msgstr[0] \"\"\nmsgstr[1] \"\"\n");
            }
            None => out.push_str("msgstr \"\"\n"),
        }
    }
    Ok(out)
}

/// One message of a catalog.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    pub msgid: String,
    pub plural: Option<String>,
    /// `msgstr`, or `msgstr[0..]` for plural messages.
    pub msgstr: Vec<String>,
    /// Line of the `msgid`.
    pub line: usize,
}

/// A parsed `.po` or `.pot`: the header entry and the messages.
#[derive(Debug, Default)]
pub struct Po {
    pub header: String,
    pub entries: Vec<Entry>,
}

fn unescape(s: &str, line: usize) -> Result<String, String> {
    let inner = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or_else(|| format!("line {line}: expected a quoted string"))?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            if c == '"' {
                return Err(format!("line {line}: unescaped quote"));
            }
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            other => return Err(format!("line {line}: unknown escape \\{other:?}")),
        }
    }
    Ok(out)
}

/// Parses a catalog. Obsolete (`#~`) entries and comments are skipped.
pub fn parse(text: &str) -> Result<Po, String> {
    #[derive(PartialEq)]
    enum Field {
        None,
        Id,
        Plural,
        Str(usize),
    }
    let mut po = Po::default();
    let mut cur = Entry::default();
    let mut field = Field::None;
    let mut started = false;
    let finish = |cur: &mut Entry, po: &mut Po, started: &mut bool| -> Result<(), String> {
        if !*started {
            return Ok(());
        }
        let e = std::mem::take(cur);
        if e.msgstr.is_empty() {
            return Err(format!("line {}: message without msgstr", e.line));
        }
        if e.msgid.is_empty() {
            po.header = e.msgstr.concat();
        } else {
            po.entries.push(e);
        }
        *started = false;
        Ok(())
    };
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let l = raw.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if let Some(rest) = l.strip_prefix("msgid_plural ") {
            cur.plural = Some(unescape(rest.trim(), n)?);
            field = Field::Plural;
        } else if let Some(rest) = l.strip_prefix("msgid ") {
            finish(&mut cur, &mut po, &mut started)?;
            started = true;
            cur.line = n;
            cur.msgid = unescape(rest.trim(), n)?;
            field = Field::Id;
        } else if let Some(rest) = l.strip_prefix("msgstr[") {
            let (idx, s) = rest
                .split_once(']')
                .ok_or_else(|| format!("line {n}: bad msgstr[]"))?;
            let idx: usize = idx.parse().map_err(|_| format!("line {n}: bad index"))?;
            if idx != cur.msgstr.len() {
                return Err(format!("line {n}: msgstr[{idx}] out of order"));
            }
            cur.msgstr.push(unescape(s.trim(), n)?);
            field = Field::Str(idx);
        } else if let Some(rest) = l.strip_prefix("msgstr ") {
            if cur.plural.is_some() {
                return Err(format!("line {n}: plural message needs msgstr[N]"));
            }
            cur.msgstr.push(unescape(rest.trim(), n)?);
            field = Field::Str(0);
        } else if l.starts_with('"') {
            let s = unescape(l, n)?;
            match field {
                Field::Id => cur.msgid.push_str(&s),
                Field::Plural => {
                    if let Some(p) = &mut cur.plural {
                        p.push_str(&s);
                    }
                }
                Field::Str(i) => cur.msgstr[i].push_str(&s),
                Field::None => return Err(format!("line {n}: string outside a message")),
            }
        } else if l.starts_with("msgctxt") {
            return Err(format!("line {n}: contexts are not used by JUNC"));
        } else {
            return Err(format!("line {n}: cannot read {l:?}"));
        }
    }
    finish(&mut cur, &mut po, &mut started)?;
    Ok(po)
}

/// `{name}` placeholders, as a set.
fn placeholders(s: &str) -> BTreeSet<&str> {
    let mut out = BTreeSet::new();
    let mut rest = s;
    while let Some(i) = rest.find('{') {
        let after = &rest[i + 1..];
        let Some(j) = after.find('}') else { break };
        out.insert(&after[..j]);
        rest = &after[j + 1..];
    }
    out
}

/// `nplurals` from a header.
fn nplurals(header: &str) -> Option<usize> {
    let line = header
        .lines()
        .find_map(|l| l.strip_prefix("Plural-Forms:"))?;
    let (_, rest) = line.split_once("nplurals=")?;
    rest.split(';').next()?.trim().parse().ok()
}

/// Problems of one catalog against the template.
pub fn check_catalog(name: &str, po: &Po, template: &Po) -> Vec<String> {
    let mut out = Vec::new();
    let n = nplurals(&po.header);
    if n.is_none() {
        out.push(format!("{name}: no Plural-Forms in the header"));
    }
    if let Err(e) = po
        .header
        .lines()
        .find_map(|l| l.strip_prefix("Plural-Forms:"))
        .map_or(Ok(()), |v| junc_l10n::plural::from_header(v).map(|_| ()))
    {
        out.push(format!("{name}: {e}"));
    }
    let have: BTreeMap<&str, &Entry> = po.entries.iter().map(|e| (e.msgid.as_str(), e)).collect();
    for t in &template.entries {
        let Some(e) = have.get(t.msgid.as_str()) else {
            out.push(format!("{name}: missing {:?} (run msgmerge)", t.msgid));
            continue;
        };
        if e.plural != t.plural {
            out.push(format!(
                "{name}:{}: plural form differs from the template",
                e.line
            ));
        }
    }
    for e in &po.entries {
        if !template.entries.iter().any(|t| t.msgid == e.msgid) {
            out.push(format!(
                "{name}:{}: {:?} is not in the template",
                e.line, e.msgid
            ));
        }
        if let (Some(n), Some(_)) = (n, &e.plural)
            && e.msgstr.len() != n
        {
            out.push(format!(
                "{name}:{}: {} plural forms, the header says {n}",
                e.line,
                e.msgstr.len()
            ));
        }
        let english: BTreeSet<&str> = placeholders(&e.msgid)
            .into_iter()
            .chain(e.plural.as_deref().map(placeholders).unwrap_or_default())
            .filter(|p| *p != "n")
            .collect();
        for s in e.msgstr.iter().filter(|s| !s.is_empty()) {
            let got: BTreeSet<&str> = placeholders(s).into_iter().filter(|p| *p != "n").collect();
            if got != english {
                out.push(format!(
                    "{name}:{}: placeholders {got:?} differ from the English {english:?}",
                    e.line
                ));
            }
        }
    }
    out
}

/// The template text xgettext makes from the sources.
pub fn extract(root: &Path) -> Result<String, String> {
    let ls = Command::new("git")
        .args(["ls-files", "crates/*.rs"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    let files: Vec<String> = String::from_utf8_lossy(&ls.stdout)
        .lines()
        .filter(|f| f.contains("/src/"))
        .map(str::to_owned)
        .collect();
    let out = Command::new("xgettext")
        .args([
            "--language=Rust",
            "--from-code=UTF-8",
            "--keyword=tr",
            "--keyword=trf",
            "--keyword=trn:1,2",
            "--add-comments=TRANSLATORS",
            "--sort-by-file",
            // File names only: line numbers would make every edit a
            // template change.
            "--add-location=file",
            "--package-name=junc",
            "--msgid-bugs-address=https://github.com/ejsergeev/junc/issues",
            "-o",
            "-",
        ])
        .args(&files)
        .current_dir(root)
        .output()
        .map_err(|e| format!("xgettext: {e} (install gettext ≥ 0.24)"))?;
    if !out.status.success() {
        return Err(format!(
            "xgettext failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    // The creation date would make every extraction a change.
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.starts_with("\"POT-Creation-Date:"))
        .map(|l| format!("{l}\n"))
        .collect())
}

/// Every problem of the catalogs under `root`. Without xgettext the
/// template's freshness is not checked (CI has it).
pub fn check(root: &Path) -> Vec<String> {
    let dir = po_dir(root);
    let pot = dir.join("junc.pot");
    let Ok(text) = std::fs::read_to_string(&pot) else {
        return vec![format!(
            "{} is missing: run cargo xtask i18n extract",
            pot.display()
        )];
    };
    let mut out = Vec::new();
    match extract(root) {
        Ok(fresh) if fresh != text => {
            out.push("po/cli/junc.pot is stale: run cargo xtask i18n extract".to_owned());
        }
        Ok(_) => {}
        Err(e) if std::env::var_os("CI").is_some() => out.push(e),
        Err(_) => {}
    }
    let template = match parse(&text) {
        Ok(t) => t,
        Err(e) => return vec![format!("junc.pot: {e}")],
    };
    for (name, path) in catalogs(&dir) {
        match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| parse(&t))
        {
            Ok(po) => out.extend(check_catalog(&name, &po, &template)),
            Err(e) => out.push(format!("{name}: {e}")),
        }
    }
    out.extend(check_gui(root));
    out.extend(check_plugin(root));
    out
}

/// The GUI's template and catalogs (no external tools needed).
fn check_gui(root: &Path) -> Vec<String> {
    let dir = gui_dir(root);
    let pot = dir.join("junc-gui.pot");
    let Ok(text) = std::fs::read_to_string(&pot) else {
        return vec![format!(
            "{} is missing: run cargo xtask i18n extract",
            pot.display()
        )];
    };
    let mut out = Vec::new();
    match extract_gui(root) {
        Ok(fresh) if fresh != text => {
            out.push("po/gui/junc-gui.pot is stale: run cargo xtask i18n extract".to_owned());
        }
        Ok(_) => {}
        Err(e) => out.push(e),
    }
    let template = match parse(&text) {
        Ok(t) => t,
        Err(e) => return vec![format!("junc-gui.pot: {e}")],
    };
    for (name, path) in catalogs(&dir) {
        match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| parse(&t))
        {
            Ok(po) => out.extend(check_catalog(&format!("gui/{name}"), &po, &template)),
            Err(e) => out.push(format!("gui/{name}: {e}")),
        }
    }
    out
}

/// `(file name, path)` of every `.po` in `dir`, sorted.
fn catalogs(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .map(|d| {
            d.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "po"))
                .map(|p| {
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    (name, p)
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// Compiles every catalog into `out/<lang>/LC_MESSAGES/junc.mo`; returns
/// the languages written.
pub fn build(root: &Path, out: &Path) -> Result<Vec<String>, String> {
    let mut langs = Vec::new();
    for (name, path) in catalogs(&po_dir(root)) {
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{name}: {e}"))?;
        let po = parse(&text).map_err(|e| format!("{name}: {e}"))?;
        let mut entries = vec![(String::new(), po.header.clone())];
        for e in &po.entries {
            if e.msgstr.iter().all(String::is_empty) {
                continue;
            }
            let id = match &e.plural {
                Some(p) => format!("{}\0{p}", e.msgid),
                None => e.msgid.clone(),
            };
            entries.push((id, e.msgstr.join("\0")));
        }
        let lang = name.trim_end_matches(".po").to_owned();
        let dir = out.join(&lang).join("LC_MESSAGES");
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        std::fs::write(dir.join("junc.mo"), junc_l10n::mo::write(&entries))
            .map_err(|e| format!("{lang}: {e}"))?;
        langs.push(lang);
    }
    Ok(langs)
}

// The Omarchy bar plugin (B23). Omarchy plugins read flat JSON catalogs,
// one per Omarchy language code, keyed by the plugin's own keys
// (`popup.on`), with plural entries as objects keyed by CLDR category. The
// English catalog `omarchy/plugin/i18n/en.json` is the source: `extract`
// writes `po/plugin/junc-plugin.pot` from it (msgid the English, a plural
// entry's `one`/`other` as msgid/msgid_plural), translators work on
// `po/plugin/<lang>.po` like the CLI's and the GUI's, and `plugin` writes
// `omarchy/plugin/i18n/<lang>.json` back from them. `check` fails when the
// template or a JSON catalog is stale.

/// Where the plugin's `.po` files are.
pub fn plugin_po_dir(root: &Path) -> PathBuf {
    root.join("po/plugin")
}

/// Where the plugin reads its JSON catalogs.
pub fn plugin_json_dir(root: &Path) -> PathBuf {
    root.join("omarchy/plugin/i18n")
}

/// The English catalog: key → text or plural forms.
fn plugin_english(root: &Path) -> Result<BTreeMap<String, serde_json::Value>, String> {
    let path = plugin_json_dir(root).join("en.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let map: BTreeMap<String, serde_json::Value> =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut seen = BTreeSet::new();
    for (key, value) in &map {
        let id = match value {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Object(o) => match (o.get("one"), o.get("other")) {
                (Some(serde_json::Value::String(one)), Some(serde_json::Value::String(_))) => {
                    one.clone()
                }
                _ => return Err(format!("en.json: {key} needs one and other")),
            },
            _ => return Err(format!("en.json: {key} is not text")),
        };
        if !seen.insert(id.clone()) {
            return Err(format!(
                "en.json: {id:?} is there twice; a .po can tell them apart only by text"
            ));
        }
    }
    Ok(map)
}

/// The plugin's template.
pub fn extract_plugin(root: &Path) -> Result<String, String> {
    let mut out = String::from(
        "# The Omarchy bar plugin's messages, from cargo xtask i18n extract\n\
         # (omarchy/plugin/i18n/en.json). Do not edit.\n\
         msgid \"\"\n\
         msgstr \"\"\n\
         \"Project-Id-Version: junc-plugin\\n\"\n\
         \"Content-Type: text/plain; charset=UTF-8\\n\"\n",
    );
    for (key, value) in plugin_english(root)? {
        out.push_str(&format!("\n#. {key}\n#: omarchy/plugin/i18n/en.json\n"));
        match value {
            serde_json::Value::Object(o) => {
                let s = |k: &str| {
                    o.get(k)
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned()
                };
                out.push_str(&format!("msgid {}\n", po_string(&s("one"))));
                out.push_str(&format!("msgid_plural {}\n", po_string(&s("other"))));
                out.push_str("msgstr[0] \"\"\nmsgstr[1] \"\"\n");
            }
            v => {
                out.push_str(&format!(
                    "msgid {}\n",
                    po_string(v.as_str().unwrap_or_default())
                ));
                out.push_str("msgstr \"\"\n");
            }
        }
    }
    Ok(out)
}

/// CLDR plural category of each gettext plural form, by language: the
/// form order of the Plural-Forms each language uses. A language with
/// plural messages has to be listed here before its catalog builds.
fn cldr_forms(lang: &str) -> Option<&'static [&'static str]> {
    Some(match lang.split('-').next().unwrap_or(lang) {
        "ja" | "ko" | "zh" | "th" | "vi" | "id" | "ms" => &["other"],
        "ru" | "uk" | "pl" | "hr" | "sr" => &["one", "few", "many"],
        "cs" | "sk" => &["one", "few", "other"],
        "de" | "en" | "nl" | "sv" | "da" | "no" | "nb" | "fi" | "et" | "it" | "es" | "pt"
        | "ca" | "el" | "bg" | "hu" | "tr" | "fr" | "hi" | "bn" | "fa" | "is" => &["one", "other"],
        _ => return None,
    })
}

/// `(language, JSON text)` for every `po/plugin/<lang>.po`. Untranslated
/// messages are left out, so they fall back to English.
pub fn build_plugin(root: &Path) -> Result<Vec<(String, String)>, String> {
    let english = plugin_english(root)?;
    let key_of: BTreeMap<String, &str> = english
        .iter()
        .map(|(k, v)| {
            let id = v
                .as_str()
                .or_else(|| v.get("one").and_then(|x| x.as_str()))
                .unwrap_or_default();
            (id.to_owned(), k.as_str())
        })
        .collect();
    let mut out = Vec::new();
    for (name, path) in catalogs(&plugin_po_dir(root)) {
        let lang = name.trim_end_matches(".po").to_owned();
        let po = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| parse(&t))
            .map_err(|e| format!("plugin/{name}: {e}"))?;
        let mut json = BTreeMap::new();
        for e in &po.entries {
            let Some(key) = key_of.get(&e.msgid) else {
                return Err(format!("plugin/{name}: {:?} is not in en.json", e.msgid));
            };
            if e.msgstr.iter().all(String::is_empty) {
                continue;
            }
            let value = if e.plural.is_some() {
                let forms = cldr_forms(&lang).ok_or_else(|| {
                    format!("plugin/{name}: add {lang}'s plural categories to cldr_forms")
                })?;
                if forms.len() != e.msgstr.len() {
                    return Err(format!(
                        "plugin/{name}:{}: {} forms, {lang} has {}",
                        e.line,
                        e.msgstr.len(),
                        forms.len()
                    ));
                }
                serde_json::Value::Object(
                    forms
                        .iter()
                        .zip(&e.msgstr)
                        .map(|(c, s)| ((*c).to_owned(), serde_json::Value::String(s.clone())))
                        .collect(),
                )
            } else {
                serde_json::Value::String(e.msgstr[0].clone())
            };
            json.insert((*key).to_owned(), value);
        }
        let text = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())? + "\n";
        out.push((lang, text));
    }
    Ok(out)
}

/// The plugin's template, catalogs and JSON files.
fn check_plugin(root: &Path) -> Vec<String> {
    let dir = plugin_po_dir(root);
    let pot = dir.join("junc-plugin.pot");
    let Ok(text) = std::fs::read_to_string(&pot) else {
        return vec![format!(
            "{} is missing: run cargo xtask i18n extract",
            pot.display()
        )];
    };
    let mut out = Vec::new();
    match extract_plugin(root) {
        Ok(fresh) if fresh != text => {
            out.push("po/plugin/junc-plugin.pot is stale: run cargo xtask i18n extract".to_owned());
        }
        Ok(_) => {}
        Err(e) => out.push(e),
    }
    let template = match parse(&text) {
        Ok(t) => t,
        Err(e) => return vec![format!("junc-plugin.pot: {e}")],
    };
    for (name, path) in catalogs(&dir) {
        match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| parse(&t))
        {
            Ok(po) => out.extend(check_catalog(&format!("plugin/{name}"), &po, &template)),
            Err(e) => out.push(format!("plugin/{name}: {e}")),
        }
    }
    match build_plugin(root) {
        Ok(files) => {
            for (lang, json) in files {
                let path = plugin_json_dir(root).join(format!("{lang}.json"));
                if std::fs::read_to_string(&path).ok().as_deref() != Some(json.as_str()) {
                    out.push(format!(
                        "omarchy/plugin/i18n/{lang}.json is stale: run cargo xtask i18n plugin"
                    ));
                }
            }
        }
        Err(e) => out.push(e),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const POT: &str = r#"msgid ""
msgstr ""
"Content-Type: text/plain; charset=UTF-8\n"

#: a.rs:1
msgid "Turned {name} off."
msgstr ""

msgid "{n} input"
msgid_plural "{n} inputs"
msgstr[0] ""
msgstr[1] ""
"#;

    const DE: &str = r#"msgid ""
msgstr ""
"Content-Type: text/plain; charset=UTF-8\n"
"Plural-Forms: nplurals=2; plural=(n != 1);\n"

msgid "Turned {name} off."
msgstr "{name} "
"ausgeschaltet."

msgid "{n} input"
msgid_plural "{n} inputs"
msgstr[0] "{n} Eingang"
msgstr[1] "{n} Eingänge"

#~ msgid "Old"
#~ msgstr "Alt"
"#;

    #[test]
    fn slint_messages_are_found_with_plurals_and_escapes() {
        let ui = r#"Text { text: @tr("Delete “{0}”?", root.title); }
x: @tr("{n} App" | "{n} Apps" % root.apps) + @tr("Say \"hi\"\n");
icon: "\u{f0415}";"#;
        let m = slint_messages(ui).expect("parse");
        assert_eq!(m.len(), 3);
        assert_eq!(m[0], ("Delete “{0}”?".to_owned(), None, 1));
        assert_eq!(m[1].1.as_deref(), Some("{n} Apps"));
        assert_eq!(m[2].0, "Say \"hi\"\n");
        assert_eq!(po_string(&m[2].0), r#""Say \"hi\"\n""#);
        assert!(slint_messages(r#"@tr("ctx" => "x")"#).is_err());
    }

    #[test]
    fn catalogs_parse_with_continuations_and_plurals() {
        let po = parse(DE).expect("parse");
        assert!(po.header.contains("nplurals=2"));
        assert_eq!(po.entries.len(), 2);
        assert_eq!(po.entries[0].msgstr, ["{name} ausgeschaltet."]);
        assert_eq!(po.entries[1].plural.as_deref(), Some("{n} inputs"));
        assert_eq!(po.entries[1].msgstr.len(), 2);
        let pot = parse(POT).expect("pot");
        assert!(check_catalog("de.po", &po, &pot).is_empty());
    }

    #[test]
    fn mistakes_are_caught_but_untranslated_is_fine() {
        let pot = parse(POT).expect("pot");
        let untranslated = DE
            .replace("msgstr \"{name} \"\n\"ausgeschaltet.\"", "msgstr \"\"")
            .replace("\"{n} Eingang\"", "\"\"")
            .replace("\"{n} Eingänge\"", "\"\"");
        assert_eq!(
            check_catalog("de.po", &parse(&untranslated).expect("parse"), &pot),
            Vec::<String>::new()
        );
        let cases = [
            (DE.replace("{name} \"", "{nom} \""), "placeholders"),
            (
                DE.replace("msgstr[1] \"{n} Eingänge\"\n", ""),
                "1 plural forms",
            ),
            (
                DE.replace("\"Plural-Forms: nplurals=2; plural=(n != 1);\\n\"\n", ""),
                "no Plural-Forms",
            ),
            (
                DE.replace("msgid \"Turned {name} off.\"", "msgid \"Turned it off.\""),
                "missing",
            ),
            (
                DE.replace("plural=(n != 1)", "plural=(n = 1)"),
                "plural expression",
            ),
        ];
        for (text, want) in cases {
            let po = parse(&text).expect("parse");
            let problems = check_catalog("de.po", &po, &pot).join("\n");
            assert!(problems.contains(want), "{want}: {problems}");
        }
        for bad in [
            "msgid \"a\"\n",
            "msgid \"a\nmsgstr \"b\"\n",
            "msgid \"a\"\nmsgstr \"\\q\"\n",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} parsed");
        }
    }

    #[test]
    fn build_writes_catalogs_junc_l10n_reads() {
        let root = std::env::temp_dir().join(format!("junc-i18n-{}", std::process::id()));
        std::fs::create_dir_all(po_dir(&root)).expect("dir");
        std::fs::write(po_dir(&root).join("de.po"), DE).expect("po");
        let out = root.join("locale");
        assert_eq!(build(&root, &out).expect("build"), ["de"]);
        let bytes = std::fs::read(out.join("de/LC_MESSAGES/junc.mo")).expect("mo");
        let c = junc_l10n::mo::Catalog::parse(&bytes).expect("catalog");
        assert_eq!(c.get("Turned {name} off."), Some("{name} ausgeschaltet."));
        assert_eq!(c.nget("{n} input", 2), Some("{n} Eingänge"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
