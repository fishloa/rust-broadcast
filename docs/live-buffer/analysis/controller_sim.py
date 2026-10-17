"""Playback-consumption model over the traces: zap time and starvation per target.

Model (all integer microseconds): media arrived M(t) = media time of the last
PCR chunk that arrived at or before t (sample-and-hold; under-estimates by at
most one PCR interval, ~35 ms, so results are slightly conservative).
buffered = M(t) - P where P is the playback position. Playing: P advances 1 s/s.
Starved when buffered <= STARVE_US. The CONTROLLER OWNS PAUSE (zenith-tv
measurement, DESIGN.md s7): on Starved the caller pauses, P freezes, and
playback resumes only when buffered >= resume target; the resume position is the
frozen P (no clock running ahead of the media).
"""
import os
import sys
sys.path.insert(0, os.path.dirname(__file__))
from common import load, pcr_samples

STARVE_US = 20_000  # Apple-side starved threshold quoted by zenith-tv (0.02 s)


def simulate(rows, initial, after, th=STARVE_US):
    s = pcr_samples(rows)
    P = s[0][2]  # start playback at the first media time
    state, tplay_ref, P0 = "buf", None, None
    events, zap, stalls = [], None, []
    minbuf, last_m = None, s[0][2]
    stall_start = None
    for k, (_, a, m) in enumerate(s):
        # Playing: did we starve before this sample arrived?
        if state == "play":
            t_starve = tplay_ref + (last_m - th - P0)
            if t_starve < a:
                P = last_m - th
                state, stall_start = "buf", t_starve
                stalls.append([t_starve, None, a - t_starve])
        last_m = m
        if state == "play":
            minbuf = (m - (P0 + (a - tplay_ref))) if minbuf is None else min(minbuf, m - (P0 + (a - tplay_ref)))
        else:
            need = initial if zap is None else after
            if m - P >= need:
                state, tplay_ref, P0 = "play", a, P
                if zap is None:
                    zap = a - s[0][1]
                else:
                    stalls[-1][1] = a
    return zap, stalls, minbuf


def main():
    print("### State machine on the real traces (start = first byte, resume target = after-rebuffer)\n")
    for name in ("tf-1", "t18"):
        rows = load(name)
        print(f"\n#### {name}\n")
        print("| initial target | after-rebuffer target | zap time to Playing (ms) | stalls | first stall at (s) | freeze (pause) duration (ms) | min buffer while playing (ms) |\n|---|---|---|---|---|---|---|")
        for ini in (250, 500, 1000, 1400, 1500, 2000, 3000):
            for aft in ((1000, 2000, 2100) if name == "tf-1" and ini in (500, 1000) else (None,)):
                zap, st, mb = simulate(rows, ini * 1000, (aft or ini) * 1000)
                fz = ", ".join(f"{(x[1]-x[0])/1e3:.0f}" if x[1] else "unresolved" for x in st) or "-"
                at = f"{st[0][0]/1e6:.2f}" if st else "-"
                print(f"| {ini} | {aft or ini} | {zap/1e3:.0f} | {len(st)} | {at} | {fz} | {mb/1e3:.0f} |")
    # analytic minimum: smallest initial target with zero stalls
    for name in ("tf-1", "t18"):
        rows = load(name)
        lo = next(x for x in range(0, 4001, 5) if not simulate(rows, x * 1000, x * 1000)[1])
        print(f"\nSmallest initial target (5 ms grid, {STARVE_US//1000} ms starve threshold) with ZERO stalls over {name}: **{lo} ms**.")
    rows = load("tf-1")
    z, st, mb = simulate(rows, 1_500_000, 1_500_000)
    print(f"\nResidual buffer after surviving the gap at initial=1500 ms: min {mb/1e3:.0f} ms, and it is permanent (media is paced live, D never recovers): the next RTO-class gap stalls. With initial 2000/3000:")
    for ini in (2000, 3000):
        _, _, mb = simulate(rows, ini * 1000, ini * 1000)
        print(f"- initial {ini} ms -> min buffer {mb/1e3:.0f} ms after the gap; a SECOND 1.4 s gap would stall: {mb < 1_400_000}")


if __name__ == "__main__":
    main()
