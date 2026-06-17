## DownloadServerInitiate (DSI) — §7.3.6, messageId 0x1006

| Field | Bits | Value / notes |
|---|---|---|
| serverId | 20×8 | DVB: all 20 bytes 0xFF (TR 101 202 §4.7.5.2; confirmed in the live capture) |
| compatibilityDescriptor() | var | 16-bit length + body, typed as `compatibility::CompatibilityDescriptor` (TS 102 006 Table 15) |
| privateDataLength | 16 | |
| privateData | 8×N | kept raw — SSU: GroupInfoIndication (TS 102 006 Table 6); object carousel: ServiceGatewayInfo (TR 101 202 Table 4.15) |

