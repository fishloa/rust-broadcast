# live-buffer: trace analysis

Evidence base for the defaults in [`DESIGN.md`](DESIGN.md). Everything below is produced by the deterministic stdlib-python scripts in [`analysis/`](analysis/) (`trace_stats.py`, `estimators.py`, `controller_sim.py`; shared loaders in `common.py`), run against the PRIVATE traces `private/fixtures/live-buffer/*.csv` (zenith-tv, Apple TV 4K, live HTTP-TS, 8 traces; provenance in that directory's `PROVENANCE.md`; the traces are not redistributable and are never copied into this public repo, only derived numbers are). Re-run: `python3 docs/live-buffer/analysis/<script>.py [--dir <traces>]`; output is byte-stable. This file is the pasted output plus findings; regenerate the tables by re-running the scripts. **Revision 2** (all 8 traces; the earlier t18 was replaced by the 12:03 run, so revision 1 numbers are superseded).

**Trace classes.** Gap-free ("clean"): `t18`, `rai-1`, `rai-sport`, `orf1`, `orf-sport`, `servustv#2` (the second connection of servustv): 339.5 s in total. Gap events (stall-class (~1.0-1.5 s), 1.33-1.42 s): `tf-1` (mid-stream, old server queue), `6ter` (mid-stream, bigger queue), `servustv#1` (a STARTUP gap 0.49 s into the stream, then a clean `source_end` at 13.9 s). Reconnect: `servustv` (`source_end`, `reconnecting`, 3.38 s of dead time, new connection joins the live edge 1.3 s ahead). `servustv` is split at the reconnect into `#1`/`#2` everywhere below.

**Methods.** Arrival time = `arrival_ns/1000` (integer us, rebased). Media time = `pcr_27mhz/27` us, unrolled with the `clock33::pcr_signed_distance` half-range rule. Only chunks carrying a PCR (one every ~35 ms, 105 ms on some servustv stretches) have a media time. PDV samples are taken at those chunks only (chunks without a PCR are skipped, not interpolated), and the playback model holds the last PCR's media time until the next one (sample-and-hold: it under-estimates arrived media by at most one PCR interval, so buffer figures are conservative by <= ~35 ms). PDV is the RFC 5481 relative-transit form `D = arrival - media` minus a causal sliding minimum of D (2 s and 10 s windows). Analysis scripts use floats where convenient; the crate uses the integer forms of DESIGN.md section 3.

**Sample-size honesty.** Three gap events in 471 s of gap-class traces, one per trace; five of the seven 12:03 channels were gap-free at the same wall time as the two that were not. That fixes the *existence and size* of the event (three gaps of 1334, 1342 and 1422 ms: a tight cluster, of a 1.0-1.5 s delivery stall whose cause is undetermined (path retransmission or a producer-side stall in the server; downstream zenith later traced it to producer starvation plus a pacer that re-anchors instead of bursting)) but not its rate (3/471 s is ~23/h with an enormous interval) nor whether it belongs to the channel, the server queue or the path. Rates below are stated with that caveat; the synthetic suite carries scenario coverage, not the real traces.

## Tables

### Trace summary (servustv split at its reconnect into #1/#2)

| trace | class | chunks | PCR chunks | duration s | mean Mbit/s | media/wall |
|---|---|---|---|---|---|---|
| t18 | clean | 92216 | 1723 | 59.6 | 3.37 | 0.99998 |
| rai-1 | clean | 46805 | 2415 | 59.4 | 6.17 | 1.00012 |
| rai-sport | clean | 111176 | 2409 | 59.4 | 7.55 | 0.99988 |
| orf1 | clean | 7025 | 2068 | 59.4 | 7.72 | 1.00006 |
| orf-sport | clean | 6762 | 2103 | 59.9 | 7.28 | 1.00002 |
| servustv#1 | STALL | 1980 | 353 | 13.9 | 8.62 | 0.90597 |
| servustv#2 | clean | 16663 | 1214 | 41.9 | 9.83 | 1.00007 |
| 6ter | STALL | 40079 | 1628 | 57.9 | 4.30 | 0.97770 |
| tf-1 | STALL | 19175 | 1689 | 59.9 | 5.12 | 0.97747 |

Clean traces total 339.5 s (0.094 h) of data; stall traces 131.7 s.

### Chunk inter-arrival gap (ms)

| trace | p50 | p90 | p99 | p99.9 | max |
|---|---|---|---|---|---|
| t18 | 0.09 | 0.27 | 15.67 | 16.43 | 28.60 |
| rai-1 | 0.07 | 5.06 | 13.87 | 15.06 | 24.69 |
| rai-sport | 0.06 | 0.25 | 12.98 | 13.75 | 22.31 |
| orf1 | 4.52 | 21.59 | 24.77 | 32.18 | 37.26 |
| orf-sport | 5.35 | 21.92 | 26.20 | 31.34 | 32.72 |
| servustv#1 | 0.07 | 34.59 | 35.55 | 104.39 | 1334.71 |
| servustv#2 | 0.07 | 13.01 | 22.02 | 26.30 | 30.39 |
| 6ter | 0.09 | 1.00 | 34.95 | 36.02 | 1421.61 |
| tf-1 | 1.51 | 7.75 | 34.89 | 36.35 | 1341.63 |
| **pooled clean** | 0.08 | 0.52 | 20.90 | 23.83 | 37.26 |

### PDV (RFC 5481, D - causal sliding min over 2 s / 10 s), ms

Samples are PCR-carrying chunks only. 'pooled clean' concatenates the PDV samples of the 6 gap-free traces (t18, rai-1, rai-sport, orf1, orf-sport, servustv#2).

| trace | window | p50 | p90 | p99 | p99.9 | max |
|---|---|---|---|---|---|---|
| t18 | 2 s | 2.74 | 9.31 | 12.14 | 15.89 | 24.36 |
| t18 | 10 s | 5.16 | 10.08 | 13.28 | 15.98 | 24.50 |
| rai-1 | 2 s | 0.62 | 1.20 | 9.23 | 13.22 | 16.78 |
| rai-1 | 10 s | 0.73 | 1.28 | 9.28 | 13.22 | 16.93 |
| rai-sport | 2 s | 0.82 | 5.29 | 10.01 | 11.25 | 20.24 |
| rai-sport | 10 s | 1.09 | 10.01 | 10.65 | 11.38 | 20.24 |
| orf1 | 2 s | 1.49 | 11.26 | 17.69 | 25.00 | 31.61 |
| orf1 | 10 s | 4.26 | 12.65 | 18.95 | 25.00 | 35.75 |
| orf-sport | 2 s | 2.42 | 15.29 | 25.05 | 29.78 | 35.38 |
| orf-sport | 10 s | 10.19 | 20.76 | 27.52 | 32.19 | 35.88 |
| servustv#1 | 2 s | 0.70 | 1.68 | 1305.51 | 1336.18 | 1336.18 |
| servustv#1 | 10 s | 1305.16 | 1305.70 | 1306.40 | 1336.18 | 1336.18 |
| servustv#2 | 2 s | 0.76 | 4.58 | 10.95 | 14.24 | 14.74 |
| servustv#2 | 10 s | 0.86 | 4.70 | 10.95 | 14.31 | 14.87 |
| 6ter | 2 s | 0.68 | 3.46 | 45.69 | 1430.87 | 1447.00 |
| 6ter | 10 s | 1.01 | 1395.52 | 1430.89 | 1441.21 | 1447.08 |
| tf-1 | 2 s | 1.35 | 5.33 | 1386.96 | 1400.73 | 1401.73 |
| tf-1 | 10 s | 2.19 | 1356.10 | 1391.14 | 1401.45 | 1402.45 |
| **pooled clean** | 2 s | 0.91 | 9.27 | 19.04 | 26.98 | 35.38 |
| **pooled clean** | 10 s | 1.29 | 12.52 | 24.93 | 28.88 | 35.88 |

### RFC 3550 A.8 smoothed jitter J (integer, ms)

| trace | J p50 | J p99 | J max |
|---|---|---|---|
| t18 | 2.07 | 3.44 | 4.93 |
| rai-1 | 0.34 | 5.37 | 5.76 |
| rai-sport | 0.44 | 5.92 | 7.02 |
| orf1 | 4.68 | 8.40 | 9.10 |
| orf-sport | 3.57 | 10.09 | 10.37 |
| servustv#1 | 0.30 | 71.99 | 85.25 |
| servustv#2 | 1.45 | 5.47 | 5.70 |
| 6ter | 0.56 | 33.01 | 91.11 |
| tf-1 | 1.40 | 32.54 | 89.58 |

### PCR-vs-arrival drift (OLS slope of D on arrival time; negative = media clock fast)

Stall traces are cut before the gap and traces under 30 s are skipped (a level shift is not drift); servustv uses its two connections separately. Traces are 10-60 s: single-trace estimates carry about +-50-100 ppm of PDV-noise error, so only the pooled spread is meaningful.

| trace | span s | OLS ppm |
|---|---|---|
| t18 | 59.6 | -0.7 |
| rai-1 | 59.3 | -67.8 |
| rai-sport | 59.3 | +72.9 |
| orf1 | 59.4 | -13.6 |
| orf-sport | 59.9 | +27.9 |
| servustv#2 | 41.8 | -51.8 |
| tf-1 | 45.9 | -37.9 |

Pooled: median -14 ppm, min -68, max +73 ppm (n=7). 100 ppm over the 2 s minimum window = 0.2 ms: the windowed minimum absorbs any drift in this range; a whole-trace running minimum would inflate PDV by `drift x elapsed` (6 ms/min at 100 ppm).

### tf-1: structure after the gap (chunk 17897)

Gap 1341.63 ms (t=45.94 s to 47.28 s). Arrival advanced 1433 ms across it, media 35 ms: the media was delivered late by 1399 ms, nothing skipped. Pre-gap rate 4.31 Mbit/s (preceding chunks).

| window after gap | chunks | Mbit/s | media s delivered per wall s |
|---|---|---|---|
| 100 ms | 196 | 5.1 | 1.029 |
| 250 ms | 362 | 6.6 | 1.057 |
| 500 ms | 380 | 5.9 | 1.028 |
| 1000 ms | 432 | 5.8 | 1.012 |
| 2000 ms | 526 | 5.4 | 1.006 |
| 5000 ms | 685 | 5.1 | 1.009 |

Delay excess D(t) - D(pre-gap) (ms), 0 = back to baseline:

| after first post-gap chunk | D excess |
|---|---|
| 0 s | 1399 |
| 1 s | 1388 |
| 2 s | 1386 |
| 5 s | 1352 |
| 10 s | 1352 |
| end (12.7 s) | 1353 |

### 6ter: structure after the gap (chunk 37847)

Gap 1421.61 ms (t=24.64 s to 26.06 s). Arrival advanced 1482 ms across it, media 36 ms: the media was delivered late by 1447 ms, nothing skipped. Pre-gap rate 2.93 Mbit/s (preceding chunks).

| window after gap | chunks | Mbit/s | media s delivered per wall s |
|---|---|---|---|
| 100 ms | 13 | 3.7 | 1.299 |
| 250 ms | 22 | 3.4 | 1.087 |
| 500 ms | 36 | 3.3 | 1.037 |
| 1000 ms | 65 | 3.3 | 1.018 |
| 2000 ms | 120 | 3.4 | 1.009 |
| 5000 ms | 296 | 3.9 | 1.010 |

Delay excess D(t) - D(pre-gap) (ms), 0 = back to baseline:

| after first post-gap chunk | D excess |
|---|---|
| 0 s | 1447 |
| 1 s | 1430 |
| 2 s | 1430 |
| 5 s | 1396 |
| 10 s | 1329 |
| 20 s | 1329 |
| end (31.8 s) | 1295 |

### servustv#1: structure after the gap (chunk 136)

Gap 1334.71 ms (t=0.49 s to 1.83 s). Arrival advanced 1367 ms across it, media 34 ms: the media was delivered late by 1333 ms, nothing skipped. Pre-gap rate 10.17 Mbit/s (preceding chunks).

| window after gap | chunks | Mbit/s | media s delivered per wall s |
|---|---|---|---|
| 100 ms | 21 | 12.2 | 1.799 |
| 250 ms | 49 | 11.7 | 1.145 |
| 500 ms | 79 | 10.7 | 1.067 |
| 1000 ms | 184 | 10.2 | 1.033 |
| 2000 ms | 409 | 10.1 | 1.016 |
| 5000 ms | 916 | 9.8 | 1.006 |

Delay excess D(t) - D(pre-gap) (ms), 0 = back to baseline:

| after first post-gap chunk | D excess |
|---|---|
| 0 s | 1333 |
| 1 s | 1302 |
| 2 s | 1301 |
| 5 s | 1302 |
| 10 s | 1302 |
| end (12.1 s) | 1303 |

### servustv: source_end then reconnect (chunk 1980)

Arrival gap 3384.96 ms (events: `source_end` 10.7 s after loading, `reconnecting(attempt: 1)` 54 ms later, `playing` 5.0 s after that; the trace's own first 13.9 s are `servustv#1`, which ALSO holds a 1.33 s STARTUP gap 0.49 s in, see above). Across the gap arrival advanced 3420 ms but media advanced 4720 ms: the new connection joined the live edge **1300 ms ahead** of where the old one stopped (D fell 1300 ms). The PCR is continuous in value (no wrap, no backwards step); it is a forward media jump. D then stays constant (baseline 1.3 s lower): a reconnect is a re-baseline event, not delay variation, and the 3.4 s dead time is not evidence about path jitter.

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

#### 6ter: estimator output (ms) around the gap (gap ends t=26.06 s, PDV peak 1447 ms)

| estimator | max before gap (t>5s) | pre-gap (t=26.0s) | gap+0.1s | gap+2s | gap+5s | end (57.9s) |
|---|---|---|---|---|---|---|
| hist q95 T250ms f0.98 | 10 | 10 | 10 | 1440 | 10 | 10 |
| hist q99 T250ms f0.98 | 10 | 10 | 10 | 1450 | 1450 | 20 |
| max-hold 60s (then linear decay over 60s) T250ms | 8 | 8 | 8 | 1447 | 1447 | 1447 |
| max-hold 120s (then linear decay over 120s) T250ms | 8 | 8 | 8 | 1447 | 1447 | 1447 |
| hist q95 T500ms f0.98 | 10 | 10 | 10 | 10 | 10 | 10 |
| hist q99 T500ms f0.98 | 10 | 10 | 10 | 1450 | 1450 | 1440 |
| max-hold 60s (then linear decay over 60s) T500ms | 8 | 8 | 8 | 1447 | 1447 | 1447 |
| max-hold 120s (then linear decay over 120s) T500ms | 8 | 8 | 8 | 1447 | 1447 | 1447 |
| EWMA 0.999 mean+3dev (per PCR sample) | 3 | 3 | 20 | 92 | 90 | 63 |
| RFC 3550 J (x1) | 3 | 1 | 81 | 3 | 1 | 0 |
| RFC 3550 J x3 | 10 | 2 | 243 | 9 | 3 | 1 |

#### t18: estimator output (ms), clean trace (t>5 s)

| estimator | p50 | max | at end |
|---|---|---|---|
| hist q95 T500ms f0.98 | 20 | 20 | 20 |
| hist q99 T500ms f0.98 | 20 | 30 | 30 |
| max-hold 60s (then linear decay over 60s) T500ms | 16 | 24 | 24 |
| EWMA 0.999 mean+3dev (per PCR sample) | 6 | 11 | 11 |
| RFC 3550 J (x1) | 2 | 5 | 3 |
| RFC 3550 J x3 | 6 | 15 | 8 |

#### rai-1: estimator output (ms), clean trace (t>5 s)

| estimator | p50 | max | at end |
|---|---|---|---|
| hist q95 T500ms f0.98 | 10 | 20 | 20 |
| hist q99 T500ms f0.98 | 20 | 20 | 20 |
| max-hold 60s (then linear decay over 60s) T500ms | 17 | 17 | 17 |
| EWMA 0.999 mean+3dev (per PCR sample) | 1 | 5 | 5 |
| RFC 3550 J (x1) | 0 | 6 | 5 |
| RFC 3550 J x3 | 1 | 17 | 16 |

#### rai-sport: estimator output (ms), clean trace (t>5 s)

| estimator | p50 | max | at end |
|---|---|---|---|
| hist q95 T500ms f0.98 | 20 | 20 | 20 |
| hist q99 T500ms f0.98 | 30 | 30 | 20 |
| max-hold 60s (then linear decay over 60s) T500ms | 20 | 20 | 20 |
| EWMA 0.999 mean+3dev (per PCR sample) | 5 | 7 | 4 |
| RFC 3550 J (x1) | 0 | 7 | 0 |
| RFC 3550 J x3 | 1 | 21 | 1 |

#### orf1: estimator output (ms), clean trace (t>5 s)

| estimator | p50 | max | at end |
|---|---|---|---|
| hist q95 T500ms f0.98 | 30 | 30 | 30 |
| hist q99 T500ms f0.98 | 30 | 40 | 40 |
| max-hold 60s (then linear decay over 60s) T500ms | 27 | 32 | 32 |
| EWMA 0.999 mean+3dev (per PCR sample) | 9 | 13 | 13 |
| RFC 3550 J (x1) | 5 | 9 | 3 |
| RFC 3550 J x3 | 14 | 27 | 9 |

#### orf-sport: estimator output (ms), clean trace (t>5 s)

| estimator | p50 | max | at end |
|---|---|---|---|
| hist q95 T500ms f0.98 | 30 | 30 | 30 |
| hist q99 T500ms f0.98 | 40 | 40 | 40 |
| max-hold 60s (then linear decay over 60s) T500ms | 35 | 35 | 35 |
| EWMA 0.999 mean+3dev (per PCR sample) | 17 | 19 | 19 |
| RFC 3550 J (x1) | 4 | 10 | 8 |
| RFC 3550 J x3 | 11 | 31 | 24 |

#### servustv#2: estimator output (ms), clean trace (t>5 s)

| estimator | p50 | max | at end |
|---|---|---|---|
| hist q95 T500ms f0.98 | 20 | 20 | 20 |
| hist q99 T500ms f0.98 | 20 | 20 | 20 |
| max-hold 60s (then linear decay over 60s) T500ms | 15 | 15 | 15 |
| EWMA 0.999 mean+3dev (per PCR sample) | 2 | 4 | 4 |
| RFC 3550 J (x1) | 2 | 6 | 4 |
| RFC 3550 J x3 | 5 | 17 | 11 |

#### Histogram memory of a single 1.4 s interval-max (analytic, forget f=0.98, new-interval weight 1-f=0.02)

After the event the mass in the 1.4 s bucket is (1-f)*f^n. A quantile q still includes it while that mass exceeds (1-q) of the total (total mass ~ 1).

| q | holds the event for n intervals | at T=250 ms | at T=500 ms |
|---|---|---|---|
| 0.95 | 0 (never holds: 1-f = 0.02 <= 0.050) | 0 s | 0 s |
| 0.99 | 34 | 8.6 s | 17.2 s |
| 0.995 | 69 | 17.2 s | 34.3 s |

## Playback model, policy comparison and false starvation

### Smallest start target with zero stalls (5 ms grid, 20 ms starve threshold)

| trace | min zero-stall start (ms) | zap at that target (ms) |
|---|---|---|
| tf-1 | 1430 | 1460 |
| 6ter | 1475 | 1510 |

servustv#1 is a STARTUP gap: the player is still Buffering when it hits, so it cannot starve; it only delays Playing (zap table below).

### Policy comparison on the three gap traces (zap = first PCR chunk to Playing; freeze = length of each pause)

| trace | policy | zap ms | stalls | freeze ms |
|---|---|---|---|---|
| tf-1 | fixed 1 s start / 2 s resume | 1033 | 1 | 2364 @46.9s |
| tf-1 | flat 1.5 s | 1532 | 0 | - |
| tf-1 | adaptive (initial 0.5 s) | 501 | 1 | 3017 @46.4s |
| tf-1 | adaptive (initial 1.5 s) | 1532 | 0 | - |
| tf-1 | adaptive + seed peak 1.45 s | 1746 | 0 | - |
| 6ter | fixed 1 s start / 2 s resume | 1017 | 1 | 2411 @25.6s |
| 6ter | flat 1.5 s | 1510 | 0 | - |
| 6ter | adaptive (initial 0.5 s) | 526 | 1 | 3078 @25.1s |
| 6ter | adaptive (initial 1.5 s) | 1510 | 0 | - |
| 6ter | adaptive + seed peak 1.45 s | 1755 | 0 | - |
| servustv#1 | fixed 1 s start / 2 s resume | 2319 | 0 | - |
| servustv#1 | flat 1.5 s | 2810 | 0 | - |
| servustv#1 | adaptive (initial 0.5 s) | 2915 | 0 | - |
| servustv#1 | adaptive (initial 1.5 s) | 2915 | 0 | - |
| servustv#1 | adaptive + seed peak 1.45 s | 3056 | 0 | - |

### Start target vs (zap ms / number of pauses) on every trace, resume target 2 s

| start target | tf-1 | 6ter | servustv#1 | t18 | rai-1 | rai-sport | orf1 | orf-sport | servustv#2 |
|---|---|---|---|---|---|---|---|---|---|
| 300 | 318 / 1 | 315 / 1 | 314 / 1 | 317 / 0 | 302 / 0 | 322 / 0 | 329 / 0 | 317 / 0 | 314 / 0 |
| 500 | 501 / 1 | 526 / 1 | 1860 / 0 | 539 / 0 | 501 / 0 | 527 / 0 | 508 / 0 | 532 / 0 | 524 / 0 |
| 1000 | 1033 / 1 | 1017 / 1 | 2319 / 0 | 1008 / 0 | 999 / 0 | 1009 / 0 | 1018 / 0 | 1007 / 0 | 1014 / 0 |
| 1500 | 1532 / 0 | 1510 / 0 | 2810 / 0 | 1535 / 0 | 1518 / 0 | 1506 / 0 | 1528 / 0 | 1517 / 0 | 1513 / 0 |
| 2000 | 2024 / 0 | 2001 / 0 | 3335 / 0 | 2035 / 0 | 2015 / 0 | 2021 / 0 | 2015 / 0 | 2027 / 0 | 2003 / 0 |

### False starvation and target distribution of the ADAPTIVE controller (initial 0.5 s) on the gap-free traces

| trace | duration s | false pauses | target p50 ms | p99 | max | % time at min (300 ms) | zap ms |
|---|---|---|---|---|---|---|---|
| t18 | 59.6 | 0 | 300 | 300 | 300 | 100.0 | 539 |
| rai-1 | 59.4 | 0 | 300 | 300 | 300 | 100.0 | 501 |
| rai-sport | 59.4 | 0 | 300 | 300 | 300 | 100.0 | 527 |
| orf1 | 59.4 | 0 | 300 | 300 | 300 | 100.0 | 508 |
| orf-sport | 59.9 | 0 | 300 | 300 | 300 | 100.0 | 532 |
| servustv#2 | 41.9 | 0 | 300 | 300 | 300 | 100.0 | 524 |
| **pooled** | 339.5 | 0 | 300 | 300 | 300 | 100.0 | |

False starvations: 0 in 0.094 h of gap-free data = 0.0/h observed. With zero events the rule-of-three 95% upper bound is 32/h: the data cannot show a lower rate than that, so the synthetic clean-jitter scenarios must carry the rest.

### Adaptive target over time on the gap traces (ms)

| trace | before gap | just after gap end | +5 s | +20 s | end of trace | max |
|---|---|---|---|---|---|---|
| tf-1 | 300 | 1682 | 1682 | 1682 | 1682 | 1682 |
| 6ter | 300 | 1736 | 1736 | 1736 | 1736 | 1736 |

### Repeat-gap test (6ter with a second identical 1.43 s late-shift injected 20 s after the first gap)

| policy | zap ms | pauses | freeze ms (each) |
|---|---|---|---|
| fixed 1 s start / 2 s resume | 1017 | 1 | 2411 @25.6s |
| flat 1.5 s | 1510 | 1 | 2769 @46.2s |
| adaptive (initial 0.5 s) | 526 | 1 | 3078 @25.1s |
| adaptive (initial 1.5 s) | 1510 | 1 | 3435 @46.2s |
| adaptive + seed peak 1.45 s | 1755 | 1 | 3191 @46.4s |

### Synthetic lossy link (30 min, one 1.43 s no-catch-up gap every N s; N=180 s is the pooled real rate, 3 events in 471 s of gap-class traces is ~20/h with n=3)

| gap every | policy | zap ms | pauses (stall + re-prime) | frozen s per 30 min | frozen % |
|---|---|---|---|---|---|
| 180 s | fixed 1 s start / 2 s resume | 1021 | 5 (5+0) | 13.9 | 0.77 |
| 180 s | flat 1.5 s | 1501 | 4 (4+0) | 11.5 | 0.64 |
| 180 s | adaptive (initial 0.5 s) | 526 | 5 (5+0) | 14.5 | 0.81 |
| 180 s | adaptive (initial 1.5 s) | 1501 | 4 (4+0) | 12.1 | 0.67 |
| 180 s | adaptive + seed peak 1.45 s | 1758 | 4 (4+0) | 11.9 | 0.66 |
| 180 s | adaptive (0.5 s) + re-prime at <50% of target for 2 s | 526 | 9 (1+8) | 14.1 | 0.78 |
| 180 s | adaptive (1.5 s) + re-prime | 1501 | 9 (0+9) | 13.1 | 0.73 |
| 600 s | fixed 1 s start / 2 s resume | 1021 | 1 (1+0) | 2.4 | 0.13 |
| 600 s | flat 1.5 s | 1501 | 1 (1+0) | 2.9 | 0.16 |
| 600 s | adaptive (initial 0.5 s) | 526 | 1 (1+0) | 3.1 | 0.17 |
| 600 s | adaptive (initial 1.5 s) | 1501 | 1 (1+0) | 3.5 | 0.20 |
| 600 s | adaptive + seed peak 1.45 s | 1758 | 1 (1+0) | 3.3 | 0.18 |
| 600 s | adaptive (0.5 s) + re-prime at <50% of target for 2 s | 526 | 2 (1+1) | 4.1 | 0.23 |
| 600 s | adaptive (1.5 s) + re-prime | 1501 | 2 (0+2) | 3.1 | 0.17 |

## Findings

1. **Gap-free traces (6 traces, 339.5 s):** pooled chunk-gap p99.9 = 23.8 ms, max 37.3 ms; pooled PDV (2 s window) p50/p90/p99/p99.9/max = 0.9/9.3/19.0/27.0/35.4 ms (worst trace orf-sport: p99 25 ms, max 35 ms); RFC 3550 J max 10.4 ms. The sizing problem on a healthy link is the fixed granularity (PCR spacing ~35 ms, AC-3 frame 32 ms, Apple start hold 0.15-0.2 s measured by zenith-tv), not jitter. The adaptive target sits at its 300 ms floor 100.0% of the time on all six (the controller does not hover above min), with 0 false pauses; the 95% upper bound on the false-pause rate that 0.094 h of data can show is 32/h (rule of three), so the synthetic suite must carry the rest.
2. **Three delivery stalls (~1.0-1.5 s stall class) of 1334/1342/1422 ms** (PDV peaks 1336/1402/1447 ms). Outside them the same traces are as clean as the gap-free ones (tf-1 PDV p90 5.3 ms, 6ter p90 3.5 ms). A gap is a late delivery, never a loss: media delivered late by 1333/1399/1447 ms, nothing skipped.
3. **No catch-up (decision-critical, now n=3).** After each gap media is delivered at 1.00-1.03x the live rate (VBR noise included) and the delay excess falls by only ~0.5-1% per second: tf-1 1399 -> 1353 ms over 12.7 s; 6ter 1447 -> 1295 ms over 31.8 s (a slow ~5 ms/s catch-up: it would take ~5 minutes to recover); servustv#1 1333 -> 1303 ms over 12 s. The expected "clean late burst" of 6ter is not a burst in media terms: 13 chunks in the first 100 ms, 1.3x media rate for 100 ms, 1.09x at 250 ms, 1.02x at 1 s. Practical consequence unchanged from revision 1: a gap permanently removes ~1.4 s of lead; a lead is rebuilt only by waiting (a pause), and **the total frozen time over a session is conserved**: the synthetic 30-minute lossy run shows every policy freezing 11.5-14.5 s for 10 gaps (~1.4 s per gap), regardless of start target. A larger initial buffer absorbs ONE gap invisibly (once per session), nothing else. Why the sender does not catch up (server pacing vs receiver back-pressure) is not determined by the traces; making it burst is the one change that would reduce total freeze, and it is outside this crate (section 8 of DESIGN.md).
4. **Minimum buffer to survive a gap:** tf-1 1430 ms, 6ter 1475 ms (PDV peak 1447 + 20 ms starve threshold + <= 8 ms quantisation): a flat 1.5 s start survives both but with only 70/25 ms to spare and **leaves 25-70 ms of lead permanently**, so the next gap stalls (repeat-gap test: flat 1.5 s and adaptive-1.5 s stall on the second gap with a 2.8-3.4 s freeze; fixed 1 s/2 s and adaptive-0.5 s, having already stalled once and refilled to 2.0/2.1 s, survive it). The startup gap (servustv#1) cannot starve (the player is still Buffering) but costs zap time: 1 s start -> zap 2319 ms, flat 1.5 s -> 2810 ms, adaptive 0.5 s -> 2915 ms (the controller raises the start target after seeing the gap while still filling: peak 1333 ms x 1.2 = 1.6 s), vs the 4.1 s the player actually took.
5. **Policy comparison (gap traces, zap ms / pauses / freeze ms):** fixed 1 s start, 2 s resume: tf-1 1033/1/2364, 6ter 1017/1/2411. Flat 1.5 s: 1532/0/- and 1510/0/-. Adaptive (start 0.5 s, resume max(target, 1.5 x peak) = 2.1-2.2 s): 501/1/3017 and 526/1/3078. So the proposal saves ~0.5 s of zap time on every zap and pays +0.6-0.7 s of freeze per stall against the fixed policy (the price of resuming at 1.5 x depth instead of 2 s; it is what makes the second gap survivable at lower cost than the fixed policy's ratchet, which never relaxes).
6. **Estimators:**
   - EWMA mean + 3 sigma (0.999/update): 6 ms (tf-1) / 3 ms (6ter) before, 98 / 92 ms after: blind to a rare stall. Rejected for stall sizing.
   - RFC 3550 J: 84 / 81 ms on the gap (J x3: 252 / 243 ms), 3 ms two seconds later: a reporting statistic, kept only as `Stats::rfc3550_jitter_us`.
   - Forgetting histogram of interval maxima (10 ms buckets, f = 0.98): q95 shows the gap for one interval only (1400/1440 ms at gap+2 s at T=250, 20/10 ms again by gap+5 s; never at T=500 for tf-1/6ter); q99 holds it (value 1400-1450 ms to the end of both traces, analytic hold 8.6 s at T=250, 17 s at T=500, plus the 2 s elevated-PDV reinforcement). **Using that histogram baseline inside `k1 * baseline + margin` double-counts the stall**: the first version of the policy simulation produced a target of 2920 ms (2 x 1410 + 100) after tf-1's gap instead of 1.68 s, a 3.8 s freeze instead of 3.0 s, and a 2.9 s start on servustv#1. Fix (applied in the sim and in DESIGN.md): interval maxima above `outlier_us = 500 ms` feed only the max-hold, never the histogram. With it the target after the gaps is 1682 ms (tf-1) / 1736 ms (6ter) = `k2 x peak`.
   - Max-hold: 16/8 ms before, 1402/1447 ms after, for the rest of both traces (60 s vs 120 s indistinguishable: the traces end 12.7/31.8 s after the gap; decay is exercised only by the synthetic suite).
7. **Drift** (single-trace OLS, traces >= 30 s, stall traces cut before the gap): t18 -1, rai-1 -68, rai-sport +73, orf1 -14, orf-sport +28, servustv#2 -52, 6ter -107, tf-1 -38 ppm: median -14 ppm, range -107..+73 ppm. A 60 s trace resolves drift to roughly +-50-100 ppm, so the spread is mostly estimator noise; the earlier "+33 ppm" figure is inside it. 100 ppm moves D by 0.2 ms across the 2 s minimum window; a whole-trace running minimum would inflate PDV by 6 ms/min. Windowed minimum, never a running one: unchanged.
8. **Reconnect (servustv):** `source_end` at 10.7 s, the new connection's first data 3.38 s after the last old chunk. The PCR is continuous in value and moves FORWARD: media advanced 4720 ms across 3420 ms of arrival (the new connection joined the live edge 1300 ms ahead), so D fell by 1300 ms and then stayed constant. A reconnect is therefore a re-baseline, not delay variation: the 3.4 s dead time must not enter the stall memory (it would clamp the target to `max_us` for two minutes) and the 1.3 s forward jump sits only 30% above the 1 s `discontinuity_us` threshold, so the controller must be TOLD about the reconnect (`Hint::Reconnected`) rather than infer it.
