## Table 4.17a — FormatSpecifier in the carousel_identifier_descriptor
_§4.7.7.1, PDF p. 46 — bit-widths read from the page-46 PDF render (the
`pdftotext` bits column was misaligned; verified against the rendered table)._

| FormatId | FormatSpecifier | Notes |
|---|---|---|
| `0x00` | (none) | ServiceGateway located only via the DSI/DII messages |
| `0x01` | the aggregated ServiceGateway-location fields below | all `uimsbf` |
| `0x02`–`0x7F` | reserved (DVB) | |
| `0x80`–`0xFF` | reserved (private) | |

`FormatId = 0x01` FormatSpecifier (16 bytes fixed + ObjectKeyData):

| Field | bits | Comment |
|---|---|---|
| `ModuleVersion` | 8 | |
| `ModuleId` | 16 | |
| `BlockSize` | 16 | |
| `ModuleSize` | 32 | |
| `CompressionMethod` | 8 | |
| `OriginalSize` | 32 | |
| `TimeOut` | 8 | seconds — **NB:** TR 101 202 v1.2.1 shows 8 bits here; the later canonical carousel_identifier_descriptor (TS 102 809 / EN 301 192) uses a 32-bit TimeOut. This crate follows the vendored TR 101 202 (8-bit). |
| `ObjectKeyLength` | 8 | N1 |
| `ObjectKeyData` × N1 | 8 each | object key of the ServiceGateway object |
