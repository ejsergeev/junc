// SPDX-License-Identifier: GPL-3.0-or-later

//! Compiles the UI with its translations (architecture §8): the catalogs of
//! `po/gui/` and the en_XA pseudo-locale made from the template, bundled
//! into the binary in the layout Slint reads
//! (`<dir>/<lang>/LC_MESSAGES/junc-gui.po`). Messages carry no context
//! (`cargo xtask i18n extract` writes them so).

use std::path::Path;

/// A PO string's text (the template writes each on one line).
fn unquote(s: &str) -> String {
    let inner = s.trim().trim_start_matches('"').trim_end_matches('"');
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn quote(s: &str) -> String {
    let mut out = String::from('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// en_XA: every message of the template through `junc_l10n::pseudo`.
fn pseudo(pot: &str) -> String {
    let mut out = String::from(
        "msgid \"\"\nmsgstr \"\"\n\"Content-Type: text/plain; charset=UTF-8\\n\"\n\
         \"Plural-Forms: nplurals=2; plural=(n != 1);\\n\"\n",
    );
    let mut msgid: Option<String> = None;
    for line in pot.lines() {
        if let Some(v) = line.strip_prefix("msgid ") {
            msgid = Some(unquote(v)).filter(|m| !m.is_empty());
        } else if let (Some(v), Some(id)) = (line.strip_prefix("msgid_plural "), &msgid) {
            let plural = unquote(v);
            out.push_str(&format!(
                "\nmsgid {}\nmsgid_plural {}\nmsgstr[0] {}\nmsgstr[1] {}\n",
                quote(id),
                quote(&plural),
                quote(&junc_l10n::pseudo(id)),
                quote(&junc_l10n::pseudo(&plural))
            ));
            msgid = None;
        } else if line.starts_with("msgstr")
            && let Some(id) = msgid.take()
        {
            out.push_str(&format!(
                "\nmsgid {}\nmsgstr {}\n",
                quote(&id),
                quote(&junc_l10n::pseudo(&id))
            ));
        }
    }
    out
}

fn bundle(po: &Path, out: &Path) -> std::io::Result<()> {
    let write = |lang: &str, text: &str| -> std::io::Result<()> {
        let dir = out.join(lang).join("LC_MESSAGES");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("junc-gui.po"), text)
    };
    for entry in std::fs::read_dir(po)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "po")
            && let Some(lang) = path.file_stem().and_then(|s| s.to_str())
        {
            write(lang, &std::fs::read_to_string(&path)?)?;
        }
    }
    write(
        junc_l10n::registry::PSEUDO,
        &pseudo(&std::fs::read_to_string(po.join("junc-gui.pot"))?),
    )
}

fn main() {
    let po = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../po/gui");
    println!("cargo:rerun-if-changed={}", po.display());
    let out = Path::new(&std::env::var("OUT_DIR").unwrap_or_default()).join("translations");
    if let Err(e) = bundle(&po, &out) {
        panic!("po/gui: {e}");
    }
    // Debug info lets tests find elements by their accessible labels
    // (spike S6); release builds leave it out.
    let debug = std::env::var("PROFILE").is_ok_and(|p| p != "release");
    let config = slint_build::CompilerConfiguration::new()
        .with_debug_info(debug)
        .with_bundled_translations(out)
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    if let Err(e) = slint_build::compile_with_config("ui/app.slint", config) {
        panic!("ui/app.slint: {e}");
    }
}
