#!/usr/bin/env bash
# Smoke-run a shard of the libFuzzer targets. Run from the repo root.
#
# Usage: tools/fuzz-smoke.sh <shard 0..N-1> <shards N> <max_total_time seconds>
#
# Targets are dealt round-robin from `cargo fuzz list`, so a shard's membership
# changes when targets are added, but every target lands in exactly one shard.
set -euo pipefail
shard="$1" shards="$2" budget="$3"
cd "$(dirname "$0")/../fuzz"

fail=0
i=0
for t in $(cargo +nightly fuzz list); do
    if [ $((i % shards)) -eq "$shard" ]; then
        echo "::group::$t"
        if ! cargo +nightly fuzz run "$t" --target x86_64-unknown-linux-gnu \
             -- -runs=20000 -max_total_time="$budget" -print_final_stats=1; then
            echo "::error::fuzz target $t failed"
            fail=1
        fi
        echo "::endgroup::"
    fi
    i=$((i + 1))
done
exit "$fail"
