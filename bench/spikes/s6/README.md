# Spike S6: Slint bundled translations, live switch, ICU4X numbers

Run: `cargo test` in this directory (standalone; the workspace excludes
`bench/`). Slint 1.19.1, testing backend, no display needed.

What `tests/switch.rs` shows, in one process:

- `.po` catalogs under `translations/<lang>/LC_MESSAGES/s6.po` are compiled
  into the binary (`slint-build` `with_bundled_translations`); no `.mo`,
  no `setlocale`, no `LANGUAGE`.
- `slint::select_bundled_translation(lang)` switches every `@tr` in place:
  en → de → ru → bn → en. The accessible labels (what AT-SPI reads) change
  with it. A language that was not bundled is refused and the UI keeps
  its language.
- `@tr("{0} input connected" | "{0} inputs connected" % inputs, inputs-text)`:
  the int property picks the plural form (Russian 22 → входа, 25 → входов),
  and `{0}` shows the count as `junc_l10n::numbers` formats it
  (`1.234` in German, `২১` in Bengali).

Findings for the GUI (B12):

- Slint looks translations up with `msgctxt` set to the component name
  (`App` here), as `slint-tr-extractor` writes them. GUI catalogs carry
  contexts; the CLI's do not.
- Finding elements by label in tests needs Slint debug info
  (`CompilerConfiguration::with_debug_info(true)`), for test builds only.
