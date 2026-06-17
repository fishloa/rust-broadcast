## Table 3 — L1-future data fields
_§5.2.5, PDF p.19_

| Field | Field length (bits) | Format | Description |
|---|---|---|---|
| L1DYN_NEXT_LEN | 16 | uimsbf | Length of "dynamic, next frame" field. Set to zero if L1DYN_NEXT block is absent. |
| L1DYN_NEXT | 8×⌈L1DYN_NEXT_LEN/8⌉ | bflbfzpb | L1-post "dynamic, next frame" fields. Optional in single-RF mode, mandatory in TFS. |
| L1DYN_NEXT2_LEN | 16 | uimsbf | Length of "dynamic, next-but-one frame" in TFS mode. Set to zero if L1DYN_NEXT2 block is absent. |
| L1DYN_NEXT2 | 8×⌈L1DYN_NEXT2_LEN/8⌉ | bflbfzpb | L1-post "dynamic, next-but-one frame" fields, in the order defined in clause 7.2.3.2 of ETSI EN 302 755 [1]. Optional in TFS, and shall not be present in single-RF mode. |
| NUM_INBAND | 8 | uimsbf | Number of PLPs for which in-band signalling is present in the following loop. |
| For i=1..NUM_INBAND { | | | In-band signalling loop. |
| PLP_ID | 8 | uimsbf | PLP ID for the PLP containing the in-band signalling data given by the following INBAND field. |
| INBAND_LEN | 16 | | Length of following INBAND field in bits. |
| INBAND | 8×⌈INBAND_LEN/8⌉ | bflbfzpb | In-band signalling fields for the PLP indicated by PLP_ID, in the order defined in clause 5.2.3 of ETSI EN 302 755 [1]. |
| } | | | |

