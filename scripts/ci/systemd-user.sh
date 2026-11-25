#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# A15 acceptance (F9): boot an Arch container with systemd as PID 1, install
# the release binaries and the shipped units, and log a user in with no GUI
# (scripts/ci/systemd-in-container.sh). Needs the release build from
# check.sh in .ci-cache/target/release.
# usage: scripts/ci/systemd-user.sh IMAGE
set -euo pipefail
cd "$(dirname "$0")/../.."
image=$1
name=junc-systemd-$$
prepared=junc-systemd-image-$$
# Packages first, in a plain container: once systemd boots, the image's own
# network setup can replace Docker's DNS (seen on Arch Linux ARM).
docker run --name "$name-prep" -v "$PWD:/src:ro" "$image" /src/scripts/ci/systemd-prepare.sh
docker commit "$name-prep" "$prepared" >/dev/null
docker rm "$name-prep" >/dev/null
docker run -d --name "$name" --privileged --cgroupns=host --network none \
  -v /sys/fs/cgroup:/sys/fs/cgroup:rw --tmpfs /run --tmpfs /run/lock --tmpfs /tmp \
  -e container=docker -v "$PWD:/src:ro" "$prepared" /usr/lib/systemd/systemd >/dev/null
cleanup() {
  status=$?
  if (( status != 0 )); then
    echo "== journal (last 80 lines)"
    docker exec "$name" journalctl --no-pager -n 80 2>/dev/null || true
  fi
  docker rm -f "$name" >/dev/null 2>&1 || true
  docker rmi "$prepared" >/dev/null 2>&1 || true
  exit "$status"
}
trap cleanup EXIT
docker exec "$name" /src/scripts/ci/systemd-in-container.sh
