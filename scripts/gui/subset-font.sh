#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Rebuild the font GUI snapshots render with (crates/junc-gui/fonts/): a
# subset of JetBrains Mono Nerd Font (OFL-1.1, no reserved font name) with
# Latin, the punctuation and arrows JUNC uses, and the Nerd Font icons in
# ui/. Snapshots use only this font, so they look the same on every
# machine. Run it after adding an icon; commit the font on its own.
# usage: scripts/gui/subset-font.sh   (needs pyftsubset from fonttools)
set -euo pipefail
cd "$(dirname "$0")/../.."
src=${JUNC_FONT_SOURCE:-/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf}
out=crates/junc-gui/fonts/JetBrainsMonoNerdFont-JuncSubset.ttf
pyft=${PYFTSUBSET:-pyftsubset}
# Icons used in ui/, plus the Spec's speaker and HDMI glyphs.
icons=$(grep -rohE '\\u\{f[0-9a-f]{4,5}\}' crates/junc-gui/ui | sed -E 's/\\u\{(.*)\}/U+\1/' | sort -u | tr '\n' ',')
"$pyft" "$src" --output-file="$out" --layout-features='*' --no-hinting \
  --unicodes="U+0020-007E,U+00A0-017F,U+2010-2027,U+2030-205E,U+2190-21FF,U+2212,U+2260-2265,U+25A0-25FF,U+27E6-27E7,${icons}U+F04C3,U+F087F"
ls -l "$out"
