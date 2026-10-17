## Table 56 — Syntax of est_download_time_descriptor
_§10.2.8, PDF pp. 67-67_

| est_download_time_descriptor(){ | No. of bytes | Value |
|---|---|---|
| descriptor_tag | 1 | 0x07 |
| descriptor_length | 1 |  |
| est_download_time | 4 |  |
| } |  |  |

Semantics (EN 301 192 V1.7.1 §10.2.8, PDF p.67; verified on rendered page):

- est_download_time: "this 32-bit field gives the estimated download time of data in seconds." (unit: seconds)

