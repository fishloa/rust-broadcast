# live-buffer: trace analysis

Evidence base for the defaults in [`DESIGN.md`](DESIGN.md). Everything below is produced by the deterministic stdlib-python scripts in [`analysis/`](analysis/) (`trace_stats.py`, `estimators.py`, `controller_sim.py`; shared loaders in `common.py`), run against the PRIVATE traces `private/fixtures/live-buffer/{tf-1,t18}.csv` (zenith-tv, Apple TV 4K, live HTTP-TS; provenance in that directory's `PROVENANCE.md`; the traces are not redistributable and are never copied into this public repo, only derived numbers are). Re-run: `python3 docs/live-buffer/analysis/<script>.py [--dir <traces>]`; output is byte-stable. This file is the pasted output plus findings; regenerate the tables by re-running the scripts.

**Methods.** Arrival time = `arrival_ns/1000` (integer us, rebased). Media time = `pcr_27mhz/27` us, unrolled with the `clock33::pcr_signed_distance` half-range rule. Only chunks carrying a PCR (1689 of 19175 in tf-1, 1702 of 86326 in t18, one every ~35 ms) have a media time. PDV samples are taken at those chunks only (chunks without a PCR are skipped, not interpolated), and the playback model holds the last PCR's media time until the next one (sample-and-hold: it under-estimates arrived media by at most one PCR interval, so buffer figures are conservative by <= ~35 ms). PDV is the RFC 5481 relative-transit form `D = arrival - media` minus a causal sliding minimum of D (2 s and 10 s windows; "run-min" is the whole-trace running minimum, which is what zenith-tv's `measure-delivery-jitter` calls lateness). Analysis scripts use floats where convenient; the crate uses the integer forms of DESIGN.md section 3.

**Single-event limitation (read this first).** tf-1 contains exactly ONE stall; t18 contains none. Every number about stall behaviour is an anecdote of n=1: it proves that an RTO-class gap exists and gives a lower bound for the stall memory, not a distribution. Defaults derived from it are tagged accordingly; the test plan (DESIGN.md section 5) adds synthetic traces and DESIGN.md section 8 requests the other six channels.

## Tables

### Trace summary

| trace | chunks | PCR chunks | duration s | bytes | mean Mbit/s | PCR media s | media/wall |
|---|---|---|---|---|---|---|---|
| t18 | 86326 | 1702 | 59.10 | 23622952 | 3.20 | 59.10 | 1.000148 |
| tf-1 | 19175 | 1689 | 59.93 | 38328876 | 5.12 | 58.58 | 0.977465 |

### Chunk inter-arrival gap (ms)

| trace | p50 | p90 | p99 | p99.9 | max | chunk idx of max |
|---|---|---|---|---|---|
| t18 | 0.12 | 0.42 | 14.96 | 16.00 | 24.43 | 73220 |
| tf-1 | 1.51 | 7.75 | 34.89 | 36.35 | 1341.63 | 17897 |

### PCR-chunk inter-arrival (ms): the sample spacing the estimators see

| trace | p50 | p99 | max |
|---|---|---|---|
| t18 | 35.58 | 40.09 | 46.83 |
| tf-1 | 35.14 | 41.17 | 1433.46 |

### PDV (RFC 5481: D - Dmin) over PCR chunks, ms

D = arrival_us - media_us per PCR-carrying chunk (chunks without PCR carry no media time and are skipped; the media time of a PCR chunk is that chunk's last PCR, a stair-case approximation good to one PCR interval, see above). Dmin = causal sliding minimum over the stated window; 'run-min' is the whole-trace running minimum (= zenith-tv's `lateness`).

| trace | Dmin window | p50 | p90 | p99 | p99.9 | max |
|---|---|---|---|---|---|---|
| t18 | 2 s | 1.77 | 9.68 | 14.02 | 15.33 | 16.03 |
| t18 | 10 s | 4.72 | 11.23 | 15.11 | 17.46 | 19.98 |
| t18 | run-min | 9.69 | 12.37 | 15.36 | 20.18 | 20.20 |
| tf-1 | 2 s | 1.35 | 5.33 | 1386.96 | 1400.73 | 1401.73 |
| tf-1 | 10 s | 2.19 | 1356.10 | 1391.14 | 1401.45 | 1402.45 |
| tf-1 | run-min | 2.93 | 1357.35 | 1391.91 | 1402.22 | 1403.22 |

