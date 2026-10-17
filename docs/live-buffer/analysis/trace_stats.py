"""Chunk-gap, PDV, RFC 3550 J, drift and burst statistics -> markdown tables."""
import sys
sys.path.insert(0, __import__("os").path.dirname(__file__))
from common import load, pcr_samples, pct, pdv_series, trace_dir

QS = [50, 90, 99, 99.9, 100]


def fmt(us):
    return f"{us/1000:.2f}"


def main():
    names = ["t18", "tf-1"]
    data = {n: load(n) for n in names}
    print("### Trace summary\n\n| trace | chunks | PCR chunks | duration s | bytes | mean Mbit/s | PCR media s | media/wall |\n|---|---|---|---|---|---|---|---|")
    for n, r in data.items():
        s = pcr_samples(r)
        dur = r[-1][0] / 1e6
        tb = sum(x[1] for x in r)
        med = (s[-1][2] - s[0][2]) / 1e6
        print(f"| {n} | {len(r)} | {len(s)} | {dur:.2f} | {tb} | {tb*8/dur/1e6:.2f} | {med:.2f} | {med/((s[-1][1]-s[0][1])/1e6):.6f} |")

    print("\n### Chunk inter-arrival gap (ms)\n\n| trace | " + " | ".join(f"p{q}" for q in QS[:-1]) + " | max | chunk idx of max |\n|---|" + "---|" * 5)
    for n, r in data.items():
        g = [r[i][0] - r[i - 1][0] for i in range(1, len(r))]
        sg = sorted(g)
        mi = g.index(max(g)) + 1
        print(f"| {n} | " + " | ".join(fmt(pct(sg, q)) for q in QS[:-1]) + f" | {fmt(sg[-1])} | {mi} |")
    print("\n### PCR-chunk inter-arrival (ms): the sample spacing the estimators see\n\n| trace | p50 | p99 | max |\n|---|---|---|---|")
    for n, r in data.items():
        s = pcr_samples(r)
        g = sorted(s[i][1] - s[i - 1][1] for i in range(1, len(s)))
        print(f"| {n} | {fmt(pct(g,50))} | {fmt(pct(g,99))} | {fmt(g[-1])} |")

    print("\n### PDV (RFC 5481: D - Dmin) over PCR chunks, ms\n")
    print("D = arrival_us - media_us per PCR-carrying chunk (chunks without PCR carry no media time and are skipped; the media time of a PCR chunk is that chunk's last PCR, a stair-case approximation good to one PCR interval, see above). Dmin = causal sliding minimum over the stated window; 'run-min' is the whole-trace running minimum (= zenith-tv's `lateness`).\n")
    print("| trace | Dmin window | p50 | p90 | p99 | p99.9 | max |\n|---|---|---|---|---|---|---|")
    for n, r in data.items():
        for lab, w in (("2 s", 2_000_000), ("10 s", 10_000_000), ("run-min", None)):
            p, _ = pdv_series(r, w)
            v = sorted(x[2] for x in p)
            print(f"| {n} | {lab} | " + " | ".join(fmt(pct(v, q)) for q in QS) + " |")

    print("\n### RFC 3550 A.8 smoothed jitter J (integer form J += (|D|-J)/16, D = difference of successive relative transits), ms\n")
    print("| trace | J p50 | J p99 | J max | J at end | max PDV(10 s) for scale |\n|---|---|---|---|---|---|")
    for n, r in data.items():
        s = pcr_samples(r)
        j, js = 0, []
        for k in range(1, len(s)):
            d = (s[k][1] - s[k][2]) - (s[k - 1][1] - s[k - 1][2])
            j += (abs(d) - j) // 16  # integer, truncating like the RFC's C code
            js.append(j)
        sj = sorted(js)
        p10, _ = pdv_series(r, 10_000_000)
        print(f"| {n} | {fmt(pct(sj,50))} | {fmt(pct(sj,99))} | {fmt(sj[-1])} | {fmt(js[-1])} | {fmt(max(x[2] for x in p10))} |")

    print("\n### PCR-vs-arrival drift (OLS slope of D=arrival-media on arrival time, unrolled PCR)\n")
    print("Sign: negative = the encoder (media) clock runs FAST relative to the TV clock. tf-1 is cut at the gap (t < 45.9 s) because a level shift is not drift. Block slopes show the estimator noise: 15 ms of PDV over a 15 s block is 1000 ppm.\n")
    print("| trace | OLS ppm (clean span) | span s | 15 s block slopes ppm | drift over span ms |\n|---|---|---|---|---|")
    for n, r in data.items():
        s = pcr_samples(r)
        if n == "tf-1":
            s = [x for x in s if x[1] < 45_900_000]
        def slope(sub):
            xs = [a for _, a, _ in sub]
            ys = [a - m for _, a, m in sub]
            mx, my = sum(xs) / len(xs), sum(ys) / len(ys)
            return sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / sum((x - mx) ** 2 for x in xs) * 1e6
        blocks = [slope([x for x in s if k * 15e6 <= x[1] < (k + 1) * 15e6]) for k in range(int(s[-1][1] // 15e6))]
        a = slope(s)
        span = (s[-1][1] - s[0][1]) / 1e6
        print(f"| {n} | {a:+.1f} | {span:.1f} | " + ", ".join(f"{b:+.0f}" for b in blocks) + f" | {a*span/1e3:+.2f} |")
    for n, r in data.items():
        s = pcr_samples(r)
        if n == "t18":
            e = [(a - m) for _, a, m in s]
            print(f"\nt18 D range over 59 s: min {min(e)/1e3:.2f} ms .. max {max(e)/1e3:.2f} ms; zenith-tv's own figure +33 ppm = {33e-6*59e3:.1f} ms over 59 s, i.e. below the {max(e)/1e3-min(e)/1e3:.0f} ms PDV band: invisible to a 2-10 s windowed minimum.")

    r = data["tf-1"]
    g = [r[i][0] - r[i - 1][0] for i in range(1, len(r))]
    gi = g.index(max(g)) + 1
    t_gap_end = r[gi][0]
    print(f"\n### tf-1 burst structure around the gap (chunk {gi})\n")
    print(f"Gap = {fmt(g[gi-1])} ms between chunk {gi-1} (t={r[gi-1][0]/1e6:.3f} s) and chunk {gi} (t={t_gap_end/1e6:.3f} s). Nominal rate before the gap: {sum(x[1] for x in r[gi-2000:gi])*8/((r[gi-1][0]-r[gi-2000][0])/1e6)/1e6:.2f} Mbit/s over the preceding 2000 chunks.\n")
    print("| window after gap | chunks | bytes | Mbit/s | x nominal |\n|---|---|---|---|---|")
    nom = sum(x[1] for x in r[gi - 2000:gi]) * 8 / ((r[gi - 1][0] - r[gi - 2000][0]) / 1e6)
    for w in (10_000, 50_000, 100_000, 250_000, 500_000, 1_000_000):
        sub = [x for x in r[gi:] if x[0] - t_gap_end < w]
        b = sum(x[1] for x in sub)
        print(f"| {w//1000} ms | {len(sub)} | {b} | {b*8/(w/1e6)/1e6:.1f} | {b*8/(w/1e6)/nom:.1f} |")
    s = pcr_samples(r)
    before = [x for x in s if x[0] < gi][-1]
    after = [x for x in s if x[0] >= gi][0]
    d0 = before[1] - before[2]
    print(f"\nAcross the gap the arrival clock advanced {(after[1]-before[1])/1e3:.1f} ms but media advanced only {(after[2]-before[2])/1e3:.1f} ms: nothing was skipped, the media was delivered LATE by {(after[1]-after[2]-d0)/1e3:.0f} ms.\n")
    print("Excess delay D(t) - D(pre-gap) after the gap (ms): a catch-up burst would bring this back to 0; a live-paced sender leaves it constant.\n")
    print("| t after first post-gap chunk | D excess ms |\n|---|---|")
    for off in (0, 500_000, 1_000_000, 2_000_000, 5_000_000, 10_000_000):
        x = next((x for x in s if x[1] >= t_gap_end + off), None)
        if x:
            print(f"| {off/1e6:.1f} s | {(x[1]-x[2]-d0)/1e3:.0f} |")
    print("\nThe 1.4x-nominal byte rate in the table above is VBR content, not catch-up: the media-time delivery rate after the gap is 1.00x (D constant to within 35 ms). **The stall permanently removes ~1.4 s of buffer; the sender never refills it.**")

if __name__ == "__main__":
    main()
