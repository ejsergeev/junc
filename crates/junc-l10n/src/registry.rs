// SPDX-License-Identifier: GPL-3.0-or-later

//! Omarchy's language registry (vendored in `data/omarchy-locales.json`):
//! the languages JUNC offers, their codes, number format and direction.
//! Codes are Omarchy's (`pt-BR`, `zh-CN`, `es-MX`); catalogs use the
//! gettext form ([`Language::gettext`]).

use serde::Deserialize;
use std::sync::OnceLock;

const DATA: &str = include_str!("../data/omarchy-locales.json");

/// The pseudo-locale for layout tests: English, accented, +40 %.
pub const PSEUDO: &str = "en_XA";

/// One language of the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Language {
    /// Registry code, as in `~/.config/omarchy/language` (`pt-BR`).
    pub code: String,
    /// Its own name ("Deutsch").
    pub name: String,
    /// Its English name ("German").
    pub english_name: String,
    /// BCP 47 tag for formatting numbers (`de-DE`).
    pub format_locale: String,
    /// Written right to left.
    pub rtl: bool,
    /// Decimal separator.
    pub decimal: String,
    /// The digit zero of its numbering system (`0`, `۰`, `০`).
    pub zero_digit: char,
}

impl Language {
    /// The code in gettext form, as catalogs are named: `pt-BR` → `pt_BR`,
    /// `no` → `nb`.
    pub fn gettext(&self) -> String {
        if self.code == "no" {
            return "nb".to_owned();
        }
        self.code.replace('-', "_")
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Raw {
    name: String,
    english_name: String,
    format_locale: String,
    #[serde(default)]
    direction: Option<String>,
    decimal: String,
    zero_digit: String,
}

/// The registry object's entries in file order (serde_json's map sorts
/// its keys unless a workspace-wide feature is turned on).
struct Ordered(Vec<(String, Raw)>);

impl<'de> Deserialize<'de> for Ordered {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Ordered;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object of languages")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut m: A,
            ) -> Result<Ordered, A::Error> {
                let mut out = Vec::new();
                while let Some(entry) = m.next_entry::<String, Raw>()? {
                    out.push(entry);
                }
                Ok(Ordered(out))
            }
        }
        d.deserialize_map(V)
    }
}

/// Every language of the registry, in its order (English first).
pub fn all() -> &'static [Language] {
    static ALL: OnceLock<Vec<Language>> = OnceLock::new();
    ALL.get_or_init(|| {
        let Ordered(entries) = serde_json::from_str(DATA).unwrap_or(Ordered(Vec::new()));
        entries
            .into_iter()
            .map(|(code, r)| Language {
                code,
                name: r.name,
                english_name: r.english_name,
                format_locale: r.format_locale,
                rtl: r.direction.as_deref() == Some("rtl"),
                decimal: r.decimal,
                zero_digit: r.zero_digit.chars().next().unwrap_or('0'),
            })
            .collect()
    })
}

/// The languages of v1: every left-to-right one. Right-to-left languages
/// show English until JUNC supports RTL layout (PRD).
pub fn v1() -> impl Iterator<Item = &'static Language> {
    all().iter().filter(|l| !l.rtl)
}

/// The registry language for a code in any common form (`pt-BR`, `pt_BR`,
/// `de_DE`, `nb`), falling back from a region to the bare language.
pub fn find(code: &str) -> Option<&'static Language> {
    let code = code.trim().replace('_', "-");
    let code = if code == "nb" { "no".to_owned() } else { code };
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    all().iter().find(|l| same(&l.code, &code)).or_else(|| {
        let base = code.split('-').next()?;
        all().iter().find(|l| same(&l.code, base))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_has_the_v1_languages() {
        assert_eq!(all().len(), 50);
        assert_eq!(all()[0].code, "en");
        assert_eq!(v1().count(), 46, "46 left-to-right languages in v1");
        let rtl: Vec<&str> = all()
            .iter()
            .filter(|l| l.rtl)
            .map(|l| l.code.as_str())
            .collect();
        assert_eq!(rtl, ["ar", "ur", "fa", "he"]);
        let fa = find("fa").expect("fa");
        assert_eq!(fa.zero_digit, '۰');
        assert_eq!(find("bn").expect("bn").zero_digit, '০');
    }

    #[test]
    fn codes_are_found_in_any_form() {
        assert_eq!(find("pt-BR").map(|l| l.code.as_str()), Some("pt-BR"));
        assert_eq!(find("pt_br").map(|l| l.code.as_str()), Some("pt-BR"));
        assert_eq!(find("pt_PT").map(|l| l.code.as_str()), Some("pt"));
        assert_eq!(find("de_DE").map(|l| l.code.as_str()), Some("de"));
        assert_eq!(find("nb").map(|l| l.code.as_str()), Some("no"));
        assert_eq!(find("tlh"), None);
        assert_eq!(find("zh-CN").expect("zh").gettext(), "zh_CN");
        assert_eq!(find("no").expect("no").gettext(), "nb");
    }
}