### RFC 3550 A.8 smoothed jitter J (integer form J += (|D|-J)/16, D = difference of successive relative transits), ms

| trace | J p50 | J p99 | J max | J at end | max PDV(10 s) for scale |
|---|---|---|---|---|---|
| t18 | 1.30 | 3.37 | 3.69 | 1.98 | 19.98 |
| tf-1 | 1.40 | 32.54 | 89.58 | 0.41 | 1402.45 |

### PCR-vs-arrival drift (OLS slope of D=arrival-media on arrival time, unrolled PCR)

Sign: negative = the encoder (media) clock runs FAST relative to the TV clock. tf-1 is cut at the gap (t < 45.9 s) because a level shift is not drift. Block slopes show the estimator noise: 15 ms of PDV over a 15 s block is 1000 ppm.

| trace | OLS ppm (clean span) | span s | 15 s block slopes ppm | drift over span ms |
|---|---|---|---|---|
| t18 | -32.6 | 59.1 | -16, +48, -52 | -1.92 |
| tf-1 | -37.9 | 45.9 | -48, -88, +28 | -1.74 |

t18 D range over 59 s: min -17.09 ms .. max 6.44 ms; zenith-tv's own figure +33 ppm = 1.9 ms over 59 s, i.e. below the 24 ms PDV band: invisible to a 2-10 s windowed minimum.

### tf-1 burst structure around the gap (chunk 17897)

Gap = 1341.63 ms between chunk 17896 (t=45.940 s) and chunk 17897 (t=47.281 s). Nominal rate before the gap: 4.23 Mbit/s over the preceding 2000 chunks.

| window after gap | chunks | bytes | Mbit/s | x nominal |
|---|---|---|---|---|
| 10 ms | 4 | 3008 | 2.4 | 0.6 |
| 50 ms | 61 | 20116 | 3.2 | 0.8 |
| 100 ms | 196 | 64296 | 5.1 | 1.2 |
| 250 ms | 362 | 204732 | 6.6 | 1.5 |
| 500 ms | 380 | 371676 | 5.9 | 1.4 |
| 1000 ms | 432 | 722860 | 5.8 | 1.4 |

Across the gap the arrival clock advanced 1433.5 ms but media advanced only 34.9 ms: nothing was skipped, the media was delivered LATE by 1399 ms.

Excess delay D(t) - D(pre-gap) after the gap (ms): a catch-up burst would bring this back to 0; a live-paced sender leaves it constant.

| t after first post-gap chunk | D excess ms |
|---|---|
| 0.0 s | 1399 |
| 0.5 s | 1387 |
| 1.0 s | 1388 |
| 2.0 s | 1386 |
| 5.0 s | 1352 |
| 10.0 s | 1352 |

The 1.4x-nominal byte rate in the table above is VBR content, not catch-up: the media-time delivery rate after the gap is 1.00x (D constant to within 35 ms). **The stall permanently removes ~1.4 s of buffer; the sender never refills it.**

## Estimators


#### tf-1: estimator output (ms) around the gap (gap ends t=47.28 s, PDV peak 1402 ms)

| estimator | max before gap (t>5s) | pre-gap (t=47.2s) | gap+0.1s | gap+2s | gap+5s | end (59.9s) |
|---|---|---|---|---|---|---|
| hist q95 T250ms f0.98 | 20 | 20 | 20 | 1400 | 20 | 10 |
| hist q99 T250ms f0.98 | 20 | 20 | 20 | 1410 | 1410 | 1400 |
| max-hold 60s (then linear decay over 60s) T250ms | 16 | 16 | 16 | 1402 | 1402 | 1402 |
| max-hold 120s (then linear decay over 120s) T250ms | 16 | 16 | 16 | 1402 | 1402 | 1402 |
| hist q95 T500ms f0.98 | 20 | 20 | 20 | 20 | 20 | 20 |
| hist q99 T500ms f0.98 | 20 | 20 | 20 | 1410 | 1410 | 1410 |
| max-hold 60s (then linear decay over 60s) T500ms | 16 | 16 | 16 | 1402 | 1402 | 1402 |
| max-hold 120s (then linear decay over 120s) T500ms | 16 | 16 | 16 | 1402 | 1402 | 1402 |
| EWMA 0.999 mean+3dev (per PCR sample) | 6 | 6 | 17 | 98 | 95 | 88 |
| RFC 3550 J (x1) | 4 | 2 | 84 | 3 | 1 | 0 |
| RFC 3550 J x3 | 13 | 7 | 252 | 8 | 2 | 1 |

