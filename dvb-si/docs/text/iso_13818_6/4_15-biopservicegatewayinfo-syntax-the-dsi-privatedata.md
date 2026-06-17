## Table 4.15 — BIOP::ServiceGatewayInfo syntax (the DSI `privateData`)
_§4.7.5.2, PDF p. 43 — carried in the DownloadServerInitiate `privateDataByte`_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| `IOP::IOR()` | | + | IOR of the ServiceGateway (Table 4.3) |
| `downloadTaps_count` | 8 | N1 | software-download Taps |
| `Tap()` × N1 | | + | semantics not defined by TR 101 202 (parse-to-raw) |
| `serviceContextList_count` | 8 | N2 | |
| per context: `context_id` | 32 | | |
| per context: `context_data_length` | 16 | N9 | |
| per context: `context_data_byte` × N9 | 8 each | + | |
| `userInfoLength` | 16 | N3 | |
| `userInfo_data_byte` × N3 | 8 each | + | descriptor loop |

In the DSI, the `serverId` is 20 bytes of `0xFF`, the `compatibilityDescriptor()`
is zero-length, and `privateDataLength` gives the byte count of this structure.
The `userInfo` field is a DVB/private descriptor loop. The `downloadTaps`/
`serviceContextList` semantics are not defined by TR 101 202, so they are parsed
to raw bytes (in practice `downloadTaps_count` is typically 0).

---

