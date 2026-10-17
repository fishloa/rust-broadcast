"""Playback-consumption model + policy comparison over all traces.

Model (integer microseconds): media arrived M(t) = media time of the last PCR chunk that
arrived at or before t (sample-and-hold; conservative by <= one PCR interval, ~35 ms).
lead = M(t) - P, P = playback position. Playing: P advances 1 s/s. Starved when
lead <= STARVE_US. The controller OWNS pause (zenith-tv measurement, DESIGN.md 2.4): on Starved
P freezes, playback resumes only when lead >= resume target, at the frozen P.

Policies compared:
  fixed(1 s/2 s)  zenith-tv today: start 1 s, resume 2 s
  flat 1.5 s      start = resume = 1.5 s
  adaptive        the proposal: start = initial, resume = max(target, after_rebuffer x depth),
                  target = clamp(min, max(k1*q99+margin, k2*stall_peak), max) with a slew-limited fall;
                  interval maxima above `outlier` (500 ms) feed ONLY the max-hold, never the histogram
                  (otherwise k1 doubles the stall: first version of this sim gave target 2.9 s)
"""
import os
import sys
sys.path.insert(0, os.path.dirname(__file__))
from collections import deque
from common import CLEAN, STALL, load, pcr_samples, pct

STARVE_US = 20_000
S = 1_000_000
GAP_TRACES = ["tf-1", "6ter", "servustv#1"]


