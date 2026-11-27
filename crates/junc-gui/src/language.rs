// SPDX-License-Identifier: GPL-3.0-or-later

//! The language picker of Settings (design board 31, architecture §8):
//! the current language first, then the 46 left-to-right languages of
//! Omarchy's registry by their own names, filtered by what is typed.
//! Right-to-left languages show English until JUNC lays out RTL (PRD), so
//! they are not offered.

use junc_l10n::registry;

/// One row of the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Registry code (`de`, `pt-BR`).
    pub code: String,
    /// The language's own name ("Deutsch").
    pub name: String,
    /// The language in use now (the "Current" group).
    pub current: bool,
}

/// The language's own name, for the Settings row; English for a code the
/// registry does not know.
pub fn native_name(code: &str) -> String {
    registry::find(code).map_or_else(|| "English".to_owned(), |l| l.name.clone())
}

/// The rows for `current` and the search text: the current language,
/// then every v1 language but it, by code (as board 31 lists them). A search
/// matches the own name, the English name or the code, in any case.
pub fn rows(current: &str, search: &str) -> Vec<Row> {
    let search = search.trim().to_lowercase();
    let current = registry::find(current).map_or("en", |l| l.code.as_str());
    let matches = |l: &registry::Language| {
        search.is_empty()
            || l.name.to_lowercase().contains(&search)
            || l.english_name.to_lowercase().contains(&search)
            || l.code.to_lowercase().starts_with(&search)
    };
    let mut out: Vec<Row> = registry::v1()
        .filter(|l| l.code == current && matches(l))
        .map(|l| Row {
            code: l.code.clone(),
            name: l.name.clone(),
            current: true,
        })
        .collect();
    let mut rest: Vec<&registry::Language> = registry::v1()
        .filter(|l| l.code != current && matches(l))
        .collect();
    rest.sort_by(|a, b| a.code.cmp(&b.code));
    out.extend(rest.into_iter().map(|l| Row {
        code: l.code.clone(),
        name: l.name.clone(),
        current: false,
    }));
    out
}

/// How many languages the picker offers.
pub fn count() -> usize {
    registry::v1().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_language_comes_first_then_every_ltr_one() {
        let all = rows("de", "");
        assert_eq!(all.len(), count());
        assert_eq!(count(), 46, "Omarchy's LTR languages");
        assert!(all[0].current);
        assert_eq!(all[0].name, "Deutsch");
        assert!(all[1..].iter().all(|r| !r.current));
        assert!(all.iter().all(|r| r.code != "ar"), "RTL waits for RTL");
        assert!(
            all[1..].windows(2).all(|w| w[0].code < w[1].code),
            "by code"
        );
    }

    #[test]
    fn a_search_matches_own_or_english_names() {
        let ja: Vec<String> = rows("en", "japan").into_iter().map(|r| r.name).collect();
        assert_eq!(ja, ["日本語"]);
        assert_eq!(rows("en", "deutsch")[0].code, "de");
        assert!(rows("en", "zz-nothing").is_empty());
        assert_eq!(native_name("de"), "Deutsch");
        assert_eq!(native_name("xx"), "English");
    }
}
