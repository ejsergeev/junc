#!/usr/bin/env bash
# Name-availability probe for "junc" across package registries.
# Usage: ./check_names.sh [name ...]   (default: junc junc-daemon junc-gui junc-cli)
# Output: one line per registry/name with EXACT (taken/free) and fuzzy hits.
# Requires: curl, jq. Optional: yay (prints `yay -Ss` output if present).
set -uo pipefail
NAMES=("${@:-junc junc-daemon junc-gui junc-cli}")
# shellcheck disable=SC2206
NAMES=(${NAMES[*]})
UA="junc-namecheck/1.0 (+https://github.com/; validation script)"
c() { curl -s -m 30 -A "$UA" "$@"; }
code() { curl -s -o /dev/null -w '%{http_code}' -m 30 -A "$UA" "$@"; }

echo "# name check $(date -u +%FT%TZ)"
for n in "${NAMES[@]}"; do
  echo "## $n"
  # AUR: exact info + substring search on name
  ex=$(c "https://aur.archlinux.org/rpc/v5/info?arg[]=$n" | jq -r '.resultcount')
  echo "AUR exact: $([ "$ex" = 0 ] && echo FREE || echo TAKEN)"
  # Arch official repos: exact name
  ex=$(c "https://archlinux.org/packages/search/json/?name=$n" | jq -r '.results|length')
  echo "Arch repos exact: $([ "$ex" = 0 ] && echo FREE || echo TAKEN)"
  # crates.io exact
  s=$(code "https://crates.io/api/v1/crates/$n")
  echo "crates.io exact: $([ "$s" = 404 ] && echo FREE || echo "TAKEN (HTTP $s)")"
  # npm exact
  s=$(code "https://registry.npmjs.org/$n")
  echo "npm exact: $([ "$s" = 404 ] && echo FREE || echo "TAKEN (HTTP $s)")"
  # Flathub: app ids containing the name
  hits=$(c -X POST -H 'Content-Type: application/json' -d "{\"query\":\"$n\"}" \
      https://flathub.org/api/v2/search | jq -r '.hits[]?.app_id' | grep -i "$n" || true)
  echo "Flathub app-id contains '$n': ${hits:-none}"
done

echo "## fuzzy: AUR names containing 'junc'"
c "https://aur.archlinux.org/rpc/v5/search/junc?by=name" | jq -r '.results[]|"\(.Name)\t\(.Description)"'
echo "## fuzzy: Arch repo pkgnames containing 'junc'"
c "https://archlinux.org/packages/search/json/?q=junc" | jq -r '.results[].pkgname' | grep -i junc | sort -u
echo "## fuzzy: crates.io names starting junc"
c "https://crates.io/api/v1/crates?q=junc&per_page=100" | jq -r '.crates[].name' | grep -i '^junc' || true
echo "## fuzzy: npm names containing junc"
c "https://registry.npmjs.org/-/v1/search?text=junc&size=100" | jq -r '.objects[].package.name' | grep -i junc || true
echo "## Flathub full-text hits for 'junc'"
c -X POST -H 'Content-Type: application/json' -d '{"query":"junc"}' https://flathub.org/api/v2/search \
  | jq -r '.hits[]?|"\(.app_id)\t\(.name)"' | head -20
if command -v yay >/dev/null; then echo "## yay -Ss junc"; yay -Ss junc; fi
if command -v gh >/dev/null && gh auth status >/dev/null 2>&1; then
  echo "## GitHub repos named junc*"
  gh search repos junc --match name --limit 30 --json fullName,description,stargazersCount \
    -q '.[]|"\(.fullName)\t★\(.stargazersCount)\t\(.description)"'
  echo "## GitHub user/org 'junc'"; gh api users/junc -q '.login+" ("+.type+")"' 2>/dev/null || echo "none"
fi
