# Snapshot font

`JetBrainsMonoNerdFont-JuncSubset.ttf` is a subset of JetBrains Mono Nerd
Font 3.5.1 (`ttf-jetbrains-mono-nerd-basic`), licensed under the SIL Open
Font License 1.1 (`OFL.txt`; Nerd Fonts reserve no font name). It holds
Latin, the punctuation and arrows JUNC uses and the icons of `ui/`.
GUI snapshots (feature `scenarios`) render with this font only, never the
system's, so they are the same on every machine. The app itself uses the
system font fontconfig names. Rebuild with `scripts/gui/subset-font.sh`.
