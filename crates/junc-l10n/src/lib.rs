// SPDX-License-Identifier: GPL-3.0-or-later

//! Language resolution (settings, Omarchy language file, locale
//! environment), translation catalogs and locale-aware number formatting
//! (docs/architecture.md §8).
//!
//! Every message goes through [`tr`], [`trf`] or [`trn`] with its English
//! text as the gettext msgid (`cargo xtask i18n extract` collects them).
//! [`set_language`] loads the catalog `junc.mo` of a registry language from
//! `$JUNC_LOCALEDIR` or `/usr/share/locale`; a message the catalog lacks
//! stays English. Placeholders are named (`{name}`) and whole sentences
//! are translated, never fragments. The pseudo-locale `en_XA` (accented,
//! +40 %, bracketed) shows untranslated or clipped text in layout tests.

#![forbid(unsafe_code)]

pub mod mo;
pub mod plural;
pub mod registry;
pub mod watch;

use mo::Catalog;
use std::borrow::Cow;
use std::fmt::Display;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// The gettext domain: catalogs are `<dir>/<lang>/LC_MESSAGES/junc.mo`.
pub const DOMAIN: &str = "junc";

/// The language in use and its catalog.
#[derive(Debug)]
struct Active {
    code: String,
    catalog: Option<Catalog>,
}

static ACTIVE: RwLock<Option<Arc<Active>>> = RwLock::new(None);

fn active() -> Option<Arc<Active>> {
    ACTIVE.read().ok().and_then(|a| a.clone())
}

/// Where catalogs are: `$JUNC_LOCALEDIR` (tests, development) or
/// `/usr/share/locale`.
pub fn locale_dir() -> PathBuf {
    std::env::var_os("JUNC_LOCALEDIR")
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from("/usr/share/locale"), PathBuf::from)
}

/// The catalog for `lang`: its own (`pt_BR`), else its base language's.
fn load(dir: &std::path::Path, lang: &registry::Language) -> Option<Catalog> {
    let code = lang.gettext();
    let base = code.split('_').next().unwrap_or(&code).to_owned();
    [code, base].iter().find_map(|c| {
        let path = dir.join(c).join("LC_MESSAGES").join(format!("{DOMAIN}.mo"));
        Catalog::parse(&std::fs::read(path).ok()?).ok()
    })
}

/// Switches every following message to `code` (any form [`resolve`]
/// accepts) and returns the language now in use: a registry code, `en`
/// for unknown and right-to-left languages, or `en_XA`.
pub fn set_language(code: &str) -> String {
    set_language_in(&locale_dir(), code)
}

/// [`set_language`] with catalogs from `dir`.
pub fn set_language_in(dir: &std::path::Path, code: &str) -> String {
    let lang = effective(code);
    let catalog = registry::find(&lang)
        .filter(|_| lang != "en")
        .and_then(|l| load(dir, l));
    let next = Arc::new(Active {
        code: lang.clone(),
        catalog,
    });
    if let Ok(mut a) = ACTIVE.write() {
        *a = Some(next);
    }
    lang
}

/// The language in use (`en` until [`set_language`]).
pub fn language() -> String {
    active().map_or_else(|| "en".to_owned(), |a| a.code.clone())
}

/// What [`set_language`] would use for `code`.
fn effective(code: &str) -> String {
    let Some(code) = normalize(code) else {
        return "en".to_owned();
    };
    if code.eq_ignore_ascii_case(registry::PSEUDO) {
        return registry::PSEUDO.to_owned();
    }
    match registry::find(&code) {
        Some(l) if !l.rtl => l.code.clone(),
        _ => "en".to_owned(),
    }
}

/// `en_XA`: letters accented outside `{placeholders}`, padded to +40 %,
/// in brackets, so clipping and untranslated text show.
pub fn pseudo(s: &str) -> String {
    const FROM: &str = "aceinousyACEINOUSY";
    const TO: [char; 18] = [
        'å', 'ç', 'é', 'î', 'ñ', 'ö', 'û', 'š', 'ý', 'Å', 'Ç', 'É', 'Î', 'Ñ', 'Ö', 'Û', 'Š', 'Ý',
    ];
    let mut body = String::with_capacity(s.len() * 2);
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        let swapped = (depth == 0)
            .then(|| FROM.find(c).and_then(|i| TO.get(i).copied()))
            .flatten();
        body.push(swapped.unwrap_or(c));
    }
    let len = s.chars().count();
    let pad = ((len * 4 + 5) / 10).max(1);
    format!("⟦{body} {}⟧", "·".repeat(pad - 1))
}

/// The message for `msgid` in the current language.
pub fn tr(msgid: &'static str) -> Cow<'static, str> {
    let Some(a) = active() else {
        return Cow::Borrowed(msgid);
    };
    if a.code == registry::PSEUDO {
        return Cow::Owned(pseudo(msgid));
    }
    match a.catalog.as_ref().and_then(|c| c.get(msgid)) {
        Some(t) => Cow::Owned(t.to_owned()),
        None => Cow::Borrowed(msgid),
    }
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
    let english = if n == 1 { singular } else { plural };
    let text = match active() {
        Some(a) if a.code == registry::PSEUDO => pseudo(english),
        Some(a) => a
            .catalog
            .as_ref()
            .and_then(|c| c.nget(singular, n))
            .unwrap_or(english)
            .to_owned(),
        None => english.to_owned(),
    };
    let mut all: Vec<(&str, &dyn Display)> = vec![("n", &n)];
    all.extend_from_slice(args);
    fill(&text, &all)
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

/// The language to use, as [`set_language`] takes it: `--lang` or the
/// setting, else the Omarchy language file (an unknown code there means
/// English), else the first usable locale variable, else `en`.
pub fn resolve(sources: &Sources) -> String {
    if let Some(code) = sources.explicit.as_deref().and_then(normalize) {
        return effective(&code);
    }
    let file = sources
        .omarchy_file
        .as_ref()
        .and_then(|f| std::fs::read_to_string(f).ok());
    if let Some(text) = file {
        return effective(text.lines().next().unwrap_or(""));
    }
    sources
        .env
        .iter()
        .filter_map(|v| normalize(v))
        .map(|c| effective(&c))
        .find(|c| c != "en")
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
    fn the_pseudo_locale_keeps_placeholders() {
        assert_eq!(pseudo("Turn {name} on"), "⟦Tûrñ {name} öñ ·····⟧");
        assert_eq!(pseudo("Ok"), "⟦Ök ⟧");
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
        assert_eq!(resolve(&s), "pt-BR");
        // An unknown code in Omarchy's file means English, not the locale.
        std::fs::write(&file, "tlh\n").expect("file");
        assert_eq!(resolve(&s), "en");
        // Right-to-left languages show English until RTL lands.
        std::fs::write(&file, "ar\n").expect("file");
        assert_eq!(resolve(&s), "en");
        s.omarchy_file = Some(dir.join("missing"));
        assert_eq!(resolve(&s), "de", "C is skipped");
        s.env.clear();
        assert_eq!(resolve(&s), "en");
        s.explicit = Some("en_XA".into());
        assert_eq!(resolve(&s), "en_XA");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
