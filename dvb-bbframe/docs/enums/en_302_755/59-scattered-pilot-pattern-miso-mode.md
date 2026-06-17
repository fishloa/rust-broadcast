## Table 59 — Scattered pilot pattern (MISO mode)
_§9.2.3.1, PDF p. 109 — allowed combinations of FFT size and guard interval_

| FFT size | 1/128 | 1/32 | 1/16 | 19/256 | 1/8 | 19/128 | 1/4 |
|---|---|---|---|---|---|---|---|
| 32K | PP8, PP4, PP6 | PP8, PP4 | PP2, PP8 | PP2, PP8 | NA | NA | NA |
| 16K | PP8, PP4, PP5 | PP8, PP4, PP5 | PP3, PP8 | PP3, PP8 | PP1, PP8 | PP1, PP8 | NA |
| 8K | PP8, PP4, PP5 | PP8, PP4, PP5 | PP3, PP8 | PP3, PP8 | PP1, PP8 | PP1, PP8 | NA |
| 4K, 2K | NA | PP4, PP5 | PP3 | NA | PP1 | NA | NA |
| 1K | NA | NA | PP3 | NA | PP1 | NA | NA |

NOTE 3: For the 32K case (SISO or MISO), it is not expected that a receiver will need to implement linear temporal interpolation of the pilots over more than 2 OFDM symbols. For all other cases, a maximum of four symbols of linear temporal interpolation are assumed. For the pilot pattern PP8, it is assumed that a receiver will use a "zero-order-hold" technique, although other more advanced techniques may be used if desired.

NOTE 4: When the value DX·DY (with DX and DY taken from table 57) is less than the reciprocal of the guard interval fraction, it is assumed that frequency only interpolation will be used in SISO mode, and hence the frame closing symbol is also not required.

NOTE 5: The allowed combinations of scattered pilot pattern, FFT size and guard interval are modified for T2-Lite — see annex I.

