// SPDX-License-Identifier: GPL-3.0-or-later

//! A pure-Rust reader for GNU `.mo` catalogs (UTF-8 only, as JUNC writes
//! them), with plural forms. No C library and no locale setup: the
//! translation does not depend on `setlocale` (the validation found gettext
//! ignores `LANGUAGE` under a `C` locale).

use crate::plural::{self, Expr};
use std::collections::HashMap;

/// One compiled catalog.
#[derive(Debug, Clone)]
pub struct Catalog {
    /// msgid (the singular for plural entries) → translations.
    entries: HashMap<String, Vec<String>>,
    nplurals: usize,
    plural: Expr,
}

const MAGIC_LE: u32 = 0x9504_12de;
const MAGIC_BE: u32 = 0xde12_0495;

impl Catalog {
    /// Parses `.mo` bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let word = |at: usize, big: bool| -> Result<u32, String> {
            let b: [u8; 4] = bytes
                .get(at..at + 4)
                .and_then(|s| s.try_into().ok())
                .ok_or("truncated .mo file")?;
            Ok(if big {
                u32::from_be_bytes(b)
            } else {
                u32::from_le_bytes(b)
            })
        };
        let big = match word(0, false)? {
            MAGIC_LE => false,
            MAGIC_BE => true,
            _ => return Err("not a .mo file".into()),
        };
        let at = |v: u32| usize::try_from(v).map_err(|e| e.to_string());
        let count = at(word(8, big)?)?;
        let originals = at(word(12, big)?)?;
        let translations = at(word(16, big)?)?;
        let string = |table: usize, i: usize| -> Result<&str, String> {
            let len = at(word(table + 8 * i, big)?)?;
            let off = at(word(table + 8 * i + 4, big)?)?;
            let raw = bytes
                .get(off..off + len)
                .ok_or("string outside the .mo file")?;
            std::str::from_utf8(raw).map_err(|_| "catalog is not UTF-8".to_owned())
        };
        let mut entries = HashMap::with_capacity(count);
        let mut header = "";
        for i in 0..count {
            let id = string(originals, i)?;
            let text = string(translations, i)?;
            // Contexts (`ctx\x04id`) are not used by JUNC; keep them as is.
            let singular = id.split('\0').next().unwrap_or("");
            if singular.is_empty() {
                header = text;
                continue;
            }
            let forms: Vec<String> = text.split('\0').map(str::to_owned).collect();
            if forms.iter().any(|f| !f.is_empty()) {
                entries.insert(singular.to_owned(), forms);
            }
        }
        let (nplurals, plural) = header
            .lines()
            .find_map(|l| l.strip_prefix("Plural-Forms:"))
            .map_or_else(
                || Ok((2, plural::parse("n != 1").unwrap_or(Expr::Num(0)))),
                plural::from_header,
            )?;
        Ok(Self {
            entries,
            nplurals,
            plural,
        })
    }

    /// The translation of `msgid`, if the catalog has one.
    pub fn get(&self, msgid: &str) -> Option<&str> {
        self.entries
            .get(msgid)
            .and_then(|f| f.first())
            .map(String::as_str)
            .filter(|s| !s.is_empty())
    }

    /// The translation of a plural message for `n`, if the catalog has it.
    pub fn nget(&self, singular: &str, n: u64) -> Option<&str> {
        let forms = self.entries.get(singular)?;
        let i = usize::try_from(self.plural.eval(n))
            .unwrap_or(0)
            .min(self.nplurals.saturating_sub(1));
        forms.get(i).map(String::as_str).filter(|s| !s.is_empty())
    }

    /// Number of translated messages.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the catalog translates nothing.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Writes a `.mo` file (little endian, no hash table): used by tests and
/// by `cargo xtask i18n` so building catalogs needs no gettext tools.
/// `entries` are `(msgid, msgstr)`; plural entries use `\0`-joined forms.
pub fn write(entries: &[(String, String)]) -> Vec<u8> {
    let mut sorted: Vec<&(String, String)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let n = sorted.len();
    let header_len = 28;
    let tables = header_len + 16 * n;
    let mut strings = Vec::new();
    let mut orig = Vec::with_capacity(n);
    let mut trans = Vec::with_capacity(n);
    let place = |s: &str, strings: &mut Vec<u8>| {
        let off = tables + strings.len();
        strings.extend_from_slice(s.as_bytes());
        strings.push(0);
        (s.len(), off)
    };
    for (id, _) in &sorted {
        orig.push(place(id, &mut strings));
    }
    for (_, text) in &sorted {
        trans.push(place(text, &mut strings));
    }
    let u = |v: usize| u32::try_from(v).unwrap_or(u32::MAX).to_le_bytes();
    let mut out = Vec::with_capacity(tables + strings.len());
    for v in [
        MAGIC_LE as usize,
        0,
        n,
        header_len,
        header_len + 8 * n,
        0,
        tables,
    ] {
        out.extend_from_slice(&u(v));
    }
    for (len, off) in orig.iter().chain(&trans) {
        out.extend_from_slice(&u(*len));
        out.extend_from_slice(&u(*off));
    }
    out.extend_from_slice(&strings);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ru() -> Vec<u8> {
        write(&[
            (
                String::new(),
                "Content-Type: text/plain; charset=UTF-8\nPlural-Forms: nplurals=3; plural=(n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);\n".into(),
            ),
            ("Inputs".into(), "Входы".into()),
            ("Untranslated".into(), String::new()),
            (
                "{n} input\0{n} inputs".into(),
                "{n} вход\0{n} входа\0{n} входов".into(),
            ),
        ])
    }

    #[test]
    fn a_written_catalog_reads_back() {
        let c = Catalog::parse(&ru()).expect("parse");
        assert_eq!(c.get("Inputs"), Some("Входы"));
        assert_eq!(c.get("Untranslated"), None);
        assert_eq!(c.get("Missing"), None);
        assert_eq!(c.nget("{n} input", 1), Some("{n} вход"));
        assert_eq!(c.nget("{n} input", 3), Some("{n} входа"));
        assert_eq!(c.nget("{n} input", 5), Some("{n} входов"));
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn msgfmt_output_reads_too() {
        // A catalog compiled by GNU msgfmt (big hash table included), if
        // the tool is there.
        let dir = std::env::temp_dir().join(format!("junc-mo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let po = dir.join("de.po");
        std::fs::write(
            &po,
            "msgid \"\"\nmsgstr \"\"\n\"Content-Type: text/plain; charset=UTF-8\\n\"\n\"Plural-Forms: nplurals=2; plural=(n != 1);\\n\"\n\nmsgid \"Inputs\"\nmsgstr \"Eingänge\"\n\nmsgid \"{n} input\"\nmsgid_plural \"{n} inputs\"\nmsgstr[0] \"{n} Eingang\"\nmsgstr[1] \"{n} Eingänge\"\n",
        )
        .expect("po");
        let mo = dir.join("de.mo");
        let ok = std::process::Command::new("msgfmt")
            .arg("-o")
            .arg(&mo)
            .arg(&po)
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            let c = Catalog::parse(&std::fs::read(&mo).expect("mo")).expect("parse");
            assert_eq!(c.get("Inputs"), Some("Eingänge"));
            assert_eq!(c.nget("{n} input", 2), Some("{n} Eingänge"));
        } else {
            assert!(std::env::var_os("CI").is_none(), "msgfmt is missing in CI");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn garbage_is_refused() {
        assert!(Catalog::parse(b"nope").is_err());
        let mut bytes = ru();
        bytes.truncate(40);
        assert!(Catalog::parse(&bytes).is_err());
    }
}
