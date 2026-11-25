// SPDX-License-Identifier: GPL-3.0-or-later

//! Language resolution (settings, Omarchy language file, locale
//! environment), translation catalogs and locale-aware number formatting
//! (docs/architecture.md §8).
//!
//! Alpha scaffold: every message goes through [`tr`], [`trf`] or [`trn`]
//! with its English text as the gettext msgid, so `xgettext
//! --keyword=tr --keyword=trf --keyword=trn:1,2` can extract them later.
//! Catalogs are not loaded yet; the English text is returned. Placeholders
//! are named (`{name}`) and whole sentences are translated, never fragments.

#![forbid(unsafe_code)]

pub mod mo;
pub mod plural;
pub mod registry;

use std::borrow::Cow;
use std::fmt::Display;
use std::path::PathBuf;

/// The message for `msgid` in the current language.
pub fn tr(msgid: &'static str) -> Cow<'static, str> {
    Cow::Borrowed(msgid)
}

/// The message for `msgid` with `{name}` placeholders filled in.
pub fn trf(msgid: &'static str, args: &[(&str, &dyn Display)]) -> String {
    fill(&tr(msgid), args)
}

/// Singular or plural message for `n`, with placeholders filled in. `{n}` is
/// available as a placeholder too.
pub fn trn(
    singular: &'static str,
    plural: &'static str,
    n: u64,
    args: &[(&str, &dyn Display)],
) -> String {
    let text = if n == 1 { singular } else { plural };
    let mut all: Vec<(&str, &dyn Display)> = vec![("n", &n)];
    all.extend_from_slice(args);
    fill(&tr(text), &all)
}

fn fill(text: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let key = &after[..close];
                match args.iter().find(|(k, _)| *k == key) {
                    Some((_, v)) => out.push_str(&v.to_string()),
                    None => out.push_str(&rest[open..open + close + 2]),
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// A language code in gettext form: `zh-CN` → `zh_CN`, `no` → `nb`,
/// encodings and modifiers dropped (`de_DE.UTF-8@euro` → `de_DE`).
pub fn normalize(code: &str) -> Option<String> {
    let code = code.trim();
    let code = code.split(['.', '@']).next().unwrap_or("");
    if code.is_empty() || code == "C" || code == "POSIX" {
        return None;
    }
    let code = code.replace('-', "_");
    Some(if code == "no" { "nb".to_owned() } else { code })
}

/// Where the language comes from, in order of precedence.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    /// `--lang` or the GUI setting.
    pub explicit: Option<String>,
    /// `$OMARCHY_LANGUAGE_FILE`, else `$XDG_CONFIG_HOME/omarchy/language`.
    pub omarchy_file: Option<PathBuf>,
    /// `LC_ALL`, `LC_MESSAGES`, `LANG` in that order.
    pub env: Vec<String>,
}

impl Sources {
    /// The sources of this process.
    pub fn from_env(explicit: Option<&str>) -> Self {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
        let omarchy_file = var("OMARCHY_LANGUAGE_FILE").map(PathBuf::from).or_else(|| {
            var("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".config")))
                .map(|d| d.join("omarchy/language"))
        });
        let env = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
            .collect();
        Self {
            explicit: explicit.map(str::to_owned),
            omarchy_file,
            env,
        }
    }
}

/// The language to use: the first source that names one, else `en`.
pub fn resolve(sources: &Sources) -> String {
    let from_file = || {
        let text = std::fs::read_to_string(sources.omarchy_file.as_ref()?).ok()?;
        normalize(text.lines().next()?)
    };
    sources
        .explicit
        .as_deref()
        .and_then(normalize)
        .or_else(from_file)
        .or_else(|| sources.env.iter().find_map(|v| normalize(v)))
        .unwrap_or_else(|| "en".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_filled_by_name() {
        assert_eq!(
            trf("Turned {name} off.", &[("name", &"Zoom Mic")]),
            "Turned Zoom Mic off."
        );
        assert_eq!(trf("{a} and {b}", &[("b", &2), ("a", &1)]), "1 and 2");
        assert_eq!(trf("keep {unknown} and {", &[]), "keep {unknown} and {");
        assert_eq!(trn("{n} input", "{n} inputs", 1, &[]), "1 input");
        assert_eq!(trn("{n} input", "{n} inputs", 3, &[]), "3 inputs");
    }

    #[test]
    fn codes_are_normalized() {
        assert_eq!(normalize("zh-CN").as_deref(), Some("zh_CN"));
        assert_eq!(normalize("de_DE.UTF-8@euro").as_deref(), Some("de_DE"));
        assert_eq!(normalize("no").as_deref(), Some("nb"));
        assert_eq!(normalize("C"), None);
        assert_eq!(normalize(" "), None);
    }

    #[test]
    fn resolution_follows_the_documented_order() {
        let dir = std::env::temp_dir().join(format!("junc-l10n-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("language");
        std::fs::write(&file, "pt-BR\n").expect("file");
        let mut s = Sources {
            explicit: Some("fr".into()),
            omarchy_file: Some(file.clone()),
            env: vec!["C.UTF-8".into(), "de_DE.UTF-8".into()],
        };
        assert_eq!(resolve(&s), "fr");
        s.explicit = None;
        assert_eq!(resolve(&s), "pt_BR");
        s.omarchy_file = Some(dir.join("missing"));
        assert_eq!(resolve(&s), "de_DE", "C is skipped");
        s.env.clear();
        assert_eq!(resolve(&s), "en");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
