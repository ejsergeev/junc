#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Entry point inside an Arch Linux / Arch Linux ARM container: install the
# build dependencies from the distro, then run the checks as an unprivileged
# user (makepkg-style), with cargo's home inside the mounted workspace cache.
set -euo pipefail
pacman -Syu --noconfirm --needed base-devel git rust clang pkgconf pipewire cargo-deny >/dev/null
id builder >/dev/null 2>&1 || useradd -m builder
chown -R builder: /src
exec su builder -c 'export CARGO_HOME=/src/.ci-cache/cargo CARGO_TARGET_DIR=/src/.ci-cache/target; \
  mkdir -p "$CARGO_HOME"; ln -sfn "$CARGO_TARGET_DIR" /src/target; cd /src && scripts/ci/check.sh'
