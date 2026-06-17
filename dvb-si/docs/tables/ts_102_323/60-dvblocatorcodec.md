## Table 60 — DVBLocatorCodec
_§9.4.3.4.3, PDF pp. 66-66_

| Name | No. of bits | Identifier |
|---|---|---|
| DVBLocatorCodec() { | | |
| optimized codec flag | 1 | bslbf |
| if (optimized codec flag == 1) { | | |
| OptimizedDVBLocator() | | |
| } else { | | |
| dvbStringCodec() | | |
| } | | |
| } | | |

