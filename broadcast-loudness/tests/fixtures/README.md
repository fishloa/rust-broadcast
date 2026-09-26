# `broadcast-loudness/tests/fixtures/` — provenance

## `programme_dynamic.wav`

For issue #1051 (audit LOUD-C1): EBU Tech 3342's own steady-tone LRA test
vectors (`tests/compliance.rs`'s `lra_case_1`..`lra_case_4`) cannot expose a
meter that computes LRA from 400 ms momentary blocks instead of 3 s
short-term values, because a steady tone's 400 ms and 3 s loudness are
identical. This fixture is "programme-like" (fast level changes, faster
than the 3 s short-term smoothing window but slower than a single 400 ms
block) specifically to make the two diverge, matching the audit's own
description of the failure mode ("speech pauses, transients, music
dynamics").

Synthetic, generated with ffmpeg 8.1.2 (`sine` + `volume` with a
time-varying expression) — mono, 16 kHz, 16-bit PCM, 12 s, alternating
every 0.5 s between -20 dBFS and -35 dBFS (a 1 Hz square-wave envelope on a
1 kHz tone):

```bash
ffmpeg -y -f lavfi -i "sine=frequency=1000:sample_rate=16000:duration=12" \
  -af "volume=eval=frame:volume='if(lt(mod(t,1.0),0.5),0.1,0.017782794)'" \
  -ac 1 -c:a pcm_s16le programme_dynamic.wav
```

(`0.1` = 10^(-20/20), `0.017782794` = 10^(-35/20).)

Independent oracle: ffmpeg's own `ebur128` filter, run directly against
this file — NOT our renderer, NOT our own checker:

```bash
ffmpeg -i programme_dynamic.wav -af ebur128 -f null -
```

reports `LRA: 0.2 LU` (the 3 s short-term windows smooth the 1 Hz
alternation down to near-zero spread — correct: this envelope changes much
faster than the 3 s window, so almost every 3 s window sees close to the
same average and the resulting distribution is narrow). A meter that (like
this crate before the fix) computes the same percentile spread from 400 ms
momentary blocks instead sees each block sitting much closer to one pure
level, producing far more spread across 400 ms blocks and a badly inflated
LRA (`tests/compliance.rs`'s
`lra_case_5_programme_dynamic_matches_ffmpeg_oracle` records the exact
pre-fix value).

Synthetic input only (no third-party media); released under the workspace
licence (MIT OR Apache-2.0).

## `programme_segments.wav`

A second, independent LRA cross-check (coordinator review of issue #1051):
`programme_dynamic.wav` above targets ffmpeg's reported `0.2 LU` with a
±0.5 LU tolerance — a real but small gap. This fixture targets a much
larger, slower-moving LRA (segment-scale level changes, closer to real
programme dynamics) so the fix can be verified to a tighter ±0.1 LU.

Synthetic, generated with ffmpeg 8.1.2 (`sine` + `volume` with a
time-varying expression) — mono, 8 kHz, 16-bit PCM, 40 s: four 10 s
segments at -30, -20, -25, and -15 dBFS (peak level) respectively:

```bash
ffmpeg -y -f lavfi -i "sine=frequency=1000:sample_rate=8000:duration=40" \
  -af "volume=eval=frame:volume='if(lt(mod(t,40),10),0.03162277660168379,\
if(lt(mod(t,40),20),0.1,if(lt(mod(t,40),30),0.05623413251903491,\
0.1778279410038923)))'" \
  -ac 1 -c:a pcm_s16le programme_segments.wav
```

(`0.03162277660168379` = 10^(-30/20), `0.1` = 10^(-20/20),
`0.05623413251903491` = 10^(-25/20), `0.1778279410038923` = 10^(-15/20).)

Independent oracle: ffmpeg's own `ebur128` filter, run directly against
this file:

```bash
ffmpeg -i programme_segments.wav -af ebur128 -f null -
```

reports `LRA: 15.0 LU`. ffmpeg prints LRA to one decimal place, so its
printed value can be up to ±0.05 LU from the true underlying value purely
from that rounding; `tests/compliance.rs`'s
`lra_case_6_programme_segments_matches_ffmpeg_oracle_tightly` uses a ±0.1 LU
tolerance against the printed `15.0`, which comfortably absorbs that
rounding margin plus ordinary cross-implementation numerical differences
while still being a meaningfully tight check.

Synthetic input only (no third-party media); released under the workspace
licence (MIT OR Apache-2.0).
