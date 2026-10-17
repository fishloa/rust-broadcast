## Table 53 — Syntax of module_link_descriptor
_§10.2.5, PDF pp. 66-66_

| module_link_descriptor(){ | No. of bytes | Value |
|---|---|---|
| descriptor_tag | 1 | 0x04 |
| descriptor_length | 1 |  |
| position | 1 |  |
| module_id | 2 |  |
| } |  |  |

Semantics (EN 301 192 V1.7.1 §10.2.5, PDF p.66; verified on rendered page):

- position: "this is an 8-bit field identifying the position of this module in the chain. The value of 0x00 shall indicate the first module of the list. The value of 0x01 indicates an intermediate module in the list and the value of 0x02 indicates the last module of the list."
- module_id: "this is a 16-bit field that identifies the next module in the list. This field shall be ignored for the last value in the list."
- Values 0x03..=0xFF of `position` are not defined by the page.

