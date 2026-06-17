## Table 14 — Syntax of the common_descriptor_loop()
_§9.4.2.0, PDF pp. 23-23_

| Syntax | No. of | Identifier |
|---|---|---|
| | bits | |
| common_descriptor_loop() { | | |
| reserved | 4 | bslbf |
| common_descriptor_loop_length | 12 | uimsbf |
| for (i=0; i<N; i++) { | | |
| descriptor() | | |
| } | | |
| } | | |

