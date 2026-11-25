#!/usr/bin/env bash
# Render the prototype once per language and save PNGs (uses Slint's Window::take_snapshot).
# usage: i18n/screenshots.sh [outdir]   (needs DISPLAY or WAYLAND_DISPLAY)
set -u
D=$(cd "$(dirname "$0")/.." && pwd); OUT=${1:-$D/../screenshots/i18n}; mkdir -p "$OUT"
[ -f /tmp/junc-session.env ] && source /tmp/junc-session.env
export LANG=${LANG:-en_US.UTF-8}
for l in en ar he fa ur hi bn ta si th ja zh_CN ko ru de fi en_XA; do
  JUNC_LANG=$l SLINT_BACKEND=${SLINT_BACKEND:-winit-software} "$D/target/release/junc-proto" --snapshot "$OUT/$l.png" 2>/dev/null \
    && echo "ok $l" || echo "FAIL $l"
done
