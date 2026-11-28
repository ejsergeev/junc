#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# All per-PR code checks. Runs the same in CI containers and on a dev machine.
set -euo pipefail
cd "$(dirname "$0")/../.."
step() { printf '\n== %s\n' "$*"; }
step "toolchain"; rustc --version; cargo --version
step "fmt";       cargo fmt --all --check
step "clippy";    cargo clippy --workspace --all-targets --locked -- -D warnings
step "test";      cargo test --workspace --locked
step "meter";     cargo clippy -p junc-pw --features meter --all-targets --locked -- -D warnings
step "gui";       cargo clippy -p junc-gui --features scenarios --all-targets --locked -- -D warnings
cargo test -p junc-gui --features scenarios --locked
cargo build -p junc-gui --features scenarios --locked
scripts/gui/atspi-check.sh default flexoki narrow sourceOff offline wireHover picker onboardCheck onboardTemplates emptyNone emptyJunction junctionOff addListen addBus inputOptions inputOptionsNarrow cable linkDrag linkKeys rename deleteConfirm contextMenu appClosed newCable serviceDown settings import langPicker useDefault defaultRestored linkOverridden proAudio btWarning multichannel gallery
step "i18n";      command -v xgettext >/dev/null || { echo "xgettext (gettext) is missing"; exit 1; }
cargo xtask i18n check
step "deps";      scripts/ci/check-deps.sh
step "deny";      cargo deny --locked check
step "release";   cargo build --workspace --release --locked
size=$(stat -c %s target/release/juncd)
echo "juncd release binary: $size bytes"
(( size < 5 * 1024 * 1024 )) || { echo "juncd exceeds the 5 MB budget"; exit 1; }
