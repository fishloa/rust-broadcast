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

