// SPDX-License-Identifier: GPL-3.0-or-later

//! S6: one process switches language in place (no restart). The texts
//! assistive technology reads (accessible labels) change with it, and
//! plurals pick the form from the count while showing the count as ICU4X
//! formats it.

use i_slint_backend_testing::ElementRoot;
use s6::App;

fn labels(app: &App) -> Vec<String> {
    let mut out: Vec<String> = app
        .root_element()
        .query_descendants()
        .find_all()
        .into_iter()
        .filter_map(|e| e.accessible_label())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect();
    out.dedup();
    out
}

fn show(app: &App, lang: &str, n: i32) {
    slint::select_bundled_translation(if lang == "en" { "" } else { lang })
        .unwrap_or_else(|e| panic!("{lang}: {e:?}"));
    app.set_inputs(n);
    let text = junc_l10n::numbers::format_in(lang, f64::from(n), 0);
    app.set_inputs_text(text.into());
}

#[test]
fn language_switches_live_with_formatted_plurals() {
    i_slint_backend_testing::init_no_event_loop();
    let app = App::new().expect("app");
    show(&app, "en", 1);
    assert_eq!(labels(&app), ["Inputs", "Add input…", "1 input connected"]);
    show(&app, "de", 1234);
    assert_eq!(labels(&app), ["Eingänge", "Eingang hinzufügen…", "1.234 Eingänge verbunden"]);
    show(&app, "ru", 22);
    assert_eq!(labels(&app), ["Входы", "Добавить вход…", "22 входа подключено"]);
    show(&app, "ru", 25);
    assert_eq!(labels(&app)[2], "25 входов подключено");
    // Bengali digits come from ICU4X, not from Slint's {n}.
    show(&app, "bn", 21);
    assert_eq!(labels(&app), ["ইনপুট", "ইনপুট যোগ করুন…", "২১টি ইনপুট সংযুক্ত"]);
    show(&app, "en", 2);
    assert_eq!(labels(&app), ["Inputs", "Add input…", "2 inputs connected"]);
    // A language that was not bundled is refused; the UI keeps its language.
    assert!(slint::select_bundled_translation("fi").is_err());
    assert_eq!(labels(&app)[0], "Inputs");
}
