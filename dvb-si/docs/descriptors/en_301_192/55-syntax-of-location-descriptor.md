## Table 55 — Syntax of location_descriptor
_§10.2.7, PDF pp. 67-67_

| location_descriptor(){ | No. of bytes | Value |
|---|---|---|
| descriptor_tag | 1 | 0x06 |
| descriptor_length | 1 |  |
| location_tag | 1 |  |
| } |  |  |

Semantics (EN 301 192 V1.7.1 §10.2.7, PDF p.67; verified on rendered page):

- location_tag: "this 8-bit field has the same value as the component_tag field in the stream identifier descriptor."

