## Table 15 — Syntax of the compatibilityDescriptor() structure
_§9.4.2.0, PDF pp. 24-24_

| Syntax | No. of | Remarks |
|---|---|---|
| | bytes | |
| compatibilityDescriptor() { | | |
| compatibilityDescriptorLength | 2 | |
| descriptorCount | 2 | |
| for (i=0; i<N; i++) { | | |
| descriptorType | 1 | see Table 16 |
| descriptorLength | 1 | |
| specifierType | 1 | 0x01 (IEEE OUI) |
| specifierData | 3 | IEEE OUI as described in IEEE 802 [5] |
| model | 2 | zero if the model is transmitted in a manufacturer private location |
| version | 2 | zero if the version is transmitted in a manufacturer private location |
| subDescriptorCount | 1 | |
| for (i=0; i<N; i++) { | | |
| subDescriptor() | | |
| } | | |
| } | | |
| } | | |
| NOTE: Refer to ISO/IEC 13818-6 [1]. | | |

