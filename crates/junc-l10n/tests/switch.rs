// SPDX-License-Identifier: GPL-3.0-or-later

//! Switching the language in a running process (F14): catalogs load,
//! missing messages stay English, plurals follow the language's rules,
//! and `en_XA` needs no catalog. One test, because the language is
//! process-wide.

use junc_l10n::mo;
use junc_l10n::{language, set_language_in, tr, trf, trn};

fn catalog(dir: &std::path::Path, lang: &str, plural: &str, entries: &[(&str, &str)]) {
    let path = dir.join(lang).join("LC_MESSAGES");
    std::fs::create_dir_all(&path).expect("dir");
    let mut all = vec![(
        String::new(),
        format!("Content-Type: text/plain; charset=UTF-8\nPlural-Forms: {plural}\n"),
    )];
    all.extend(
        entries
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned())),
    );
    std::fs::write(path.join("junc.mo"), mo::write(&all)).expect("mo");
}

#[test]
fn the_language_switches_in_place() {
    let dir = std::env::temp_dir().join(format!("junc-switch-{}", std::process::id()));
    catalog(
        &dir,
        "de",
        "nplurals=2; plural=(n != 1);",
        &[
            ("Turned {name} off.", "{name} ausgeschaltet."),
            ("{n} input\0{n} inputs", "{n} Eingang\0{n} Eingänge"),
        ],
    );
    catalog(
        &dir,
        "ru",
        "nplurals=3; plural=(n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);",
        &[("{n} input\0{n} inputs", "{n} вход\0{n} входа\0{n} входов")],
    );

    assert_eq!(language(), "en");
    assert_eq!(trn("{n} input", "{n} inputs", 1, &[]), "1 input");
    assert_eq!(trn("{n} input", "{n} inputs", 3, &[]), "3 inputs");
    assert_eq!(set_language_in(&dir, "de_DE.UTF-8"), "de");
    assert_eq!(
        trf("Turned {name} off.", &[("name", &"Zoom Mic")]),
        "Zoom Mic ausgeschaltet."
    );
    assert_eq!(tr("Not in the catalog"), "Not in the catalog");
    assert_eq!(trn("{n} input", "{n} inputs", 1, &[]), "1 Eingang");
    assert_eq!(trn("{n} input", "{n} inputs", 3, &[]), "3 Eingänge");

    assert_eq!(set_language_in(&dir, "ru"), "ru");
    assert_eq!(trn("{n} input", "{n} inputs", 22, &[]), "22 входа");
    assert_eq!(trn("{n} input", "{n} inputs", 25, &[]), "25 входов");
    assert_eq!(
        tr("Turned {name} off."),
        "Turned {name} off.",
        "no catalog entry"
    );

    // pt-BR without a catalog of its own uses pt's; here neither exists.
    assert_eq!(set_language_in(&dir, "pt-BR"), "pt-BR");
    assert_eq!(tr("Inputs"), "Inputs");

    assert_eq!(set_language_in(&dir, "en_XA"), "en_XA");
    assert_eq!(
        trf("Turned {name} off.", &[("name", &"X")]),
        "⟦Tûrñéd X öff. ······⟧"
    );

    assert_eq!(set_language_in(&dir, "he"), "en", "RTL waits");
    assert_eq!(set_language_in(&dir, "en"), "en");
    assert_eq!(tr("Turned {name} off."), "Turned {name} off.");
    let _ = std::fs::remove_dir_all(&dir);
}
