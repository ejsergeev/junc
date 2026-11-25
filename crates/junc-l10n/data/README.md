# Vendored data

`omarchy-locales.json` is Omarchy's language registry
(`default/i18n/locales.json`, Omarchy 4.5.0, MIT license,
https://github.com/omacom/omarchy), copied unchanged. JUNC offers the same
languages as Omarchy and reads the same codes from
`~/.config/omarchy/language`. Update it by copying the file from a newer
Omarchy; `junc-l10n`'s tests check that it still parses and that the v1
set is the 46 left-to-right languages.
