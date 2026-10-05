# dvb-t2mi 11.0.0

_Released 2026-10-05._

**Major (breaking), small surface.** One API break (`L1Pre::to_bytes`, `crc32` and `serialize_with_crc` now return `Result`) and a block of serializer fixes that turn silent truncation or masking into errors, plus a much more robust raw-mode `T2miPump`. If you call those three `L1Pre` methods you must add error handling; otherwise the upgrade is a version bump. Lockstep sibling of [dvb-bbframe 11.0.0](dvb-bbframe-11.0.0.md) (its optional `dvb-bbframe` dependency moves to 11), [dvb-si 11.0.0](dvb-si-11.0.0.md) and [dvb-tools 11.0.0](dvb-tools-11.0.0.md).

## Dependency changes

```toml
-broadcast-common = { version = "9.3", default-features = false }
+broadcast-common = { version = "9.4", default-features = false }
-dvb-bbframe      = { version = "10",  optional = true, default-features = false }
+dvb-bbframe      = { version = "11.0", optional = true, default-features = false }
```

`dvb-bbframe` crosses a major epoch; if you depend on both crates, move them together (`cargo tree -i dvb-bbframe` should show one copy).

## Breaking changes

`payload::l1::pre::L1Pre::to_bytes`, `crc32` and `serialize_with_crc` return `Result` (#1095, W-T2-3). Every `L1Pre` field is public and mutable, so a value outside its wire width (for example `num_rf > 7`) used to panic in the bit writer's `expect`; it is now an error.

```rust
// before
let bytes: [u8; L1PRE_BYTES] = pre.to_bytes();
let crc: u32 = pre.crc32();
let framed: Vec<u8> = pre.serialize_with_crc();
// after
let bytes = pre.to_bytes()?;          // Result<[u8; L1PRE_BYTES]>
let crc = pre.crc32()?;               // Result<u32>
let framed = pre.serialize_with_crc()?; // Result<Vec<u8>>
```

Serializers throughout the crate now return an error instead of silently truncating a length or count that does not fit its wire field (#1129). The error is `ReservedBitsViolation` for the cases listed below.

## New

- `PacketType::is_allocated(byte)`: whether a `packet_type` byte is allocated in TS 102 773 Table 1. It is the single definition the raw-mode resync uses.
- `Stats` gains `raw_resyncs`, `raw_hunt_bytes_discarded`, `tei_dropped` and `cc_duplicates_skipped` (see below).

## Behaviour changes

- `T2miPump::raw` resynchronisation. After a CRC failure the pump used to trust the failed packet's own `payload_len_bits` forever, so one corrupted length (a bit error, or a dropped UDP datagram carrying raw T2-MI over IP) desynchronised the stream permanently, with `Stats::crc_failures` climbing. It now scans forward for the next CRC-valid packet start (a cheap header-plausibility gate first, full CRC-32 second) and resumes framing there. The hunt buffer is bounded (#1095, W-T2-1). The raw-mode resync also no longer `debug_assert`s that the seeded reassembler frames nothing (a fuzz finding); anything it frames is CRC-gated like the normal path.
- `T2miPump::feed_ts` no longer forwards every filtered-PID packet blindly. Packets with `transport_error_indicator` set are dropped (`Stats::tei_dropped`), and the one repeated packet ISO/IEC 13818-1 §2.4.3.3 permits (byte-identical payload, unchanged `continuity_counter`) is skipped (`Stats::cc_duplicates_skipped`) instead of being appended twice, which used to turn a good T2-MI packet into a CRC failure (#1095, W-T2-2).

## Fixes

- `inner_ts::InnerTsRecovery` (used by `dvb-tools t2mi --inner` without `--plp`) ran every PLP's BBFrames through one shared `CarryOverExtractor`, so a user packet split across a BBFrame boundary was corrupted whenever another PLP's frame arrived before the split completed. Carry-over state is now per PLP, mirroring `dvb_bbframe::pump::BbframePump` (#1034).
- Serializers that silently wrapped or masked an out-of-range field now return `ReservedBitsViolation` (#1095, #1129):
  - `packet::Header::serialize_into`: `superframe_idx` (4 bits) used to wrap to 0.
  - `payload::individual_addressing` function bodies: `ace_gain`, `ace_maximal_extension`, `ace_clipping_threshold`, `miso_group` rfu, `tr_papr` rfu1/`tr_clipping_threshold`/rfu2/`number_of_iterations`, `tx_sig_fef_seq_num` rfu1/`seq_num_1`/rfu2/`seq_num_2`/rfu3, `tx_sig_aux_stream_tx_id`/rfu, `rf_idx`/`frequency` rfu.
  - `payload::l1::post` framed-block writer: a bit length of 16 bits or more used to wrap into the 16-bit framed-block length field.
  - `payload::l1::post::L1ExtBlock::write`: a `data` vector longer than `data_bit_len` implies underflowed `data_bit_len - done` (debug panic; in release it wrote bits beyond the declared length and misframed the L1EXT region). The mismatch is now rejected (#1095, W-T2-5).

---

Published from tag `dvb-t2mi-v11.0.0`.
