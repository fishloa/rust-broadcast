## Table 2 — Description of the fields of the BBHEADER
_§5.1.8, PDF p. 28_

| Field | Size (Bytes) | Description |
|---|---|---|
| MATYPE | 2 | As described above |
| UPL | 2 | User Packet Length in bits, in the range [0,65535] |
| DFL | 2 | Data Field Length in bits, in the range [0,53760] |
| SYNC | 1 | Copy of the User Packet sync-byte (or 0x00 if not applicable) |
| SYNCD | 2 | The distance in bits from the beginning of the DATA FIELD to the beginning of the first transmitted UP which starts in the data field. SYNCD=0D means that the first UP is aligned to the beginning of the Data Field. SYNCD = 65535D means that no UP starts in the DATA FIELD; for GCS, SYNCD is reserved for future use and shall be set to 0D unless otherwise defined. |
| CRC-8 MODE | 1 | The XOR of the CRC-8 (1-byte) field with the MODE field (1-byte). CRC-8 is the error detection code applied to the first 9 bytes of the BBHEADER (see annex F). MODE (8 bits) shall be: • 0D Normal Mode. • 1D High Efficiency Mode. • Other values: reserved for future use. |

