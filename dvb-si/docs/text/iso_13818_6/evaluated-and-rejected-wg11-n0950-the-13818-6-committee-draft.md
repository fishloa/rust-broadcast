## Evaluated and REJECTED: WG11 N0950 (the 13818-6 committee draft)

A widely-mirrored 325-page PDF (`courses.e-ce.uth.gr/CE401/.../13818-6.pdf`,
ISO/IEC JTC1/SC29/WG11 **N0950**) is the pre-final DSM-CC *committee draft*,
not the 1998 IS — and its download protocol differs from the final standard.
Do NOT "correct" this module against it. Evidence (from the draft itself):

- draft `dsmccMessageHeader`: `protocolDiscriminator(1) · dsmccType(1) ·
  transactionId(8!) · messageId(2) · adaptationLength(1) · messageLength(2)`
  with the protocolDiscriminator value literally "[tbd]" — vs the final
  12-byte header this module implements.
- draft dsmccType table: 0x03 = "U-U configuration message" — vs Amd.2:2000
  Table 2-2 (final): 0x03 = "Download message", which is what the live M6
  capture carries (`11 03 ...`).
- draft `DownloadDataBlock`: `messageId 0x0004 · downloadTransactionId(2) ·
  moduleNumber(1) · blockNumber(2) · checksum(4)` — vs the final
  `messageId 0x1003 · downloadId(32 in header) · moduleId(16) ·
  moduleVersion(8) · reserved(8) · blockNumber(16)` observed on air.

