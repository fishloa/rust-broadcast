## Figure 25 — L1-pre signalling fields
_§7.2.2, PDF p. 62 — fixed layout. 168 information bits + 32-bit CRC = 200 bits total._

Wire order, MSB-first within each field, no padding between fields.

| # | Field | Bits | Notes |
|---|---|---|---|
| 1 | TYPE | 8 | Tx input stream types carried in the super-frame — Table 21 |
| 2 | BWT_EXT | 1 | '1' = extended carrier mode (8K/16K/32K FFT only); see §9.5 |
| 3 | S1 | 3 | Same value as the P1 signalling — Table 18 |
| 4 | S2 | 4 | Same value as the P1 signalling: S2 field 1 (3 bits, Table 19a/19b) + S2 field 2 (1 bit, Table 20) |
| 5 | L1_REPETITION_FLAG | 1 | '1' = dynamic L1-post for the next frame is also present in this frame (§7.2.3.3) |
| 6 | GUARD_INTERVAL | 3 | Guard interval of the current super-frame — Table 22 |
| 7 | PAPR | 4 | PAPR reduction in use — Table 23a (T2_VERSION='0000') / Table 23b (T2_VERSION>'0000') |
| 8 | L1_MOD | 4 | Constellation of the L1-post data block — Table 24 |
| 9 | L1_COD | 2 | Coding of the L1-post data block — Table 25 |
| 10 | L1_FEC_TYPE | 2 | L1 FEC type for the L1-post data block — Table 26 |
| 11 | L1_POST_SIZE | 18 | Size of the coded+modulated L1-post data block, in OFDM cells |
| 12 | L1_POST_INFO_SIZE | 18 | Size of the L1-post information part, in bits, incl. extension but excl. CRC (Kpost_ex_pad = this + 32) |
| 13 | PILOT_PATTERN | 4 | Scattered pilot pattern for data symbols — Table 27 |
| 14 | TX_ID_AVAILABILITY | 8 | Availability of transmitter-identification signals in the cell; 0x00 = none |
| 15 | CELL_ID | 16 | Geographic cell identifier; '0' if not provided |
| 16 | NETWORK_ID | 16 | DVB network identifier |
| 17 | T2_SYSTEM_ID | 16 | T2 system identifier within the network |
| 18 | NUM_T2_FRAMES | 8 | NT2, number of T2-frames per super-frame; minimum 2 |
| 19 | NUM_DATA_SYMBOLS | 12 | Ldata = LF − NP2, data OFDM symbols per T2-frame, excl. P1/P2 |
| 20 | REGEN_FLAG | 3 | Number of times the signal has been regenerated; '000' = none |
| 21 | L1_POST_EXTENSION | 1 | '1' = L1-post extension field present (§7.2.3.4) |
| 22 | NUM_RF | 3 | NRF, number of frequencies in the current T2 system |
| 23 | CURRENT_RF_IDX | 3 | Index of the current RF channel within the TFS structure (0..NUM_RF−1); '0' if TFS not used |
| 24 | T2_VERSION | 4 | Latest spec version the signal is based on — Table 28 |
| 25 | L1_POST_SCRAMBLED | 1 | '1' = L1-post is scrambled (§7.3.2.1). Reliable only if T2_VERSION≥'0010'; otherwise reserved/bias-balancing |
| 26 | T2_BASE_LITE | 1 | T2-base: '1' = signal is T2-Lite-compatible. T2-Lite: reserved (not bias-balanced). Reliable only if T2_VERSION≥'0010' |
| 27 | RESERVED | 4 | Reserved for future use; sometimes used for bias balancing (§7.2.3.7) |
| 28 | CRC_32 | 32 | Error detection over the entire L1-pre signalling — CRC-32/MPEG-2 (annex F) |

When `T2_VERSION = '0000'`, the fields `L1_POST_SCRAMBLED`, `T2_BASE_LITE`, `IN_BAND_B_FLAG`, `PLP_MODE`, `STATIC_FLAG`, `STATIC_PADDING_FLAG` and `FEF_LENGTH_MSB` shall all be set to 0. When `T2_VERSION = '0001'`, `L1_POST_SCRAMBLED`, `T2_BASE_LITE` and `FEF_LENGTH_MSB` are reserved (sometimes used for bias balancing).

