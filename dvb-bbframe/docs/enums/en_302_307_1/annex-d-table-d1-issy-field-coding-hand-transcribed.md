## Annex D — Table D.1: ISSY field coding (hand-transcribed)
_§D.2, Table D.1; verbatim from `specs/etsi_en_302_307_1_v01.04.01_dvb_s2.pdf` (2026-06-12)_

The geometry extractor mangles Table D.1 (a packed bit-field matrix); this is the
**verbatim** DVB-S2 ISSY coding — the definitive source for the dvb-bbframe ISSY
decoder's BUFS path. (DVB-T2 reuses this layout but differs in the signalling
sub-codes — see `en_302_755_t2.md` Annex C / Table C.1.)

First byte `bit-7` selects ISCRshort (`0`) vs the long forms (`1`); when `bit-7=1`,
`bit-6` selects ISCRlong (`0`) vs signalling (`1`). When `bit-7=1` and `bit-6=1`,
`bit-5..bit-4` select the signalling kind:

| `[5:4]` | Kind | `[3:2]` | `[1:0]` + 2nd byte | 3rd byte |
|---|---|---|---|---|
| `00` | **BUFS** | BUFS unit: `00`=bits, `01`=Kbits, `10`=Mbits, **`11`=reserved** | 2 MSBs of BUFS, then next 8 bits of BUFS (BUFS = 10 bits) | not present when ISCRshort; else reserved |
| `10` | **BUFSTAT** | BUFSTAT unit: `00`=bits, `01`=Kbits, `10`=Mbits, `11`=reserved | 2 MSBs of BUFSTAT, then next 8 bits of BUFSTAT | not present when ISCRshort; else reserved |
| others | reserved | reserved | reserved | reserved |

NOTE: For Generic Packetized Streams the optional ISCR shall be limited to the
"short" format.

**S2 vs T2 (definitive diff):** DVB-S2 (above) uses `[5:4]=10` for **BUFSTAT** and
marks the BUFS unit `11` **reserved**. DVB-T2 (EN 302 755 Table C.1) **replaces
BUFSTAT with TTO** (`[5:4]=01 = TTO`, the `0xEXXXXX` code range "shall not be
transmitted in DVB-T2") and defines BUFS unit `11 = 8Kbits`. `dvb-bbframe`'s
`BufsUnit::Kbits8` and `SignallingKind::Tto` therefore follow the **T2** coding;
S2 streams will not use `11` (reserved) nor TTO.
