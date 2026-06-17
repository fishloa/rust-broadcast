## Table 35 — Signalling format for the PLP_MODE
_§7.2.3.1, PDF p. 70_

| Value | PLP mode |
|---|---|
| 00 | Not specified |
| 01 | Normal Mode |
| 10 | High Efficiency Mode |
| 11 | Reserved for future use |

NOTE: The value '00' shall only be used if T2_VERSION in the L1-pre signalling is set to '0000' (see clause 7.2.2). The value '00' is retained for backward compatibility with previous versions of the present document and indicates that the mode is signalled only in the CRC-8/MODE field of the BBHEADER.

