"""Replay candidate delay estimators over the traces -> markdown tables.

Analysis uses floats for brevity; the crate will use the integer forms
(DESIGN.md section 3). Sample = RFC 5481 PDV (D - causal 2 s minimum D) at each
PCR-carrying chunk, arrival-timestamped.
"""
import os
import sys
sys.path.insert(0, os.path.dirname(__file__))
from common import CLEAN, STALL, load, pcr_samples, pdv_series

BUCKET_US = 10_000
NBUCKETS = 400  # 0..4 s


class Hist:
    """Forgetting histogram over per-interval maxima (NetEq-style, ramped forget)."""
    def __init__(self, forget):
        self.f, self.k, self.h = forget, 0, [0.0] * NBUCKETS

    def add(self, v):
        f = min(self.f, self.k / (self.k + 1.0))
        self.k += 1
        self.h = [x * f for x in self.h]
        self.h[min(NBUCKETS - 1, v // BUCKET_US)] += 1.0 - f

    def q(self, q):
        tot, c = sum(self.h), 0.0
        for i, x in enumerate(self.h):
            c += x
            if c >= q * tot:
                return (i + 1) * BUCKET_US
        return NBUCKETS * BUCKET_US


def interval_max(samples, T):
    """Yield (interval_end_us, max_pdv) for non-empty intervals (empty ones skipped)."""
    out, start, cur = [], None, 0
    for _, a, v in samples:
        if start is None:
            start = a
        while a >= start + T:
            if cur is not None:
                out.append((start + T, cur))
            start, cur = start + T, None
        cur = v if cur is None else max(cur, v)
    return [(e, m) for e, m in out if m is not None]


def run(name, rows):
    samp, _ = pdv_series(rows, 2_000_000)
    s = pcr_samples(rows)
    gap_i = STALL.get(name)
    gap_t = rows[gap_i][0] if gap_i else None
    series = {}
    for T in (250_000, 500_000):
        iv = interval_max(samp, T)
        for q in (0.95, 0.99):
            h, ser = Hist(0.98), []
            for e, m in iv:
                h.add(m)
                ser.append((e, h.q(q)))
            series[f"hist q{int(q*100)} T{T//1000}ms f0.98"] = ser
        for hold in (60, 120):
            pk, ser = 0, []
            last_peak_t = 0
            for e, m in iv:
                if m >= pk or e - last_peak_t > 2 * hold * 1_000_000:
                    pk, last_peak_t = m, e
                age = (e - last_peak_t) / 1e6
                held = pk if age <= hold else max(0, pk * (1 - (age - hold) / hold))
                ser.append((e, int(max(held, m))))
            series[f"max-hold {hold}s (then linear decay over {hold}s) T{T//1000}ms"] = ser
    # EWMA mean + 3 dev, per PCR sample, 0.999 (Media3 style)
    mean = dev = None
    ser = []
    for _, a, v in samp:
        if mean is None:
            mean, dev = v, 0.0
        else:
            mean = 0.999 * mean + 0.001 * v
            dev = 0.999 * dev + 0.001 * abs(v - mean)
        ser.append((a, int(mean + 3 * dev)))
    series["EWMA 0.999 mean+3dev (per PCR sample)"] = ser
    # RFC 3550 J on relative transit, x1 and x16
    j, ser, prev = 0, [], None
    for _, a, m in s:
        d = a - m
        if prev is not None:
            j += (abs(d - prev) - j) // 16
        prev = d
        ser.append((a, j))
    series["RFC 3550 J (x1)"] = ser
    series["RFC 3550 J x3"] = [(t, 3 * v) for t, v in ser]

    def at(ser, t):
        r = [v for e, v in ser if e <= t]
        return r[-1] if r else 0

    warm = 5_000_000
    if gap_t is not None:
        pre = gap_t - 100_000
        cps = [("pre-gap (t=%.1fs)" % (pre / 1e6), pre), ("gap+0.1s", gap_t + 100_000), ("gap+2s", gap_t + 2_000_000),
               ("gap+5s", gap_t + 5_000_000), ("end (%.1fs)" % (rows[-1][0] / 1e6), rows[-1][0])]
        print(f"\n#### {name}: estimator output (ms) around the gap (gap ends t={gap_t/1e6:.2f} s, PDV peak {max(v for _,a,v in samp if a>gap_t-100_000 and a<gap_t+2_000_000)/1e3:.0f} ms)\n")
        print("| estimator | max before gap (t>5s) | " + " | ".join(c for c, _ in cps) + " |\n|---|---|" + "---|" * len(cps))
        for k, ser in series.items():
            mb = max([v for e, v in ser if warm < e < pre] or [0])
            print(f"| {k} | {mb/1e3:.0f} | " + " | ".join(f"{at(ser, t)/1e3:.0f}" for _, t in cps) + " |")
    else:
        print(f"\n#### {name}: estimator output (ms), clean trace (t>5 s)\n")
        print("| estimator | p50 | max | at end |\n|---|---|---|---|")
        for k, ser in series.items():
            if "T250" in k or "120s" in k:
                continue
            v = sorted(x for e, x in ser if e > warm)
            print(f"| {k} | {v[len(v)//2]/1e3:.0f} | {v[-1]/1e3:.0f} | {ser[-1][1]/1e3:.0f} |")


def hold_analytic():
    print("\n#### Histogram memory of a single 1.4 s interval-max (analytic, forget f=0.98, new-interval weight 1-f=0.02)\n")
    print("After the event the mass in the 1.4 s bucket is (1-f)*f^n. A quantile q still includes it while that mass exceeds (1-q) of the total (total mass ~ 1).\n")
    print("| q | holds the event for n intervals | at T=250 ms | at T=500 ms |\n|---|---|---|---|")
    import math
    for q in (0.95, 0.99, 0.995):
        f = 0.98
        if (1 - f) <= (1 - q):
            print(f"| {q} | 0 (never holds: 1-f = 0.02 <= {1-q:.3f}) | 0 s | 0 s |")
        else:
            n = math.log((1 - f) / (1 - q)) / math.log(1 / f)
            print(f"| {q} | {n:.0f} | {n*0.25:.1f} s | {n*0.5:.1f} s |")


if __name__ == "__main__":
    print("### Estimator replay (PDV = D - 2 s windowed minimum; intervals with no PCR sample are skipped)")
    for n in ("tf-1", "6ter"):
        run(n, load(n))
    for n in CLEAN:
        run(n, load(n))
    hold_analytic()