class Adaptive:
    """Python mirror of the proposed estimator + policy (floats only for the histogram)."""
    def __init__(self, P):
        self.P = dict(min=300_000, initial=500_000, max=3 * S, k1=2.0, margin=100_000, k2=1.2, after=1.5,
                      q=0.99, forget=0.98, interval=500_000, window=2 * S, hold=60 * S, decay=60 * S,
                      step=100_000, bucket=10_000, nb=400, outlier=500_000)
        self.P.update(P)
        self.dq, self.hist, self.k = deque(), [0.0] * self.P["nb"], 0
        self.istart, self.imax = None, None
        self.peak, self.peak_t = 0, None
        self.base = 0
        self.target, self.last_t = self.P["min"], None

    def sample(self, a, d):
        P = self.P
        while self.dq and self.dq[-1][1] >= d:
            self.dq.pop()
        self.dq.append((a, d))
        while self.dq[0][0] <= a - P["window"]:
            self.dq.popleft()
        pdv = d - self.dq[0][1]
        if self.istart is None:
            self.istart = a
        while a >= self.istart + P["interval"]:
            if self.imax is not None and self.imax <= P["outlier"]:
                f = min(P["forget"], self.k / (self.k + 1.0))
                self.k += 1
                self.hist = [x * f for x in self.hist]
                self.hist[min(P["nb"] - 1, self.imax // P["bucket"])] += 1 - f
                tot, c = sum(self.hist), 0.0
                for i, x in enumerate(self.hist):
                    c += x
                    if c >= P["q"] * tot:
                        self.base = (i + 1) * P["bucket"]
                        break
            self.istart += P["interval"]
            self.imax = None
        self.imax = pdv if self.imax is None else max(self.imax, pdv)
        # max-hold is sample-driven (rises immediately, before the interval closes)
        if self.peak_t is None or pdv >= self.stall_peak(a):
            self.peak, self.peak_t = pdv, a
        return pdv

    def stall_peak(self, a):
        if self.peak_t is None:
            return 0
        age = a - self.peak_t
        h = self.P["hold"]
        return self.peak if age <= h else max(0, int(self.peak * (1 - (age - h) / self.P["decay"])))

    def retarget(self, a):
        P = self.P
        raw = max(P["k1"] * self.base + P["margin"], P["k2"] * self.stall_peak(a))
        raw = min(P["max"], max(P["min"], int(raw)))
        dt = 0 if self.last_t is None else a - self.last_t
        self.target = max(raw, self.target - P["step"] * dt // S)
        self.last_t = a
        return self.target

    def resume_target(self, a):
        P = self.P
        return min(P["max"], max(P["min"], int(max(self.target, P["after"] * self.stall_peak(a)))))


def simulate(rows, policy):
    """policy: ('fixed', start, resume) | ('adaptive', params[, seed_peak_us])"""
    s = pcr_samples(rows)
    ad = Adaptive(policy[1]) if policy[0] == "adaptive" else None
    if ad and len(policy) > 2 and policy[2]:
        ad.peak, ad.peak_t = policy[2], s[0][1]
        ad.retarget(s[0][1])
    P = s[0][2]
    state, tref, P0, zap, last_m = "buf", None, None, None, s[0][2]
    stalls, tgt_log = [], []
    rp = ad.P.get("reprime_frac") if ad else None
    below = None
    for _, a, m in s:
        if state == "play" and rp:
            lead = m - (P0 + (a - tref))
            if lead < rp * ad.target:
                below = a if below is None else below
                if a - below >= 2 * S:
                    P = P0 + (a - tref)
                    state, below = "buf", None
                    stalls.append([a, None, "reprime"])
            else:
                below = None
        if state == "play":
            ts = tref + (last_m - STARVE_US - P0)
            if ts < a:
                below = None
                P = last_m - STARVE_US
                state = "buf"
                stalls.append([ts, None])
        last_m = m
        if ad:
            ad.sample(a, a - m)
            tgt_log.append((a, ad.retarget(a)))
        if state == "buf":
            if policy[0] == "fixed":
                need = policy[1] if zap is None else policy[2]
            elif zap is None:
                need = max(ad.P["initial"], ad.target if ad.peak_t is not None else 0)
            elif stalls and len(stalls[-1]) > 2:
                need = ad.target
            else:
                need = ad.resume_target(a)
            if m - P >= need:
                state, tref, P0 = "play", a, P
                if zap is None:
                    zap = a - s[0][1]
                else:
                    stalls[-1][1] = a
    return zap, stalls, tgt_log


def fz(st):
    return ", ".join(f"{(x[1]-x[0])/1e3:.0f} @{x[0]/1e6:.1f}s" if x[1] else "unresolved" for x in st) or "-"


def synth(gap_every_s, dur_s=1800, gap_us=1_430_000):
    """Synthetic lossy link: PCR every 35 ms, 0-15 ms deterministic jitter, a 1.43 s dead time every
    gap_every_s (arrival shifted permanently, as in all three real events: no catch-up)."""
    rows, seed, shift, nxt = [], 12345, 0, gap_every_s * S
    for k in range(int(dur_s * S // 35_000)):
        m = k * 35_000
        if m >= nxt:
            shift += gap_us
            nxt += gap_every_s * S
        seed = (seed * 1103515245 + 12345) & 0x7FFFFFFF
        rows.append((m + shift + (seed >> 8) % 15_000, 1316, m))
    return rows


def main():
    pol = {"fixed 1 s start / 2 s resume": ("fixed", 1 * S, 2 * S),
           "flat 1.5 s": ("fixed", 1500_000, 1500_000),
           "adaptive (initial 0.5 s)": ("adaptive", {}),
           "adaptive (initial 1.5 s)": ("adaptive", {"initial": 1500_000}),
           "adaptive + seed peak 1.45 s": ("adaptive", {}, 1_450_000)}
    print("### Smallest start target with zero stalls (5 ms grid, 20 ms starve threshold)\n\n| trace | min zero-stall start (ms) | zap at that target (ms) |\n|---|---|---|")
    for n in ["tf-1", "6ter"]:
        rows = load(n)
        x = next(x for x in range(0, 4001, 5) if not simulate(rows, ("fixed", x * 1000, x * 1000))[1])
        print(f"| {n} | {x} | {simulate(rows, ('fixed', x*1000, x*1000))[0]/1e3:.0f} |")
    print("\nservustv#1 is a STARTUP gap: the player is still Buffering when it hits, so it cannot starve; it only delays Playing (zap table below).")
    print("\n### Policy comparison on the three gap traces (zap = first PCR chunk to Playing; freeze = length of each pause)\n\n| trace | policy | zap ms | stalls | freeze ms |\n|---|---|---|---|---|")
    for n in GAP_TRACES:
        rows = load(n)
        for lab, p in pol.items():
            z, st, _ = simulate(rows, p)
            print(f"| {n} | {lab} | {z/1e3:.0f} | {len(st)} | {fz(st)} |")
    print("\n### Start target vs (zap ms / number of pauses) on every trace, resume target 2 s\n\n| start target | " + " | ".join(GAP_TRACES + CLEAN) + " |\n|---|" + "---|" * 9)
    for ini in (300, 500, 1000, 1500, 2000):
        cells = []
        for n in GAP_TRACES + CLEAN:
            z, st, _ = simulate(load(n), ("fixed", ini * 1000, 2 * S))
            cells.append(f"{z/1e3:.0f} / {len(st)}")
        print(f"| {ini} | " + " | ".join(cells) + " |")

    print("\n### False starvation and target distribution of the ADAPTIVE controller (initial 0.5 s) on the gap-free traces\n")
    print("| trace | duration s | false pauses | target p50 ms | p99 | max | % time at min (300 ms) | zap ms |\n|---|---|---|---|---|---|---|---|")
    tot_t, tot_st, allt = 0, 0, []
    for n in CLEAN:
        rows = load(n)
        z, st, tl = simulate(rows, ("adaptive", {}))
        d = rows[-1][0] / 1e6
        tot_t += d
        tot_st += len(st)
        v = sorted(x[1] for x in tl)
        allt += v
        print(f"| {n} | {d:.1f} | {len(st)} | {v[len(v)//2]/1e3:.0f} | {pct(v,99)/1e3:.0f} | {v[-1]/1e3:.0f} | {100*sum(1 for x in v if x==300_000)/len(v):.1f} | {z/1e3:.0f} |")
    allt.sort()
    print(f"| **pooled** | {tot_t:.1f} | {tot_st} | {allt[len(allt)//2]/1e3:.0f} | {pct(allt,99)/1e3:.0f} | {allt[-1]/1e3:.0f} | {100*sum(1 for x in allt if x==300_000)/len(allt):.1f} | |")
    print(f"\nFalse starvations: {tot_st} in {tot_t/3600:.3f} h of gap-free data = {tot_st/(tot_t/3600):.1f}/h observed. With zero events the rule-of-three 95% upper bound is {3/(tot_t/3600):.0f}/h: the data cannot show a lower rate than that, so the synthetic clean-jitter scenarios must carry the rest.")

    print("\n### Adaptive target over time on the gap traces (ms)\n\n| trace | before gap | just after gap end | +5 s | +20 s | end of trace | max |\n|---|---|---|---|---|---|---|")
    for n in ["tf-1", "6ter"]:
        rows = load(n)
        _, st, tl = simulate(rows, ("adaptive", {}))
        ge = rows[STALL[n]][0]
        at = lambda t: [v for a, v in tl if a <= t][-1]
        print(f"| {n} | {at(ge-200_000)/1e3:.0f} | {at(ge+300_000)/1e3:.0f} | {at(ge+5*S)/1e3:.0f} | {at(min(ge+20*S, rows[-1][0]))/1e3:.0f} | {tl[-1][1]/1e3:.0f} | {max(v for _,v in tl)/1e3:.0f} |")

    print("\n### Repeat-gap test (6ter with a second identical 1.43 s late-shift injected 20 s after the first gap)\n")
    rows = load("6ter")
    gi = STALL["6ter"]
    t2 = rows[gi][0] + 20 * S
    k = next(i for i, r in enumerate(rows) if r[0] >= t2)
    rows2 = rows[:k] + [(a + 1_430_000, b, m) for a, b, m in rows[k:]]
    print("| policy | zap ms | pauses | freeze ms (each) |\n|---|---|---|---|")
    for lab, p in pol.items():
        z, st, _ = simulate(rows2, p)
        print(f"| {lab} | {z/1e3:.0f} | {len(st)} | {fz(st)} |")


    print("\n### Synthetic lossy link (30 min, one 1.43 s no-catch-up gap every N s; N=180 s is the pooled real rate, 3 events in 471 s of gap-class traces is ~20/h with n=3)\n")
    pol2 = dict(pol)
    pol2["adaptive (0.5 s) + re-prime at <50% of target for 2 s"] = ("adaptive", {"reprime_frac": 0.5})
    pol2["adaptive (1.5 s) + re-prime"] = ("adaptive", {"initial": 1500_000, "reprime_frac": 0.5})
    print("| gap every | policy | zap ms | pauses (stall + re-prime) | frozen s per 30 min | frozen % |\n|---|---|---|---|---|---|")
    for N in (180, 600):
        rows = synth(N)
        for lab, p in pol2.items():
            z, st, _ = simulate(rows, p)
            fr = sum((x[1] - x[0]) for x in st if x[1]) / 1e6
            rep = sum(1 for x in st if len(x) > 2)
            print(f"| {N} s | {lab} | {z/1e3:.0f} | {len(st)} ({len(st)-rep}+{rep}) | {fr:.1f} | {100*fr/1800:.2f} |")


if __name__ == "__main__":
    main()
