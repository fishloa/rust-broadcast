"""Shared loaders for the buffering-runtime trace analysis (deterministic, stdlib only).

Traces live in the PRIVATE repo checkout (private/fixtures/buffering-runtime/); pass
--dir or set BUFFERING_RUNTIME_TRACES. Time bases: arrival in integer microseconds
(arrival_ns // 1000), media time in integer microseconds (pcr_27mhz // 27),
with the 33-bit-base PCR wrap unrolled with the clock33 semantics
(signed distance, half-range pivot).
"""
import csv
import os
import sys

DEFAULT_DIR = "/Volumes/External/Projects/rust-broadcast/private/fixtures/buffering-runtime"
PCR_HZ = 27_000_000
PCR_WRAP = (1 << 33) * 300  # clock33::PCR_WRAP_27MHZ


def trace_dir():
    a = sys.argv[1:]
    if "--dir" in a:
        return a[a.index("--dir") + 1]
    return os.environ.get("BUFFERING_RUNTIME_TRACES", DEFAULT_DIR)


def pcr_signed_distance(frm, to):
    """clock33::pcr_signed_distance: shortest signed distance on the 27 MHz ring."""
    d = (to - frm) % PCR_WRAP
    return d - PCR_WRAP if d >= PCR_WRAP // 2 else d


ALL = ["t18", "rai-1", "rai-sport", "orf1", "orf-sport", "servustv", "6ter", "tf-1"]
STALL = {"tf-1": 17897, "6ter": 37847, "servustv#1": 136}  # chunk index that ends the delivery gap (servustv#1: STARTUP gap, 0.49 s into the stream)
CLEAN = ["t18", "rai-1", "rai-sport", "orf1", "orf-sport", "servustv#2"]
RECONNECT = {"servustv": 1980}              # chunk index of the first chunk of the new connection
STALL_TRACES = ["tf-1", "6ter", "servustv#1"]


def load(name, d=None):
    """Return list of (arrival_us, bytes, media_us_or_None) with arrival rebased to 0.
    'servustv#1' / 'servustv#2' are the two connections of servustv, split at the reconnect."""
    seg = None
    if "#" in name:
        name, seg = name.split("#")
    d = d or trace_dir()
    rows = []
    with open(os.path.join(d, name + ".csv")) as f:
        for r in csv.DictReader(f):
            rows.append((int(r["arrival_ns"]) // 1000, int(r["bytes"]),
                         int(r["pcr_27mhz"]) if r["pcr_27mhz"] else None))
    if seg:
        k = RECONNECT[name]
        rows = rows[:k] if seg == "1" else rows[k:]
    t0 = rows[0][0]
    out, prev, un = [], None, 0
    first = next(r[2] for r in rows if r[2] is not None)
    for a, b, p in rows:
        if p is None:
            out.append((a - t0, b, None))
            continue
        un = 0 if prev is None else un + pcr_signed_distance(prev, p)
        prev = p
        out.append((a - t0, b, un // 27))  # media microseconds since first PCR
    return out


def pcr_samples(rows):
    """[(chunk_index, arrival_us, media_us)] for chunks carrying a PCR."""
    return [(i, a, m) for i, (a, _, m) in enumerate(rows) if m is not None]


def pct(sorted_vals, q):
    """Nearest-rank percentile (q in 0..100) on a pre-sorted list."""
    n = len(sorted_vals)
    k = max(0, min(n - 1, -(-int(q * 1000) * n // 100000) - 1))
    return sorted_vals[k]


def windowed_min(times, vals, window_us):
    """Causal sliding minimum of vals over the trailing window (monotonic deque)."""
    from collections import deque
    dq, out = deque(), []
    for t, v in zip(times, vals):
        while dq and dq[-1][1] >= v:
            dq.pop()
        dq.append((t, v))
        while dq[0][0] <= t - window_us:
            dq.popleft()
        out.append(dq[0][1])
    return out


def pdv_series(rows, window_us):
    """RFC 5481 PDV: D=arrival-media (relative transit) minus sliding minimum D.
    window_us=None uses the whole-trace running minimum (zenith-tv's lateness)."""
    s = pcr_samples(rows)
    t = [a for _, a, _ in s]
    d = [a - m for _, a, m in s]
    if window_us is None:
        mn, out = None, []
        for x in d:
            mn = x if mn is None else min(mn, x)
            out.append(mn)
        dmin = out
    else:
        dmin = windowed_min(t, d, window_us)
    return [(i, a, x - m) for (i, a, _), x, m in zip(s, d, dmin)], d
