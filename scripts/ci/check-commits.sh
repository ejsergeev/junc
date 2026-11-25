#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Check the git rules of CONTRIBUTING.md on every commit in a range.
# usage: check-commits.sh <base-ref> [<head-ref>]   (default head: HEAD)
set -euo pipefail
base=${1:?base ref}; head=${2:-HEAD}
areas='core|daemon|cli|gui|i18n|skill|packaging|ci|docs'
fail=0
for c in $(git rev-list --no-merges "$base..$head"); do
  subj=$(git log -1 --format=%s "$c"); msg=$(git log -1 --format=%B "$c")
  author="$(git log -1 --format='%an <%ae>' "$c")"
  err=()
  [[ $subj =~ ^($areas):\ [^[:space:]] ]] || err+=("subject must start with '<area>: ' ($areas)")
  (( ${#subj} <= 72 )) || err+=("subject is ${#subj} chars (max 72)")
  grep -qxF "Signed-off-by: $author" <<<"$msg" || err+=("missing 'Signed-off-by: $author'")
  if (( ${#err[@]} )); then
    fail=1; echo "✗ ${c:0:7} $subj"; printf '    %s\n' "${err[@]}"
  fi
done
(( fail == 0 )) && echo "commit rules: ok ($(git rev-list --count --no-merges "$base..$head") commits)"
exit $fail
