"""Independent integer oracle for the `live-buffer` Rust golden tests.

A second implementation of the DESIGN.md algorithms (PDV, per-interval maxima,
Q15 forgetting histogram, stall max-hold, policy, state machine, decision
contract), written from the design text and NOT from the Rust source, in the
tick-driven protocol the crate uses: a virtual renderer ticked every
TICK_US (5 ms) plus one extra tick at every chunk arrival. The Rust test
`tests/real_traces.rs` replays the same traces and asserts the numbers this
script prints (they are pasted into that file's comments with this path).

    python3 docs/live-buffer/analysis/golden_oracle.py [--dir <traces>] [--initial-ms N]

Output, one block per trace: start time, the list of pauses (reason, pause
time, resume time), target min/max/final and the sum of the target over all
PCR chunks (a strong fingerprint of the whole target series). Times are
microseconds on the trace clock rebased to the first chunk. The playback
(PTS) domain is the unrolled PCR minus a constant, which cancels, so the
oracle works on the unrolled PCR directly (the Rust test additionally proves
the invariance under a constant offset).
"""
import csv
import os
import sys

sys.path.insert(0, os.path.dirname(__file__))
from common import ALL, pcr_signed_distance, trace_dir  # noqa: E402

TICKS_PER_US = 27
TICK_US = 5_000
GAP_RECONNECT_US = 2_000_000

CFG = dict(
    min_window=2_000_000, interval=500_000, outlier=500_000, bucket=10_000, buckets=400,
    forget=32113, quantile=990, hold=60_000_000, decay=60_000_000, disc=1_000_000,
    min=300_000, initial=1_800_000, max=3_000_000, k1=512, margin=100_000, k2=307,
    after=384, step=100_000, starve=20_000, nodata=200_000,
)


