#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Build a local Docker image "junc-ci-alarm" from the official, signed
# Arch Linux ARM aarch64 rootfs (no third-party images). Run on an arm64 host.
set -euo pipefail
url=http://os.archlinuxarm.org/os/ArchLinuxARM-aarch64-latest.tar.gz
# Arch Linux ARM Build System <builder@archlinuxarm.org>
fpr=68B3537F39A313B3E574D06777193F152BDBE6A6
work=$(mktemp -d); trap 'rm -rf "$work"' EXIT
curl -fsSL -o "$work/rootfs.tar.gz" "$url"
curl -fsSL -o "$work/rootfs.tar.gz.sig" "$url.sig"
export GNUPGHOME="$work/gnupg"; mkdir -m 700 "$GNUPGHOME"
gpg --batch --keyserver hkps://keyserver.ubuntu.com --recv-keys "$fpr"
gpg --batch --status-fd 1 --verify "$work/rootfs.tar.gz.sig" "$work/rootfs.tar.gz" \
  | grep -q "VALIDSIG $fpr" || { echo "rootfs signature check failed"; exit 1; }
docker import "$work/rootfs.tar.gz" junc-ci-alarm:base
docker build -t junc-ci-alarm - <<'DOCKERFILE'
FROM junc-ci-alarm:base
# pacman 7's Landlock sandbox cannot be applied inside docker build.
RUN sed -i -e 's/^CheckSpace/#CheckSpace/' -e '/^\[options\]/a DisableSandbox' /etc/pacman.conf \
 && pacman-key --init && pacman-key --populate archlinuxarm \
 && pacman -Syu --noconfirm --needed base-devel
DOCKERFILE
