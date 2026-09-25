#!/bin/sh
# Fetches every repository in bench/foreign/repos.txt at its pinned commit into
# <dir>/<name>. One commit, no history: what the question sets were read from.
set -eu
dir=${1:?usage: fetch.sh <dir>}
list=$(dirname "$0")/repos.txt
mkdir -p "$dir"
grep -v '^#' "$list" | while read -r name url commit; do
    [ -n "$name" ] || continue
    if [ "$(git -C "$dir/$name" rev-parse HEAD 2>/dev/null)" = "$commit" ]; then
        continue
    fi
    rm -rf "${dir:?}/$name"
    git init -q "$dir/$name"
    git -C "$dir/$name" fetch -q --depth 1 "$url" "$commit"
    git -C "$dir/$name" checkout -q FETCH_HEAD
done
