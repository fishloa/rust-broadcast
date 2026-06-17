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

