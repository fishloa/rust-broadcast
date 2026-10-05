# dvb-t2mi 11.0.0

_Released 2026-10-05._

### Added

- `PacketType::is_allocated(byte)` — whether a `packet_type` byte is allocated in TS 102 773 Table 1 (the single definition the raw-mode resync uses).

### Changed (breaking)
- Serializers now return an error, instead of silently truncating, when a
  length or count does not fit its wire field (#1129).
- Serializers now return an error, instead of silently truncating, when a
  length or count does not fit its wire field (#1129).
- `payload::l1::pre::L1Pre::to_bytes`, `crc32`, and `serialize_with_crc`
  now return `Result`: every `L1Pre` field is public and mutable, so a
  caller-set value outside its wire bit width (e.g. `num_rf > 7`)
  previously panicked inside the bit writer's `expect` instead of being
  rejected (#1095, W-T2-3).

### Fixed
- `T2miPump` raw-mode resync no longer `debug_assert`s that the seeded reassembler frames nothing (fuzz found a seed that does); anything it frames is CRC-gated like the normal path (CI fuzz finding).
- `inner_ts::InnerTsRecovery` (used by `dvb-tools t2mi --inner` without
  `--plp`) ran every PLP's BBFrames through one shared `CarryOverExtractor`,
  so a user packet split across a BBFrame boundary was corrupted (merged
  with, or replaced by, another PLP's data) whenever a different PLP's frame
  arrived before the split completed. Carry-over state is now keyed per PLP,
  mirroring `dvb_bbframe::pump::BbframePump` (#1034).
- `packet::Header::serialize_into` no longer silently wraps an out-of-range
  `superframe_idx` (4-bit field) to 0; it now rejects it with
  `ReservedBitsViolation` (#1095, #1129).
- `payload::individual_addressing`'s per-function bodies (`ace_gain`,
  `ace_maximal_extension`, `ace_clipping_threshold`, `miso_group` rfu,
  `tr_papr` rfu1/`tr_clipping_threshold`/rfu2/`number_of_iterations`,
  `tx_sig_fef_seq_num` rfu1/`seq_num_1`/rfu2/`seq_num_2`/rfu3,
  `tx_sig_aux_stream_tx_id`/rfu, `rf_idx`/`frequency` rfu) no longer silently
  mask an out-of-range field on serialize; each is now rejected with
  `ReservedBitsViolation` (#1095, #1129).
- `payload::l1::post`'s framed-block writer no longer silently wraps a
  16-bit-or-larger bit-length into the 16-bit framed-block length field; it
  now rejects it with `ReservedBitsViolation` (#1095, #1129).
- Raw-mode (`T2miPump::raw`) resynchronisation after a CRC failure: the
  failed packet's own header-implied `payload_len_bits` may itself have
  been the corrupted field, and the pump previously trusted it forever —
  one corrupted length (a bit error, or a dropped UDP datagram carrying
  raw T2-MI over IP) desynchronised the stream permanently, with
  `Stats::crc_failures` climbing and no recovery path. The pump now scans
  forward for the next CRC-valid packet start (cheap header-plausibility
  gate first, full CRC-32 validation second) and resumes framing from it,
  with new `Stats::raw_resyncs` and `Stats::raw_hunt_bytes_discarded`
  counters; the hunt buffer is bounded so garbage cannot exhaust memory
  (#1095, W-T2-1).
- `T2miPump::feed_ts` no longer forwards every filtered-PID packet to the
  reassembler blindly: packets with `transport_error_indicator` set are
  dropped, and the one legal repeated packet ISO/IEC 13818-1 §2.4.3.3
  allows (byte-identical payload with an unchanged `continuity_counter`)
  is skipped instead of being appended twice — which used to turn a good
  T2-MI packet into a CRC failure. New `Stats::tei_dropped` and
  `Stats::cc_duplicates_skipped` counters (#1095, W-T2-2).
- `payload::l1::post::L1ExtBlock::write` no longer underflows
  `data_bit_len - done` when the (public, mutable) `data` vector is longer
  than its declared bit length implies — previously a debug panic, and in
  release a wrap that wrote bits beyond the declared length and misframed
  the L1EXT region. A `data`/`data_bit_len` mismatch is now rejected with
  `ReservedBitsViolation` (#1095, W-T2-5).

---

Published from tag `dvb-t2mi-v11.0.0`.
