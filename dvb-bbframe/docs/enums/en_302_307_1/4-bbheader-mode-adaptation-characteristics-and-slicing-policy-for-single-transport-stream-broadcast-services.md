## Table 4 — BBHeader (Mode Adaptation characteristics) and Slicing Policy for Single Transport Stream Broadcast services
_§5.1.6, PDF pp. 21-21_

| Application area/configuration | MATYPE-1 | MATYPE-2 | UPL | DFL | SYNC | SYNCD | CRC-8 | Slicing policy |
|---|---|---|---|---|---|---|---|---|
| Broadcasting services / CCM, single TS | 11-1-1-0-0-Y | XXXXXXXX | 188D×8 | Kbch −80D | 47HEX | Y | Y | Break; No timeout; No Padding; No Dummy frame |

X = not defined; Y = according to configuration/computation.

Break = break packets in subsequent DATAFIELDs; Timeout: maximum delay in merger/slicer buffer.

> **Spec note:** MATYPE-2 is exactly 8 X's (`XXXXXXXX`), matching the 8-bit field width. The single TS/CCM case reserves the whole byte (ISI not applicable).

---

