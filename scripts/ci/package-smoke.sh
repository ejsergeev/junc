#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Install the built packages into a fresh Arch container and use them
# (docs/architecture.md §10.3), as root: pacman pulls the dependencies the
# packages declare, then an unprivileged user runs the installed binaries
# against an isolated PipeWire (and the GUI under Xvfb). juncd is started by D-Bus activation from
# the packaged service file, as on a desktop.
set -euo pipefail
cd "$(dirname "$0")/../.."
pkgs=(/src/.ci-cache/pkg/junc-[0-9]*.pkg.tar.* /src/.ci-cache/pkg/junc-gui-[0-9]*.pkg.tar.*)
# The archlinux Docker image skips /usr/share/locale (NoExtract); a desktop
# installs it, and the check reads the CLI's catalog from there.
sed -i '/^NoExtract/d' /etc/pacman.conf
for i in 1 2 3; do
  pacman -Syu --noconfirm >/dev/null && pacman -U --noconfirm "${pkgs[@]}" >/dev/null \
    && pacman -S --noconfirm --needed --asdeps pipewire-pulse dbus \
         xorg-server-xvfb libx11 libxcursor libxi libxrandr >/dev/null && break
  (( i == 3 )) && exit 1; sleep 20; pacman -Syy >/dev/null || true
done
pacman -Q junc junc-gui
id tester >/dev/null 2>&1 || useradd -m tester
exec su tester -c 'dbus-run-session -- /src/scripts/ci/with-pipewire.sh scripts/ci/package-check.sh'
