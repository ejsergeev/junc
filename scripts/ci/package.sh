#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Build the packages from the checked-out commit (docs/architecture.md
# §10.3), inside an Arch Linux / Arch Linux ARM container, as root:
# makepkg runs as an unprivileged user, namcap checks the result, and the
# packages land in .ci-cache/pkg/. The source is this repository at HEAD
# (not the release tag), the version 0.1.0.r<commits>.g<hash>.
set -euo pipefail
cd "$(dirname "$0")/../.."
# The PKGBUILD's makedepends, so the two lists cannot drift apart.
mapfile -t makedepends < <(source packaging/arch/PKGBUILD; printf '%s\n' "${makedepends[@]}")
for i in 1 2 3; do
  pacman -Syu --noconfirm --needed base-devel rust namcap "${makedepends[@]}" >/dev/null && break
  (( i == 3 )) && exit 1; sleep 20; pacman -Syy >/dev/null || true
done
id builder >/dev/null 2>&1 || useradd -m builder
out=/src/.ci-cache/pkg
work=$(mktemp -d /tmp/junc-pkg-XXXXXX)
rm -rf "$out"; mkdir -p "$out" /src/.ci-cache/cargo
commit=$(git -c safe.directory='*' rev-parse HEAD)
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
pkgver=$version.r$(git -c safe.directory='*' rev-list --count HEAD).g${commit:0:7}
sed -e "s|^pkgver=.*|pkgver=$pkgver|" \
    -e "s|^source=.*|source=(\"junc::git+file:///src#commit=$commit\")|" \
    packaging/arch/PKGBUILD > "$work/PKGBUILD"
cp packaging/arch/junc.install "$work/"
chown -R builder: "$work" "$out" /src/.ci-cache/cargo
su builder -c "git config --global --add safe.directory '*' \
  && cd $work && CARGO_HOME=/src/.ci-cache/cargo PKGDEST=$out makepkg --nocheck --noconfirm"
ls -l "$out"
# namcap: errors fail the build; warnings are printed for the record.
namcap "$work/PKGBUILD" | tee "$work/namcap.txt"
# Debug packages are left out: their build-id links point into the main
# package, which namcap reports as broken.
for p in "$out"/*.pkg.tar.*; do
  [[ $p == */junc-debug-* ]] || namcap "$p"
done | tee -a "$work/namcap.txt"
if grep -q ' E: ' "$work/namcap.txt"; then echo "namcap found errors"; exit 1; fi
echo "== packages built"
