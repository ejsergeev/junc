# Snapshot font

`JetBrainsMonoNerdFont-JuncSubset.ttf` is a subset of JetBrains Mono Nerd
Font 3.5.1 (`ttf-jetbrains-mono-nerd-basic`), licensed under the SIL Open
Font License 1.1 (`OFL.txt`; Nerd Fonts reserve no font name). It holds
Latin, the punctuation and arrows JUNC uses and the icons of `ui/`.
GUI snapshots (feature `scenarios`) render with this font only, never the
system's, so they are the same on every machine. The app itself uses the
system font fontconfig names. Rebuild with `scripts/gui/subset-font.sh`.

`NotoSansCJKjp-JuncSubset.otf` is a subset of Noto Sans CJK JP (Regular,
`noto-fonts-cjk` 20240730), licensed under the SIL Open Font License 1.1
(`NotoSansCJK-OFL.txt`), with only the characters `po/gui/ja.po` uses
and the punctuation of Japanese lists. Snapshots fall back to it for
Japanese scripts; the same script rebuilds it.
