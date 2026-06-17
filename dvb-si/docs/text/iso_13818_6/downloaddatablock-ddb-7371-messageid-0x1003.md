## DownloadDataBlock (DDB) — §7.3.7.1, messageId 0x1003

The dsmccDownloadDataHeader is followed by:

| Field | Bits | Value / notes |
|---|---|---|
| moduleId | 16 | |
| moduleVersion | 8 | must match the DII entry |
| reserved | 8 | 0xFF |
| blockNumber | 16 | block index; byte offset = blockNumber × blockSize |
| blockData | to end | `messageLength − adaptationLength − 6` bytes |

