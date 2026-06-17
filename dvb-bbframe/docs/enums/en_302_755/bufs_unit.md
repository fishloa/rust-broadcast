## Annex C — Table C.1: ISSY field coding (hand-transcribed)
_§C.1.0, Table C.1; verbatim from `specs/etsi_en_302_755_v01.04.01_dvb_t2.pdf` (2026-06-13)_

The geometry extractor mangles Table C.1 (a packed bit-field matrix); this is the
**verbatim** DVB-T2 ISSY coding — the definitive source for the dvb-bbframe ISSY
decoder (`issy.rs`). (DVB-S2 reuses this layout but differs in the signalling
sub-codes — see `en_302_307_1_s2.md` Annex D / Table D.1.)

ISSY is 2 or 3 bytes. First byte `bit-7` selects ISCRshort (`0`) vs the long forms
(`1`); when `bit-7=1`, `bit-6` selects ISCRlong (`0`) vs signalling (`1`). When
`bit-7=1` and `bit-6=1`, `bit-5..bit-4` select the signalling kind:

| `[5:4]` | Kind | `[3:2]` | `[1:0]` + 2nd byte | 3rd byte |
|---|---|---|---|---|
| `00` | **BUFS** | BUFS unit: `00`=bits, `01`=Kbits, `10`=Mbits, **`11`=8Kbits** | 2 MSBs of BUFS, then next 8 bits of BUFS (BUFS = 10 bits) | not present when ISCRshort; else reserved |
| `01` | **TTO** | 4 MSBs of TTO_E (spanning `[3:0]`) | 2nd byte `bit-7` = LSB of TTO_E; `bit-6..0` = TTO_M (7 bits) | TTO_L (8 bits); not present when ISCRshort (TTO_L = 0) |
| others | reserved | reserved | reserved | reserved |

Plain rows (`bit-7..bit-6`): `0` → ISCRshort (15-bit ISCR over `bit-6..0` + 2nd byte);
`10` → ISCRlong (22-bit ISCR over `bit-5..0` + 2nd + 3rd byte).

TTO is signalled in mantissa+exponent form: **TTO = (TTO_M + TTO_L/256) × 2^TTO_E**,
with TTO_E 5 bits, TTO_M 7 bits, TTO_L 8 bits (TTO_L = 0 when ISCRshort is used).
TTO gives the time, in units of T (the elementary period, §9.5), to output the first UP.

**T2 vs S2 (definitive diff):** DVB-T2 (above) uses `[5:4]=01` for **TTO** and defines
BUFS unit `11 = 8Kbits`. The ISSY code range `0xEXXXXX` (i.e. `[5:4]=10`) **shall not be
transmitted in DVB-T2** — in DVB-S2 (EN 302 307-1 Annex D Table D.1) that range carries
**BUFSTAT**, which DVB-T2 replaces with TTO. `dvb-bbframe`'s `SignallingKind::Tto`/`BufStat`
decode both: `[5:4]=01`→TTO (T2), `[5:4]=10`→BUFSTAT (S2).
