#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# What package-smoke.sh checks in the fresh container, as a user, against
# an isolated PipeWire: the installed files, the CLI's catalogs, juncd
# through D-Bus activation, and junc doctor.
set -euo pipefail
fail() { echo "package: $*"; exit 1; }
for f in /usr/bin/juncd /usr/bin/junc /usr/bin/junc-gui \
  /usr/lib/systemd/user/juncd.service \
  /usr/share/dbus-1/services/io.github.ejsergeev.Junc1.service \
  /usr/share/junc/templates/cable.toml \
  /usr/share/junc/agent/skills/junc/SKILL.md \
  /usr/share/junc/omarchy/plugin/manifest.json \
  /usr/share/locale/de/LC_MESSAGES/junc.mo \
  /usr/share/applications/junc.desktop \
  /usr/share/icons/hicolor/scalable/apps/junc.svg \
  /usr/share/icons/hicolor/symbolic/apps/junc-symbolic.svg \
  /usr/share/icons/hicolor/48x48/apps/junc.png; do
  [ -e "$f" ] || fail "$f is missing"
done
[ -e /usr/share/junc/omarchy/plugin/tests ] && fail "the plugin's tests were packaged"
missing=$(ldd /usr/bin/juncd /usr/bin/junc /usr/bin/junc-gui | grep 'not found' || true)
[ -z "$missing" ] || fail "unresolved libraries: $missing"
# The CLI reads its catalogs from /usr/share/locale.
de=$(OMARCHY_LANGUAGE_FILE=/nonexistent LC_ALL=de_DE.UTF-8 JUNC_SKILL_DIR=/nonexistent \
  junc skill status 2>&1 || true)
grep -q 'kein JUNC-Skill' <<<"$de" || fail "the CLI is not German with LC_ALL=de_DE: $de"
# juncd is not running: the first call starts it through D-Bus activation.
junc list || { cat "$XDG_STATE_HOME"/junc/*.log 2>/dev/null || true; fail "junc list"; }
pgrep -x juncd >/dev/null || fail "juncd was not activated"
junc --json doctor > /tmp/doctor.json || true
cat /tmp/doctor.json
grep -q '"juncd"' /tmp/doctor.json || fail "junc doctor did not report juncd"
echo "== package smoke test ok"
