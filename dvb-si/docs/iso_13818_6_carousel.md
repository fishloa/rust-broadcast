# ISO/IEC 13818-6 DSM-CC download protocol — carousel message syntax

**Provenance.** ISO/IEC 13818-6:1998 is a paid ISO standard and cannot be
vendored into this repository. The message layouts below are hand-transcribed
from the well-known DSM-CC U-N download protocol (ISO/IEC 13818-6 §7.2
message headers, §7.3 download messages, §6.1 compatibilityDescriptor) and are
**not** backed by an in-repo PDF. They are instead cross-checked against three
independent in-repo grounds:

1. **A live capture** — `dvb-si/tests/fixtures/m6-single.ts` (French TNT, M6
   HbbTV carousel, PID 0x00AB) carries a complete DSI and DII, plus the start
   of a DDB (the capture ends mid-section), whose bytes decode exactly per
   these layouts (protocolDiscriminator 0x11, dsmccType 0x03, messageId
   0x1006/0x1002/0x1003, the TR 101 202 §4.7.9 transactionId rules, 20×0xFF
   serverId, DII↔DDB downloadId linkage from the DDB header prefix). The
   `carousel_fixture` integration tests pin this; full DDB/module reassembly
   is exercised end-to-end with synthetic sections built by our serializers.
2. **TR 101 202 v1.2.1** (vendored) — §4.6/§4.7.5 field semantics + DVB
   guidelines, Table 4.1 (transactionId sub-fields), Table 4.1a / 4.16
   (DSM-CC section field encoding).
3. **TS 102 006 v1.7.1** (vendored) — Table 15 reproduces the
   `compatibilityDescriptor()` structure; Table 6 (GroupInfoIndication) is the
   SSU profile's DSI `privateData`.

Carriage: U-N control messages (DSI, DII) ride DSM-CC sections with
**table_id 0x3B**; download data messages (DDB) ride **table_id 0x3C**
(EN 301 192 / TR 101 202 Table 4.1a). The section framing is `tables/dsmcc.rs`;
the payload parsing is `carousel/`.

## dsmccMessageHeader() — §7.2.2 (U-N control messages, table_id 0x3B)

| Field | Bits | Value / notes |
|---|---|---|
| protocolDiscriminator | 8 | 0x11 |
| dsmccType | 8 | 0x03 = U-N download message |
| messageId | 16 | 0x1002 = DII, 0x1006 = DSI |
| transactionId | 32 | TR 101 202 Table 4.1: bit 31 = updated flag (non-zero originator), bits 30..16 identification, bits 15..0 version. DVB: DSI uses 0x0000 in the 2 LSBs, DII non-zero (§4.7.9) |
| reserved | 8 | 0xFF |
| adaptationLength | 8 | bytes of adaptation header |
| messageLength | 16 | bytes after this field (adaptation + payload) |
| dsmccAdaptationHeader() | 8×adaptationLength | kept raw |

## dsmccDownloadDataHeader() — §7.2.4 (data messages, table_id 0x3C)

| Field | Bits | Value / notes |
|---|---|---|
| protocolDiscriminator | 8 | 0x11 |
| dsmccType | 8 | 0x03 |
| messageId | 16 | 0x1003 = DDB |
| downloadId | 32 | links DDBs to the DII that describes their modules |
| reserved | 8 | 0xFF |
| adaptationLength | 8 | |
| messageLength | 16 | bytes after this field |
| dsmccAdaptationHeader() | 8×adaptationLength | kept raw |

## DownloadServerInitiate (DSI) — §7.3.6, messageId 0x1006

| Field | Bits | Value / notes |
|---|---|---|
| serverId | 20×8 | DVB: all 20 bytes 0xFF (TR 101 202 §4.7.5.2; confirmed in the live capture) |
| compatibilityDescriptor() | var | 16-bit length + body, kept raw (TS 102 006 Table 15) |
| privateDataLength | 16 | |
| privateData | 8×N | kept raw — SSU: GroupInfoIndication (TS 102 006 Table 6); object carousel: ServiceGatewayInfo (TR 101 202 Table 4.15) |

## DownloadInfoIndication (DII) — §7.3.3, messageId 0x1002

