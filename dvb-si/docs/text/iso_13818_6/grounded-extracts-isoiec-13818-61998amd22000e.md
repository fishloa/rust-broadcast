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
