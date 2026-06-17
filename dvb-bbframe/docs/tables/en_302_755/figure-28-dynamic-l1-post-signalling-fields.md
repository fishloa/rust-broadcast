## Figure 28 — Dynamic L1-post signalling fields
_§7.2.3.2, PDF p. 72 — the dynamic part may change from frame to frame._

| # | Field | Bits | Presence | Notes |
|---|---|---|---|---|
| 1 | FRAME_IDX | 8 | always | Index of the current T2-frame within the super-frame; first frame = '0' |
| 2 | SUB_SLICE_INTERVAL | 22 | always | OFDM cells between successive sub-slices of one type-2 PLP; '0' if no type-2 PLPs |
| 3 | TYPE_2_START | 22 | always | Start position of the first type-2 PLP (cell addressing); '0' if no type-2 PLPs |
| 4 | L1_CHANGE_COUNTER | 8 | always | Super-frames ahead until the configuration changes; '0' = no scheduled change |
| 5 | START_RF_IDX | 3 | always | RF index of the first frame of the next super-frame (TFS); '0' if TFS not used |
| 6 | RESERVED_1 | 8 | always | Reserved; sometimes bias-balancing |
| | **PLP loop** | | `for i = 0..NUM_PLP−1` | same PLP order as the configurable PLP loop |
| 7 | PLP_ID | 8 | per PLP | PLP identifier |
| 8 | PLP_START | 22 | per PLP | Start position of the PLP within the frame (cell addressing) |
| 9 | PLP_NUM_BLOCKS | 10 | per PLP | Number of FEC blocks for this PLP in the current frame |
| 10 | RESERVED_2 | 8 | per PLP | Reserved; sometimes bias-balancing |
| | **post-PLP-loop** | | | |
| 11 | RESERVED_3 | 8 | always | Reserved; sometimes bias-balancing |
| | **AUX loop** | | `for i = 0..NUM_AUX−1` | |
| 12 | AUX_PRIVATE_DYN | 48 | per AUX | Auxiliary-stream dynamic data (meaning per AUX_STREAM_TYPE) |

