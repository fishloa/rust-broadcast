## Table 17 — Syntax of the target_descriptor_loop()
_§9.4.2.0, PDF pp. 24-24_

| Syntax | No. of | Identifier |
|---|---|---|
| | bits | |
| target_descriptor_loop() { | | |
| reserved | 4 | bslbf |
| target_descriptor_loop_length | 12 | uimsbf |
| for (i=0; i<N; i++) { | | |
| target_descriptor() | | |
| } | | |
| } | | |

