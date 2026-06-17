## Table 61 — OptimizedDVBLocator
_§9.4.3.4.3, PDF pp. 67-67_

| Name | No. of bits | Identifier |
|---|---|---|
| OptimizedDVBLocator() { | | |
| prefix flag | 1 | bslbf |
| if (prefix flag ==1) { | | |
| DVBLocatorPrefix() | | |
| } | | |
| ctag flag | 1 | bslbf |
| if (ctag flag ==1) { | | |
| for (i=0;i<N;i++) { | | |
| component tag | 8 | uimsbf |
| ctag continue | 1 | bslbf |
| } | | |
| } | | |
| cid flag | 1 | bslbf |
| if (cid flag ==1) { | | |
| carousel id | 32 | uimsbf |
| } | | |
| eventOrTVAflag | 2 | bslbf |
| if (eventOrTVAflag == 01) { | | |
| event id | 16 | uimsbf |
| } | | |
| else if (eventOrTVAflag == 10) { | | |
| TVA id | 16 | uimsbf |
| } | | |
| time flag | 1 | bslbf |
| if (time flag = 1){ | | |
| day flag | 1 | bslbf |
| if (day flag = 1) { | | |
| day | 16 | uimsbf |
| } | | |
| time | 17 | |
| duration | 17 | |
| } | | |
| path segments flag | 1 | bslbf |
| if (path segments flag ==1) { | | |
| path segments() | | |
| } | | |
| } | | |

