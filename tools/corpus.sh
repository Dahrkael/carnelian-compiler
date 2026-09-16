#!/bin/sh
# Real-world corpus check (dev only): fetch sources if missing, then
# enforce corpus/baseline.tsv on both frontends.
set -eu
root=$(dirname "$0")/..
if [ ! -d "$root/.corpus" ]; then
    "$root/tools/fetch_corpus.sh"
fi
exec cargo run -q -p carnelian-cli -- corpus --frontends all --check
