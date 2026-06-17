## Payload layouts (figures)

The tables above cover all numbered tables in the spec (Tables 1–19). The per-payload
bit layouts are defined in the spec **figures** (not numbered tables), transcribed here
verbatim from the PDF. These are the field widths the `dvb-t2mi` parsers implement.

| Packet type | § / Fig | Payload layout (field — bits) |
|---|---|---|
| 0x00 Baseband Frame | §5.2.1, Fig 4 | frame_idx(8) · plp_id(8) · intl_frame_start(1) · rfu(7) · BBFRAME(K_bch) |
| 0x01 Auxiliary I/Q | §5.2.2, Fig 5 | frame_idx(8) · aux_id(4) · rfu(12) · aux_stream_data(var) |
| 0x02 Arbitrary cell | §5.2.3, Fig 6 | frame_idx(8) · tx_identifier(16) · rfu(18) · start_cell_address(22) · arbitrary_cell_data(var) |
| 0x10 L1-current | §5.2.4, Fig 7 | frame_idx(8) · freq_source(2) · rfu(6) · L1-current_data(var) |
| 0x11 L1-future | §5.2.5, Fig 8 | frame_idx(8) · rfu(8) · L1-future_data(var) |
| 0x12 P2 bias | §5.2.6, Fig 9 | frame_idx(8) · rfu(17) · num_active_bias_cells_per_p2(15) |
| 0x20 T2 timestamp | §5.2.7, Fig 10 | rfu(4) · bw(4) · seconds_since_2000(40) · subseconds(27) · utco(13) |
| 0x21 Individual addressing | §5.2.8, Fig 11 | rfu(8) · individual_addressing_length(8) · individual_addressing_data(var) |
| 0x30 FEF part: Null | §5.2.9, Fig 12 | fef_idx(8) · rfu(9) · s1_field(3) · s2_field(4) |
| 0x31 FEF part: I/Q | §5.2.10, Fig 13 | fef_idx(8) · rfu(9) · s1_field(3) · s2_field(4) · fef_part_data(var) |
| 0x32 FEF part: composite | §5.2.11, Fig 15 | fef_idx(8) · rfu1(1) · s1_field(3) · s2_field(4) · rfu2(32) · num_subparts(16) |
| 0x33 FEF sub-part | §5.2.12, Fig 16 | fef_idx(8) · tx_identifier(16) · rfu1(32) · subpart_idx(16) · subpart_variety(16) · rfu2(10) · subpart_length(22) · subpart(var) |
