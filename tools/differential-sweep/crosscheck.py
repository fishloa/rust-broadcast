#!/usr/bin/env python3
"""Self-consistency check of ONE sweep output (usually the HEAD one).

`diff.py` compares two trees; this compares two probes of the same tree:
`multimux.ts_ingest` (what `TsIngestSession` publishes) against
`transmux.stream` (what `StreamingTsDemux` emitted) on every capture, per track.
They must agree on sample count and SHA-256: any shortfall is a sample the
ingest session dropped (the run-2 BUG-2 class). Exits 1 on any mismatch.

    crosscheck.py head.jsonl [baseline.jsonl]

With a second file the same totals are printed for it (informational only).
"""

import json
import sys


def load(path):
    out = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            j = json.loads(line)
            out[(j["fixture"], j["probe"])] = j["result"]
    return out


def check(res):
    """Return (missing_total, [(fixture, track, emitted, published)])."""
    missing, bad = 0, []
    fixtures = sorted(f for f, p in res if p == "multimux.ts_ingest")
    for f in fixtures:
        stream = res.get((f, "transmux.stream"), {}).get("tracks", {})
        ingest = res[(f, "multimux.ts_ingest")].get("tracks", {})
        for tid, st in stream.items():
            got = ingest.get(tid, {})
            n = got.get("samples", 0)
            if n != st["samples"] or got.get("sha256") != st["sha256"]:
                bad.append((f, tid, st["samples"], n))
                missing += st["samples"] - n
    return missing, bad, len(fixtures)


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    missing, bad, n = check(load(argv[1]))
    print(f"{argv[1]}: {n} captures, {len(bad)} mismatching tracks, {missing} samples missing")
    for f, tid, e, p in bad:
        print(f"  {f} track {tid}: emitted {e}, published {p}")
    if len(argv) > 2:
        m2, b2, _ = check(load(argv[2]))
        print(f"{argv[2]}: {len(b2)} mismatching tracks, {m2} samples missing (informational)")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
