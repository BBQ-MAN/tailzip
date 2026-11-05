#!/bin/bash
# Download all upstream ALZ/EGG samples (CC0) into tests/cache/ for the optional
# `cached_large_samples` test. Not committed (gitignored).
set -euo pipefail
cd "$(dirname "$0")"
for r in unalz:alz unegg:egg; do
  repo=${r%%:*}; dir=cache/${r##*:}; mkdir -p "$dir"
  for f in $(curl -sSf "https://api.github.com/repos/alkegi/$repo/contents/tests/data" | grep '"name"' | cut -d'"' -f4); do
    [ -f "$dir/$f" ] || curl -sSfL -o "$dir/$f" "https://raw.githubusercontent.com/alkegi/$repo/master/tests/data/$f"
  done
done
ls cache/*
