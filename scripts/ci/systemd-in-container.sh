#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Runs as root inside the systemd container of scripts/ci/systemd-user.sh
# (packages come from scripts/ci/systemd-prepare.sh; no network here).
# A user with 10 Junctions logs in (lingering, no GUI): juncd must start from
# its user unit after PipeWire, restore every Junction in under a second, be
# D-Bus activated when stopped, and follow `junc service`.
set -euo pipefail
step() { printf '\n== %s\n' "$*"; }
fail() { echo "FAIL: $*"; exit 1; }

step "boot"
systemctl is-system-running --wait >/dev/null 2>&1 || true
systemctl is-system-running || true

step "install"
install -m 755 /src/.ci-cache/target/release/juncd /src/.ci-cache/target/release/junc /usr/bin/
install -Dm 644 /src/packaging/systemd/juncd.service /usr/lib/systemd/user/juncd.service
install -Dm 644 /src/packaging/dbus/io.github.ejsergeev.Junc1.service \
  /usr/share/dbus-1/services/io.github.ejsergeev.Junc1.service
useradd -m tester
uid=$(id -u tester)
home=/home/tester
mkdir -p "$home/.config/junc" "$home/.config/systemd/user/default.target.wants"
{
  echo "schema = 1"
  for k in $(seq 0 9); do
    printf '[[junction]]\nid = "j%s"\nname = "Junction %s"\n' "$k" "$k"
    printf '[[junction.listen]]\nid = "out"\ntarget = "default"\n'
  done
} > "$home/.config/junc/junctions.toml"
# "Start at login", as `junc service enable` leaves it.
ln -s /usr/lib/systemd/user/juncd.service "$home/.config/systemd/user/default.target.wants/juncd.service"
chown -R tester: "$home"

as_user() {
  runuser -u tester -- env XDG_RUNTIME_DIR="/run/user/$uid" \
    DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$uid/bus" "$@"
}
wait_until() { # seconds, command…
  local end=$((SECONDS + $1)); shift
  until "$@" >/dev/null 2>&1; do (( SECONDS < end )) || return 1; sleep 0.05; done
}
routed() {
  as_user junc --json list | jq -e '[.data.junctions[] | select(.status == "active")] | length == 10'
}
restore_ms() { # the daemon's own measure, from its newest start
  # By identifier: in a container journald cannot map the cgroup to the
  # user unit, so --user-unit finds nothing.
  journalctl -t juncd --no-pager -o cat | grep 'graph complete after' | tail -1 \
    | sed -E 's/.*after ([0-9]+) ms/\1/' || true
}

step "login with no GUI"
t0=$(date +%s%N)
loginctl enable-linger tester
wait_until 30 test -S "/run/user/$uid/bus" || fail "no user bus"
wait_until 30 as_user systemctl --user is-active juncd.service || {
  as_user systemctl --user status juncd.service --no-pager || true; fail "juncd did not start"; }
wait_until 10 routed || { as_user junc list || true; fail "Junctions not restored"; }
t1=$(date +%s%N)
ms=$(restore_ms)
[ -n "$ms" ] || fail "no restore time in the journal"
echo "login to 10 Junctions routed: $(( (t1 - t0) / 1000000 )) ms (systemd, PipeWire and juncd); juncd restore: $ms ms"
(( ms < 1000 )) || fail "restore took $ms ms"
as_user systemd-analyze --user verify /usr/lib/systemd/user/juncd.service || fail "unit does not verify"
as_user systemctl --user show juncd.service -p Type -p NRestarts | tee /dev/stderr | grep -q '^Type=notify$'

step "D-Bus activation"
as_user systemctl --user stop juncd.service
as_user junc --json status | jq -e '.ok' >/dev/null || fail "activation"
wait_until 10 as_user systemctl --user is-active juncd.service || fail "not started by activation"
wait_until 10 routed || fail "not routed after activation"

step "junc service"
as_user junc --json service disable | jq -e '.data.enabled == false' >/dev/null || fail "disable"
as_user systemctl --user is-active juncd.service && fail "still running after disable"
as_user junc --json service enable | jq -e '.data.enabled and .data.active == "active"' >/dev/null || fail "enable"

step "log out and in again"
restores() { journalctl -t juncd -o cat | grep -c 'graph complete after' || true; }
seen=$(restores)
loginctl disable-linger tester
wait_until 30 sh -c "! test -S /run/user/$uid/bus" || fail "user session did not end"
loginctl enable-linger tester
wait_until 30 test -S "/run/user/$uid/bus" || fail "no user bus after second login"
wait_until 30 routed || fail "Junctions not restored after second login"
newer() { (( $(restores) > seen )); }
wait_until 5 newer || fail "juncd did not log the restore after the second login"
ms=$(restore_ms)
echo "second login: juncd restore $ms ms"
(( ms < 1000 )) || fail "restore took $ms ms"
echo "systemd user session: ok"
