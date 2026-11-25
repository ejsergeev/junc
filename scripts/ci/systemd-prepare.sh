#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Packages for scripts/ci/systemd-in-container.sh, installed in a plain
# container (normal Docker networking) before systemd boots.
set -euo pipefail
for i in 1 2 3; do
  pacman -Sy --noconfirm --needed pipewire wireplumber dbus-units jq >/dev/null && break
  (( i == 3 )) && { echo "FAIL: pacman"; exit 1; }
  sleep 20
done
# Arch enables these for every user through presets; an image skips them.
systemctl --global enable pipewire.socket pipewire.service wireplumber.service
# No consoles in a container: stop getty from restarting in a loop.
systemctl mask getty@tty1.service console-getty.service >/dev/null 2>&1 || true