#### t18: estimator output (ms), clean trace (t>5 s)

| estimator | p50 | max | at end |
|---|---|---|---|
| hist q95 T250ms f0.98 | 20 | 20 | 20 |
| hist q99 T250ms f0.98 | 20 | 20 | 20 |
| max-hold 60s (then linear decay over 60s) T250ms | 14 | 16 | 16 |
| max-hold 120s (then linear decay over 120s) T250ms | 14 | 16 | 16 |
| hist q95 T500ms f0.98 | 20 | 20 | 20 |
| hist q99 T500ms f0.98 | 20 | 20 | 20 |
| max-hold 60s (then linear decay over 60s) T500ms | 14 | 16 | 16 |
| max-hold 120s (then linear decay over 120s) T500ms | 14 | 16 | 16 |
| EWMA 0.999 mean+3dev (per PCR sample) | 5 | 11 | 11 |
| RFC 3550 J (x1) | 1 | 4 | 2 |
| RFC 3550 J x3 | 4 | 11 | 6 |

#### Histogram memory of a single 1.4 s interval-max (analytic, forget f=0.98, new-interval weight 1-f=0.02)

After the event the mass in the 1.4 s bucket is (1-f)*f^n. A quantile q still includes it while that mass exceeds (1-q) of the total (total mass ~ 1).

| q | holds the event for n intervals | at T=250 ms | at T=500 ms |
|---|---|---|---|
| 0.95 | 0 (never holds: 1-f = 0.02 <= 0.050) | 0 s | 0 s |
| 0.99 | 34 | 8.6 s | 17.2 s |
| 0.995 | 69 | 17.2 s | 34.3 s |

## Playback model and state machine


#### tf-1

| initial target | after-rebuffer target | zap time to Playing (ms) | stalls | first stall at (s) | freeze (pause) duration (ms) | min buffer while playing (ms) |
|---|---|---|---|---|---|---|
| 250 | 250 | 282 | 1 | 46.15 | 1369 | 264 |
| 500 | 1000 | 501 | 1 | 46.37 | 1913 | 490 |
| 500 | 2000 | 501 | 1 | 46.37 | 2897 | 490 |
| 500 | 2100 | 501 | 1 | 46.37 | 3017 | 490 |
| 1000 | 1000 | 1033 | 1 | 46.90 | 1381 | 1023 |
| 1000 | 2000 | 1033 | 1 | 46.90 | 2364 | 1023 |
| 1000 | 2100 | 1033 | 1 | 46.90 | 2485 | 1023 |
| 1400 | 1400 | 1421 | 1 | 47.29 | 1379 | 1411 |
| 1500 | 1500 | 1532 | 0 | - | - | 135 |
| 2000 | 2000 | 2024 | 0 | - | - | 627 |
| 3000 | 3000 | 3017 | 0 | - | - | 1620 |

#### t18

| initial target | after-rebuffer target | zap time to Playing (ms) | stalls | first stall at (s) | freeze (pause) duration (ms) | min buffer while playing (ms) |
|---|---|---|---|---|---|---|
| 250 | 250 | 283 | 0 | - | - | 276 |
| 500 | 500 | 531 | 0 | - | - | 524 |
| 1000 | 1000 | 1005 | 0 | - | - | 998 |
| 1400 | 1400 | 1430 | 0 | - | - | 1424 |
| 1500 | 1500 | 1502 | 0 | - | - | 1495 |
| 2000 | 2000 | 2021 | 0 | - | - | 2015 |
| 3000 | 3000 | 3010 | 0 | - | - | 3004 |

Smallest initial target (5 ms grid, 20 ms starve threshold) with ZERO stalls over tf-1: **1430 ms**.

Smallest initial target (5 ms grid, 20 ms starve threshold) with ZERO stalls over t18: **40 ms**.

Residual buffer after surviving the gap at initial=1500 ms: min 135 ms, and it is permanent (media is paced live, D never recovers): the next RTO-class gap stalls. With initial 2000/3000:
- initial 2000 ms -> min buffer 627 ms after the gap; a SECOND 1.4 s gap would stall: True
- initial 3000 ms -> min buffer 1620 ms after the gap; a SECOND 1.4 s gap would stall: False