def load(name, d):
    rows = []
    with open(os.path.join(d, name + ".csv")) as f:
        for r in csv.DictReader(f):
            rows.append((int(r["arrival_ns"]) // 1000, int(r["pcr_27mhz"]) if r["pcr_27mhz"] else None))
    t0 = rows[0][0]
    return [(a - t0, p) for a, p in rows]


class Jitter:
    def __init__(self, c):
        self.c = c
        self.peak, self.peak_at = 0, None
        self.reset_baseline()

    def reset_baseline(self):
        self.win = []                      # (t, d) monotonic: strictly increasing d
        self.last_t = None
        self.recovering = False
        self.w = [0] * self.c["buckets"]
        self.k = 0
        self.base = 0
        self.istart = self.imax = None
        self.origin = None
        self.prev_pcr = None
        self.unrolled = 0

    def stall_peak(self, now):
        if self.peak_at is None:
            return 0
        age = now - self.peak_at
        h, dec = self.c["hold"], self.c["decay"]
        if age <= h:
            return self.peak
        x = age - h
        return 0 if x >= dec else self.peak * (dec - x) // dec

    def feed(self, arrival, pcr):
        """Returns 'disc' on a discontinuity (baseline reset, sample kept as new origin) else None."""
        c = self.c
        out = None
        if self.prev_pcr == pcr:
            return "dup"
        if self.prev_pcr is not None:
            md = pcr_signed_distance(self.prev_pcr, pcr) // TICKS_PER_US if pcr_signed_distance(self.prev_pcr, pcr) >= 0 \
                else -((-pcr_signed_distance(self.prev_pcr, pcr)) // TICKS_PER_US)   # truncation toward zero
            ad = arrival - self.last_arrival
            if md < -c["disc"] or md - ad > c["disc"]:
                self.reset_baseline()
                out = "disc"
        if self.prev_pcr is not None:
            self.unrolled += pcr_signed_distance(self.prev_pcr, pcr)
        else:
            self.unrolled = 0
        m = self.unrolled // TICKS_PER_US                       # floor
        if self.origin is None:
            self.origin = (arrival, m)
        self.prev_pcr, self.last_arrival = pcr, arrival
        d = (arrival - self.origin[0]) - (m - self.origin[1])
        t = arrival - self.origin[0]
        while self.win and self.win[-1][1] >= d:
            self.win.pop()
        self.win.append((t, d))
        # long-gap rule: a silence >= the window skips expiry for this one sample
        long_gap = self.last_t is not None and t - self.last_t >= c["min_window"]
        self.last_t = t
        while not long_gap and len(self.win) > 1 and self.win[0][0] + c["min_window"] <= t:
            self.win.pop(0)
        pdv = d - self.win[0][1]
        # max-hold (sample driven)
        if self.peak_at is None or pdv >= self.stall_peak(arrival):
            self.peak, self.peak_at = pdv, arrival
        # per-interval maximum
        if self.istart is None:
            self.istart = arrival
        elif arrival >= self.istart + c["interval"]:
            if self.imax is not None:
                if self.imax > c["outlier"]:
                    self.recovering = True          # stall interval: max-hold only
                elif self.recovering:
                    self.recovering = False         # aftermath (burst tail): skipped once
                else:
                    self.hist_add(self.imax)
            self.istart += ((arrival - self.istart) // c["interval"]) * c["interval"]
            self.imax = None
        self.imax = pdv if self.imax is None else max(self.imax, pdv)
        return out

    def hist_add(self, v):
        c = self.c
        f = min(c["forget"], (self.k << 15) // (self.k + 1))
        self.k += 1
        self.w = [(x * f) >> 15 for x in self.w]
        self.w[min(c["buckets"] - 1, v // c["bucket"])] += 32768 - f
        tot = sum(self.w)
        cum = 0
        for i, x in enumerate(self.w):
            cum += x
            if cum * 1000 >= tot * c["quantile"]:
                self.base = (i + 1) * c["bucket"]
                return
        self.base = c["buckets"] * c["bucket"]


class Sim:
    def __init__(self, c, tick_us=TICK_US):
        self.c = c
        self.j = Jitter(c)
        self.target, self.last_t = c["min"], None
        self.events = []                # (kind, now, ...)
        self.state, self.started, self.neutral = "buf", False, False
        self.media_end = self.qstart = self.fill_from = self.last_playback = None
        self.last_chunk = None
        self.pause_at = 0
        self.source_down = False
        self.running, self.clock, self.t_set = False, None, 0
        self.tick_sum, self.tmin, self.tmax = 0, None, 0
        self.tick_us = tick_us
        self.min_lead = None

    # ---- policy
    def clamp(self, v):
        return max(self.c["min"], min(self.c["max"], v))

    def retarget(self, now):
        c = self.c
        need = max(((self.j.base * c["k1"]) >> 8) + c["margin"], (self.j.stall_peak(now) * c["k2"]) >> 8)
        raw = self.clamp(need)
        dt = 0 if self.last_t is None else now - self.last_t
        self.target = max(raw, max(0, self.target - c["step"] * dt // 1_000_000))
        self.last_t = now
        self.tick_sum += self.target
        self.tmin = self.target if self.tmin is None else min(self.tmin, self.target)
        self.tmax = max(self.tmax, self.target)

    def exit_target(self, now):
        c = self.c
        if not self.started:
            return self.clamp(max(c["initial"], self.target))
        if self.neutral:
            return self.target
        return self.clamp(max(self.target, (self.j.stall_peak(now) * c["after"]) >> 8))

    # ---- renderer model
    def pos(self, now):
        return self.clock + (now - self.t_set) * TICKS_PER_US if self.running else self.clock

    def apply(self, ev, now):
        k = ev[0]
        if k in ("start", "resume"):
            self.clock, self.t_set, self.running = ev[2], now, True
        elif k == "pause":
            self.clock, self.t_set, self.running = ev[2], now, False

    def emit(self, now, *ev):
        self.events.append((ev[0], now) + ev[1:])
        self.apply((ev[0], now) + ev[1:], now)

    # ---- inputs
    def chunk(self, now, pcr):
        self.last_chunk = now
        if pcr is None:
            return
        r = self.j.feed(now, pcr)
        if r == "dup":
            return
        if r == "disc":
            self.rebaseline(now)
        self.retarget(now)

    def media(self, now, end):
        self.media_end = end
        if self.qstart is None:
            self.qstart = end

    def hint(self, now, kind):
        if kind == "ended":
            self.source_down = True
        elif kind == "reconnected":
            self.source_down = False
            self.j.reset_baseline()
            self.rebaseline(now)

    def rebaseline(self, now):
        old = self.media_end
        self.media_end = None
        self.qstart = None
        if self.state == "play":
            lp, hold = self.last_playback, old
            if lp is not None and old is not None and old - lp >= 0:
                hold = lp
            self.emit(now, "pause", hold, "discontinuity")
            self.pause_at = now
            self.state, self.neutral, self.fill_from = "buf", True, None
        elif self.state == "starved":
            self.state, self.neutral, self.fill_from = "buf", True, None
        else:
            self.fill_from = None
        self.events.append(("rebaselined", now))

    def tick(self, now):
        c = self.c
        pb = self.pos(now) if self.clock is not None else None
        if pb is not None:
            self.last_playback = pb
        if self.media_end is None:
            return
        if self.state == "play":
            lead = (self.media_end - pb) // TICKS_PER_US if self.media_end >= pb else -((pb - self.media_end) // TICKS_PER_US)
            self.min_lead = lead if self.min_lead is None else min(self.min_lead, lead)
            if lead <= c["starve"]:
                hold = self.media_end if lead < 0 else pb
                silent = self.last_chunk is None or now - self.last_chunk >= c["nodata"]
                reason = "nodata" if (self.source_down or silent) else "arriving"
                self.emit(now, "pause", hold, reason)
                self.pause_at = now
                self.state, self.neutral, self.fill_from = "starved", False, hold
            return
        origin = self.fill_from if self.fill_from is not None else self.qstart
        if origin is None:
            return
        d = self.media_end - origin
        lead = d // TICKS_PER_US if d >= 0 else -((-d) // TICKS_PER_US)
        if lead >= self.exit_target(now):
            kind = "resume" if self.started else "start"
            self.started = True
            self.emit(now, kind, origin, lead)
            self.state, self.fill_from, self.neutral = "play", None, False


def run(rows, initial, tick_us=TICK_US):
    c = dict(CFG, initial=initial)
    s = Sim(c, tick_us)
    unrolled, prev = 0, None
    nxt, last_arrival = 0, 0
    for i, (a, pcr) in enumerate(rows):
        if i > 0 and a - last_arrival >= GAP_RECONNECT_US:
            s.hint(last_arrival, "ended")
            s.hint(a, "reconnected")
        while nxt <= a:
            s.tick(nxt)
            nxt += tick_us
        s.chunk(a, pcr)
        if pcr is not None:
            unrolled = 0 if prev is None else unrolled + pcr_signed_distance(prev, pcr)
            prev = pcr
            s.media(a, unrolled)
        s.tick(a)
        last_arrival = a
    return s


def summarize(name, s):
    ev = s.events
    start = next((e[1] for e in ev if e[0] == "start"), None)
    pauses, open_p = [], None
    for e in ev:
        if e[0] == "pause":
            open_p = [e[3], e[1], None]
            pauses.append(open_p)
        elif e[0] == "resume" and open_p:
            open_p[2] = e[1]
            open_p = None
    print(f"{name}: start_us={start} rebaselined={sum(1 for e in ev if e[0] == 'rebaselined')}")
    for p in pauses:
        print(f"    pause reason={p[0]} at_us={p[1]} resume_us={p[2]}")
    print(f"    min_lead_while_playing_us={s.min_lead}")
    print(f"    target min={s.tmin} max={s.tmax} final={s.target} sum_over_pcr_chunks={s.tick_sum}")


def main():
    a = sys.argv[1:]
    ini = int(a[a.index("--initial-ms") + 1]) * 1000 if "--initial-ms" in a else CFG["initial"]
    d = trace_dir()
    print(f"# initial_us={ini} tick_us={TICK_US} dir={d}")
    for n in ALL:
        summarize(n, run(load(n, d), ini))


if __name__ == "__main__":
    main()
