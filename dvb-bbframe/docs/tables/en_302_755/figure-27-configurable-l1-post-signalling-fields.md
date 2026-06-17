## Figure 27 — Configurable L1-post signalling fields
_§7.2.3.1, PDF p. 67 — the configurable part stays constant for the duration of one super-frame._

Header, then three loops (RF / FEF-conditional / PLP), then post-loop fields, then the AUX loop.

| # | Field | Bits | Presence | Notes |
|---|---|---|---|---|
| 1 | SUB_SLICES_PER_FRAME | 15 | always | Nsubslices_total, total sub-slices for type-2 PLPs in one T2-frame |
| 2 | NUM_PLP | 8 | always | Number of PLPs in the T2 system |
| 3 | NUM_AUX | 4 | always | Number of auxiliary streams |
| 4 | AUX_CONFIG_RFU | 8 | always | Reserved for future auxiliary-stream configuration use |
| | **RF loop** | | `for i = 0..NUM_RF−1` | NUM_RF from the L1-pre |
| 5 | RF_IDX | 3 | per RF | Unique index of this frequency (0..NUM_RF−1) |
| 6 | FREQUENCY | 32 | per RF | Centre frequency in Hz; '0' = unknown (do not interpret as a frequency) |
| | **FEF block** | | `if S2 == 'xxx1'` (LSB of S2 = 1) | |
| 7 | FEF_TYPE | 4 | FEF present | FEF part type — Table 29 |
| 8 | FEF_LENGTH | 22 | FEF present | FEF length in elementary periods T; T2-Lite extends with FEF_LENGTH_MSB (24-bit effective) |
| 9 | FEF_INTERVAL | 8 | FEF present | Number of T2-frames between two FEF parts |
| | **PLP loop** | | `for i = 0..NUM_PLP−1` | |
| 10 | PLP_ID | 8 | per PLP | Unique PLP identifier |
| 11 | PLP_TYPE | 3 | per PLP | Table 30 |
| 12 | PLP_PAYLOAD_TYPE | 5 | per PLP | Table 31 |
| 13 | FF_FLAG | 1 | per PLP | TFS type-1 PLP frame flag; 0 (no meaning) when TFS unused or PLP_TYPE≠'001' |
| 14 | FIRST_RF_IDX | 3 | per PLP | RF channel of a type-1 PLP in the first frame; 0 when TFS unused or PLP_TYPE≠'001' |
| 15 | FIRST_FRAME_IDX | 8 | per PLP | FRAME_IDX of the first frame the PLP occurs in; < FRAME_INTERVAL |
| 16 | PLP_GROUP_ID | 8 | per PLP | PLP group association (links data PLP to its common PLP) |
| 17 | PLP_COD | 3 | per PLP | Code rate — Table 32 |
| 18 | PLP_MOD | 3 | per PLP | Modulation — Table 33 |
| 19 | PLP_ROTATION | 1 | per PLP | '1' = constellation rotation used |
| 20 | PLP_FEC_TYPE | 2 | per PLP | Table 34 |
| 21 | PLP_NUM_BLOCKS_MAX | 10 | per PLP | Max number of FEC blocks for this PLP |
| 22 | FRAME_INTERVAL | 8 | per PLP | IJUMP, T2-frame interval within the super-frame for this PLP |
| 23 | TIME_IL_LENGTH | 8 | per PLP | Time-interleaving length (meaning depends on TIME_IL_TYPE) |
| 24 | TIME_IL_TYPE | 1 | per PLP | Time-interleaving type |
| 25 | IN_BAND_A_FLAG | 1 | per PLP | '1' = in-band signalling type A present in this PLP |
| 26 | IN_BAND_B_FLAG | 1 | per PLP | '1' = in-band signalling type B present (must be 0 if T2_VERSION='0000') |
| 27 | RESERVED_1 | 11 | per PLP | Reserved; sometimes bias-balancing |
| 28 | PLP_MODE | 2 | per PLP | Table 35 ('00' only valid if T2_VERSION='0000') |
| 29 | STATIC_FLAG | 1 | per PLP | '1' = dynamic fields static for this PLP across the super-frame |
| 30 | STATIC_PADDING_FLAG | 1 | per PLP | '1' = padding static for this PLP |
| | **post-PLP-loop** | | | |
| 31 | FEF_LENGTH_MSB | 2 | always | 2 MSBs of FEF_LENGTH for T2-Lite; reserved for T2-base (0 if T2_VERSION='0000') |
| 32 | RESERVED_2 | 30 | always | Reserved; sometimes bias-balancing |
| | **AUX loop** | | `for i = 0..NUM_AUX−1` | |
| 33 | AUX_STREAM_TYPE | 4 | per AUX | Table 36 |
| 34 | AUX_PRIVATE_CONF | 28 | per AUX | Auxiliary-stream-specific configuration (meaning per AUX_STREAM_TYPE) |

