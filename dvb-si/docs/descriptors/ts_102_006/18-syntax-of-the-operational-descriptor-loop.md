## Table 18 — Syntax of the operational_descriptor_loop()
_§9.5.2, PDF pp. 25-25_

| Syntax | No. of | Identifier |
|---|---|---|
| | bits | |
| operational_descriptor_loop() { | | |
| reserved | 4 | bslbf |
| operational_descriptor_loop_length | 12 | uimsbf |
| for (i=0; i<N; i++) { | | |
| operational_descriptor() | | |
| } | | |
| } | | |

