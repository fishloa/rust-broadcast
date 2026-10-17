## Table 57 — Syntax of group_link_descriptor
_§10.2.9, PDF pp. 67-67_

| group_link_descriptor(){ | No. of bytes | Value |
|---|---|---|
| descriptor_tag | 1 | 0x08 |
| descriptor_length | 1 |  |
| position | 1 |  |
| group_id | 4 |  |
| } |  |  |

Semantics (EN 301 192 V1.7.1 §10.2.9, PDF pp.67-68; verified on rendered pages):

- position: "this is an 8-bit field identifying the position of this group description in the chain. The value of 0x00 shall indicate the first group description of the list. The value of 0x01 indicates an intermediate group description in the list and the value of 0x02 indicates the last group description of the list."
- group_id: "this is a 32-bit field that identifies the next group description in the list. This field shall be ignored for the last value in the list."
- Values 0x03..=0xFF of `position` are not defined by the page.

