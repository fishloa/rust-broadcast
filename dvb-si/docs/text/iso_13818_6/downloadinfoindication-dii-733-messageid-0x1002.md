## DownloadInfoIndication (DII) — §7.3.3, messageId 0x1002

| Field | Bits | Value / notes |
|---|---|---|
| downloadId | 32 | matches DDB downloadId |
| blockSize | 16 | bytes per DDB block (all but the last); live capture: 4066 |
| windowSize | 8 | DVB: 0 |
| ackPeriod | 8 | DVB: 0 |
| tCDownloadWindow | 32 | DVB: 0 |
| tCDownloadScenario | 32 | |
| compatibilityDescriptor() | var | 16-bit length + body, typed as `compatibility::CompatibilityDescriptor` |
| numberOfModules | 16 | |
| per module: moduleId | 16 | |
| per module: moduleSize | 32 | total module bytes |
| per module: moduleVersion | 8 | |
| per module: moduleInfoLength | 8 | |
| per module: moduleInfo | 8×N | kept raw (object carousel: BIOP::ModuleInfo, TR 101 202 Table 4.14) |
| privateDataLength | 16 | |
| privateData | 8×N | kept raw |

