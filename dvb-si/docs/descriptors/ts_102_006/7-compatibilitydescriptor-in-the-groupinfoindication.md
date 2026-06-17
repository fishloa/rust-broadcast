## Table 7 — CompatibilityDescriptor in the GroupInfoIndication
_§8.1.1, PDF pp. 16-16_

| Syntax | No. of | Remarks |
|---|---|---|
| | bytes | |
| CompatibilityDescriptor() { | | |
| CompatibilityDescriptorLength | 2 | |
| DescriptorCount | 2 | |
| for (i=0; i<N; i++) { | | |
| descriptorType | 1 | see Table 8 |
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

