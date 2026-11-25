#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Entry point inside an Arch Linux / Arch Linux ARM container: install the
# build dependencies from the distro, then run the checks as an unprivileged
# user (makepkg-style), with cargo's home inside the mounted workspace cache.
set -euo pipefail
for i in 1 2 3; do
  pacman -Syu --noconfirm --needed base-devel git rust clang pkgconf pipewire cargo-deny dbus \
    wireplumber pipewire-pulse libpulse pipewire-jack jack-example-tools fontconfig >/dev/null && break
  (( i == 3 )) && exit 1; sleep 20; pacman -Syy >/dev/null || true
done
id builder >/dev/null 2>&1 || useradd -m builder
chown -R builder: /src
exec su builder -c 'export CARGO_HOME=/src/.ci-cache/cargo CARGO_TARGET_DIR=/src/.ci-cache/target; \
  mkdir -p "$CARGO_HOME"; ln -sfn "$CARGO_TARGET_DIR" /src/target; cd /src \
  && s=$SECONDS && scripts/ci/check.sh && echo "== checks took $((SECONDS - s)) s" \
  && s=$SECONDS && scripts/ci/live-pipewire.sh \
  && t=$((SECONDS - s)) && echo "== layer-2 suite took $t s" \
  && { (( t < 600 )) || { echo "layer-2 suite over its 10 min budget"; exit 1; }; } \
  && s=$SECONDS && scripts/ci/bench.sh && cat target/bench-results.json \
  && echo "== bench took $((SECONDS - s)) s"'
