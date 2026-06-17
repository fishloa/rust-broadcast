## Table 64 — Frame closing pilot patterns (SISO mode)
_§9.2.7.0, PDF p. 112 — combinations of FFT size, guard interval and pilot pattern for which frame closing symbols are used_

| FFT size | 1/128 | 1/32 | 1/16 | 19/256 | 1/8 | 19/128 | 1/4 |
|---|---|---|---|---|---|---|---|
| 32K | | PP6 | PP4 | PP4 | PP2 | PP2 | NA |
| 16K | | PP7, PP6 | PP4, PP5 | PP4, PP5 | PP2, PP3 | PP2, PP3 | PP1 |
| 8K | | PP7 | PP4, PP5 | PP4, PP5 | PP2, PP3 | PP2, PP3 | PP1 |
| 4K, 2K | NA | PP7 | PP4, PP5 | NA | PP2, PP3 | NA | PP1 |
| 1K | NA | NA | PP4, PP5 | NA | PP2, PP3 | NA | PP1 |

NOTE: The entry 'NA' indicates that the corresponding combination of FFT size and guard interval is not allowed. An empty entry indicates that the combination of FFT size and guard interval is allowed, but frame closing symbols are never used.

Frame closing symbols are always used in MISO mode, except with pilot pattern PP8, when frame closing symbols are never used.