| Field | Bits | Value / notes |
|---|---|---|
| downloadId | 32 | matches DDB downloadId |
| blockSize | 16 | bytes per DDB block (all but the last); live capture: 4066 |
| windowSize | 8 | DVB: 0 |
| ackPeriod | 8 | DVB: 0 |
| tCDownloadWindow | 32 | DVB: 0 |
| tCDownloadScenario | 32 | |
| compatibilityDescriptor() | var | 16-bit length + body, kept raw |
| numberOfModules | 16 | |
| per module: moduleId | 16 | |
| per module: moduleSize | 32 | total module bytes |
| per module: moduleVersion | 8 | |
| per module: moduleInfoLength | 8 | |
| per module: moduleInfo | 8×N | kept raw (object carousel: BIOP::ModuleInfo, TR 101 202 Table 4.14) |
| privateDataLength | 16 | |
| privateData | 8×N | kept raw |

## DownloadDataBlock (DDB) — §7.3.7.1, messageId 0x1003

The dsmccDownloadDataHeader is followed by:

| Field | Bits | Value / notes |
|---|---|---|
| moduleId | 16 | |
| moduleVersion | 8 | must match the DII entry |
| reserved | 8 | 0xFF |
| blockNumber | 16 | block index; byte offset = blockNumber × blockSize |
| blockData | to end | `messageLength − adaptationLength − 6` bytes |

## Module reassembly

A module is complete when, for the (downloadId, moduleId, moduleVersion) triple
announced by a DII entry, every block `0..ceil(moduleSize / blockSize)` has
been received; the final block carries `moduleSize − (nBlocks−1)×blockSize`
bytes. Implemented by `carousel::ModuleReassembler`.

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

## Grounded extracts — ISO/IEC 13818-6:1998/Amd.2:2000(E)

The following tables are transcribed from the freely-previewable Amendment 2
document (iTeh preview,
`https://cdn.standards.iteh.ai/samples/33123/9607da01e8074ee4acaed1331ec3fc27/ISO-IEC-13818-6-1998-Amd-2-2000.pdf`;
the PDF itself is not redistributable and is deliberately NOT vendored).
These are the authoritative ISO citations for this module's constants.

### Table 2-2 — MPEG-2 DSM-CC dsmccType values (as replaced by Amd.2)

| dsmccType | Description |
|---|---|
| 0x00 | ISO/IEC 13818-6 Reserved |
| 0x01 | U-N configuration message |
| 0x02 | U-N session message |
| **0x03** | **Download message** ← `DSMCC_TYPE_UN_DOWNLOAD` |
| 0x04 | SDB Channel Change Protocol message |
| 0x05 | U-N pass-thru message |
| 0x06 | SMPTE 325M Opportunistic Flow Control Protocol |
| 0x07–0x7F | ISO/IEC 13818-6 Reserved |
| 0x80–0xFF | User Defined |

### Table 2-5 — DSM-CC adaptationTypes (as replaced by Amd.2)

First byte of the dsmccAdaptationHeader (when `adaptationLength > 0`):

| adaptationType | Description |
|---|---|
| 0x00 | ISO/IEC 13818-6 Reserved |
| 0x01 | DSM-CC Conditional Access adaptation format |
| 0x02 | DSM-CC UserID adaptation format |
| 0x03 | ISO/IEC 13818-6 Reserved |
| 0x04 | DSM-CC Synchronized Download Protocol adaptation format |
| 0x05–0x7F | ISO/IEC 13818-6 Reserved |
| 0x80–0xFF | User Defined |

### Table 2-9 — Synchronized Download Protocol adaptation format (Amd.2 §2.1.4)

| Field | Bits |
|---|---|
| reserved | 16 |
| '0010' | 4 |
| PTS[32..30] | 3 |
| marker_bit ('1') | 1 |
| PTS[29..15] | 15 |
| marker_bit ('1') | 1 |
| PTS[14..0] | 15 |
| marker_bit ('1') | 1 |

PTS coded as in H.222.0; present only in the DSMCC_section conveying block 0
of a module (Amd.2 §9.2.8). We keep the adaptation header raw — this table
documents what a consumer may find in it.

### table_id confinement (Amd.2 item 7, §9.2.3)

"Only DSMCC_sections with table_id 0x3B or 0x3C shall be contained within
Transport Stream packets of stream_type 0x14" — the ISO-side confirmation of
the 0x3B (control) / 0x3C (data) split this module relies on.
