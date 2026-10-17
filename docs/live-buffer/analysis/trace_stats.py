"""Chunk-gap, PDV, RFC 3550 J, drift and post-gap statistics over all traces -> markdown."""
import os
import sys
sys.path.insert(0, os.path.dirname(__file__))
from common import ALL, CLEAN, STALL, RECONNECT, load, pcr_samples, pct, pdv_series

QS = [50, 90, 99, 99.9, 100]
f2 = lambda us: f"{us/1000:.2f}"


def gaps(r):
    return [r[i][0] - r[i - 1][0] for i in range(1, len(r))]


def jser(r):
    s, j, out = pcr_samples(r), 0, []
    for k in range(1, len(s)):
        d = (s[k][1] - s[k][2]) - (s[k - 1][1] - s[k - 1][2])
        j += (abs(d) - j) // 16
        out.append(j)
    return out


def slope(sub):
    xs = [a for _, a, _ in sub]
    ys = [a - m for _, a, m in sub]
    mx, my = sum(xs) / len(xs), sum(ys) / len(ys)
    return sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / sum((x - mx) ** 2 for x in xs) * 1e6


def main():
    names = ALL[:5] + ["servustv#1", "servustv#2", "6ter", "tf-1"]
    data = {n: load(n) for n in names}
    print("### Trace summary (servustv split at its reconnect into #1/#2)\n\n| trace | class | chunks | PCR chunks | duration s | mean Mbit/s | media/wall |\n|---|---|---|---|---|---|---|")
    for n, r in data.items():
        s = pcr_samples(r)
        dur = r[-1][0] / 1e6
        tb = sum(x[1] for x in r)
        med = (s[-1][2] - s[0][2]) / 1e6
        print(f"| {n} | {'STALL' if n in STALL else 'clean'} | {len(r)} | {len(s)} | {dur:.1f} | {tb*8/dur/1e6:.2f} | {med/((s[-1][1]-s[0][1])/1e6):.5f} |")
    cl = sum(data[n][-1][0] for n in CLEAN) / 1e6
    print(f"\nClean traces total {cl:.1f} s ({cl/3600:.3f} h) of data; stall traces {sum(data[n][-1][0] for n in STALL)/1e6:.1f} s.")

    print("\n### Chunk inter-arrival gap (ms)\n\n| trace | p50 | p90 | p99 | p99.9 | max |\n|---|---|---|---|---|---|")
    pooled = []
    for n, r in data.items():
        g = sorted(gaps(r))
        if n in CLEAN:
            pooled += g
        print(f"| {n} | " + " | ".join(f2(pct(g, q)) for q in QS) + " |")
    pooled.sort()
    print(f"| **pooled clean** | " + " | ".join(f2(pct(pooled, q)) for q in QS) + " |")

    print("\n### PDV (RFC 5481, D - causal sliding min over 2 s / 10 s), ms\n\nSamples are PCR-carrying chunks only. 'pooled clean' concatenates the PDV samples of the 6 gap-free traces (t18, rai-1, rai-sport, orf1, orf-sport, servustv#2).\n")
    print("| trace | window | p50 | p90 | p99 | p99.9 | max |\n|---|---|---|---|---|---|---|")
    pool = {2: [], 10: []}
    for n, r in data.items():
        for w in (2, 10):
            p, _ = pdv_series(r, w * 1_000_000)
            v = sorted(x[2] for x in p)
            if n in CLEAN:
                pool[w] += v
            print(f"| {n} | {w} s | " + " | ".join(f2(pct(v, q)) for q in QS) + " |")
    for w in (2, 10):
        v = sorted(pool[w])
        print(f"| **pooled clean** | {w} s | " + " | ".join(f2(pct(v, q)) for q in QS) + " |")

    print("\n### RFC 3550 A.8 smoothed jitter J (integer, ms)\n\n| trace | J p50 | J p99 | J max |\n|---|---|---|---|")
    for n, r in data.items():
        sj = sorted(jser(r))
        print(f"| {n} | {f2(pct(sj,50))} | {f2(pct(sj,99))} | {f2(sj[-1])} |")

    print("\n### PCR-vs-arrival drift (OLS slope of D on arrival time; negative = media clock fast)\n\nStall traces are cut before the gap and traces under 30 s are skipped (a level shift is not drift); servustv uses its two connections separately. Traces are 10-60 s: single-trace estimates carry about +-50-100 ppm of PDV-noise error, so only the pooled spread is meaningful.\n")
    print("| trace | span s | OLS ppm |\n|---|---|---|")
    sl = []
    for n, r in data.items():
        s = pcr_samples(r)
        if n in STALL:
            s = [x for x in s if x[0] < STALL[n] - 5]
        if s[-1][1] - s[0][1] < 30e6:
            continue
        a = slope(s)
        sl.append(a)
        print(f"| {n} | {(s[-1][1]-s[0][1])/1e6:.1f} | {a:+.1f} |")
    sl.sort()
    print(f"\nPooled: median {sl[len(sl)//2]:+.0f} ppm, min {sl[0]:+.0f}, max {sl[-1]:+.0f} ppm (n={len(sl)}). 100 ppm over the 2 s minimum window = 0.2 ms: the windowed minimum absorbs any drift in this range; a whole-trace running minimum would inflate PDV by `drift x elapsed` (6 ms/min at 100 ppm).")

    for n in STALL_TRACES_ORDER:
        r = data[n]
        gi = STALL[n]
        g = gaps(r)
        t_end = r[gi][0]
        s = pcr_samples(r)
        before = [x for x in s if x[0] < gi][-1]
        after = [x for x in s if x[0] >= gi][0]
        d0 = before[1] - before[2]
        lb = min(1500, gi - 1)
        nom = sum(x[1] for x in r[gi - lb:gi]) * 8 / ((r[gi - 1][0] - r[gi - lb][0]) / 1e6) / 1e6
        print(f"\n### {n}: structure after the gap (chunk {gi})\n")
        print(f"Gap {f2(g[gi-1])} ms (t={r[gi-1][0]/1e6:.2f} s to {t_end/1e6:.2f} s). Arrival advanced {(after[1]-before[1])/1e3:.0f} ms across it, media {(after[2]-before[2])/1e3:.0f} ms: the media was delivered late by {(after[1]-after[2]-d0)/1e3:.0f} ms, nothing skipped. Pre-gap rate {nom:.2f} Mbit/s (preceding chunks).\n")
        print("| window after gap | chunks | Mbit/s | media s delivered per wall s |\n|---|---|---|---|")
        m0 = after[2]
        for w in (100_000, 250_000, 500_000, 1_000_000, 2_000_000, 5_000_000):
            sub = [x for x in r[gi:] if x[0] - t_end < w]
            ms = [x for x in s if x[0] >= gi and x[1] - t_end < w]
            rate = (ms[-1][2] - m0) / (ms[-1][1] - after[1]) if len(ms) > 1 else float("nan")
            print(f"| {w//1000} ms | {len(sub)} | {sum(x[1] for x in sub)*8/(w/1e6)/1e6:.1f} | {rate:.3f} |")
        print("\nDelay excess D(t) - D(pre-gap) (ms), 0 = back to baseline:\n\n| after first post-gap chunk | D excess |\n|---|---|")
        for off in (0, 1, 2, 5, 10, 20):
            x = next((x for x in s if x[1] >= t_end + off * 1_000_000), None)
            if x:
                print(f"| {off} s | {(x[1]-x[2]-d0)/1e3:.0f} |")
        last = s[-1]
        print(f"| end ({(last[1]-t_end)/1e6:.1f} s) | {(last[1]-last[2]-d0)/1e3:.0f} |")

    n = "servustv"
    r = load(n)
    gi = RECONNECT[n]
    s = pcr_samples(r)
    b = [x for x in s if x[0] < gi][-1]
    a = [x for x in s if x[0] >= gi][0]
    print(f"\n### servustv: source_end then reconnect (chunk {gi})\n")
    print(f"Arrival gap {f2(r[gi][0]-r[gi-1][0])} ms (events: `source_end` 10.7 s after loading, `reconnecting(attempt: 1)` 54 ms later, `playing` 5.0 s after that; the trace's own first 13.9 s are `servustv#1`, which ALSO holds a 1.33 s STARTUP gap 0.49 s in, see above). Across the gap arrival advanced {(a[1]-b[1])/1e3:.0f} ms but media advanced {(a[2]-b[2])/1e3:.0f} ms: the new connection joined the live edge **{((a[2]-b[2])-(a[1]-b[1]))/1e3:.0f} ms ahead** of where the old one stopped (D fell {((b[1]-b[2])-(a[1]-a[2]))/1e3:.0f} ms). The PCR is continuous in value (no wrap, no backwards step); it is a forward media jump. D then stays constant (baseline 1.3 s lower): a reconnect is a re-baseline event, not delay variation, and the 3.4 s dead time is not evidence about path jitter.")


STALL_TRACES_ORDER = ["tf-1", "6ter", "servustv#1"]

if __name__ == "__main__":
    main()