## Findings

1. **Clean link (t18):** chunk gaps are bounded by 25 ms, PDV (2 s min) p99.9 = 15 ms, max 16 ms; RFC 3550 J stays under 4 ms. Sizing on a clean link is dominated by fixed granularity (PCR spacing ~35 ms, an AC-3 frame is 32 ms, plus the Apple start hold of 0.15-0.2 s measured by zenith-tv), not jitter. Smallest zero-stall initial target: 40 ms.
2. **Lossy link (tf-1):** outside the gap the link is as clean as t18 (PDV p90 5 ms with a 2 s window; the p99 of 1.39 s is purely the gap region, ~35 PCR samples of 1689). The gap is a TCP-RTO-class event: 1.342 s between chunks (RFC 6298: RTO floor 1 s; a second consecutive timeout doubles it, a third quadruples it).
3. **No catch-up burst (decision-critical).** After the gap the media-time delivery rate is 1.00x: D stays +1.35..1.40 s above its pre-gap level for the 12.6 s remaining. The stall does not delay-then-flush: it permanently removes ~1.4 s of buffer because the sender is live-paced. Buffer is rebuilt only by waiting (a refill at 1 s/s while paused) because playback cannot be time-scaled (AC-3/E-AC-3 passthrough). A buffer that survives a gap is permanently depleted: a 1500 ms start target survives with 135 ms left and a second gap would stall; even 2000 ms leaves 627 ms. Why the sender does not catch up (server pacing vs receiver back-pressure) is not determined by the trace: open question for zenith-tv.
4. **Minimum buffer to survive the gap: 1430 ms** (PDV peak 1402 ms + 20 ms starve threshold + <= 8 ms quantisation), costing a 1.43 s zap on every channel change. zenith-tv's fixed 1 s start target stalls at t=46.9 s (zap 1033 ms; freeze 1.38 s if resume target 1 s, 2.36 s if 2 s). The player's own log on the same trace shows `buffering` 47.37 s after `loading` and `playing` 2.33 s later.
5. **Estimators:**
   - EWMA mean + 3 sigma (0.999/update): 6 ms before the gap, 98 ms after: blind to a rare stall (the 1.4 s sample is 1/1000 of the mean). Rejected for stall sizing.
   - RFC 3550 J: 84 ms on the gap (J x3: 252 ms), 3 ms two seconds later: it covers 6% of the gap and forgets in ~2 s. A reporting statistic (RFC 3550 6.4.1), not a sizing one; kept only as `Stats::rfc3550_jitter_us`.
   - Forgetting histogram of interval maxima (10 ms buckets, f = 0.98): **q95 retains a single event for at most the intervals it lasts** (new-interval weight 0.02 < 1-q = 0.05; the replay shows 1400 ms at gap+2 s and 20 ms again by gap+5 s at T=250); q99 retains it: analytic 34 intervals (8.6 s at T=250 ms, 17 s at T=500 ms), and in the replay longer (value 1400-1410 ms through the end of the trace, 12.6 s) because the elevated PDV lasts 2 s (the minimum window) and feeds several intervals. Still seconds, against an inter-stall time on a bad link of minutes: the histogram needs a separate stall memory (max-hold).
   - Max-hold: 16 ms before the gap, 1402 ms for the rest of the trace; 60 s vs 120 s are indistinguishable because the trace ends 12.6 s after the gap (decay is not exercised by real data; exercised by the synthetic RTO-pattern tests).
6. **Drift:** t18 OLS -32.6 ppm (media clock fast; matches zenith-tv's +33 ppm in magnitude, sign convention differs), tf-1 pre-gap -37.9 ppm. 15 s block slopes scatter +-50..90 ppm, so a 60 s trace cannot resolve drift better than about +-50 ppm. 33 ppm moves D by 2 ms/min, below the 15-20 ms PDV band; a windowed minimum absorbs it, a whole-trace running minimum would not (PDV inflates by `drift * elapsed`: 2 ms/min at 33 ppm, 6 ms/min at the +-100 ppm used by the synthetic tests). Hence windowed minimum, never a running minimum.
7. **zenith-tv's fixed policy:** a 1 s start stalls on the 1.34 s gap; a 2 s one survives but costs 2 s zap on every zap. The adaptive policy in DESIGN.md pays the extra only after evidence (and only where the evidence says it is needed).
