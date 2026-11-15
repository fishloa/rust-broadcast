# rmt-flute 0.10.0

_Released 2026-11-15._

### Changed
- BREAKING: `LctHeader::tsi` is a `Tsi` and `toi` a `Toi` (new types) carrying the decoded identifier and the field width the `S`/`O`/`H` flags give it, instead of raw borrowed bytes; `Tsi::new`/`Toi::new` reject a width the flags cannot describe and a value the width cannot hold (#2770).
- BREAKING: `fec_payload_id` is a `FecPayloadId` (new type) in `AlcPacket`, `NormData`, and `NormCmdBody::{Flush, Squelch}`, replacing the borrowed `&[u8]`; the value owns its bytes and reports its own length, so serialization no longer pairs a slice with a separately supplied length. `NormCmdBody` loses its lifetime parameter along with the borrowed field (#2770).
- BREAKING: `SourceBlockPartition`'s fields are private; read them with the `transfer_length()`/`encoding_symbol_length()`/`source_symbols()`/`num_blocks()`/`larger_block_len()`/`smaller_block_len()`/`larger_blocks()` accessors (#2766, #2771).
- The `panic_idioms` workspace guard now covers this crate; the NORM_CMD CC `send_time_sec`/`send_time_usec` reader builds the 4-byte arrays directly instead of `.try_into().unwrap()` (#2773).
- `SourceBlockPartition::new` returns `Error::FieldOverflow` when the partition exceeds what a `FecPayloadId128` can address (more than `u32::MAX` source blocks, or more than `u16::MAX` symbols in a block) (#2766).

### Fixed
- A TSI/TOI wider than eight bytes (e.g. a fourteen-byte TOI, `O=3`, `H=1`) serializes big-endian with its leading zero bytes instead of panicking on an out-of-range offset (#2770).
- `LctHeader` rejects a version other than `LCT_VERSION` (1) on parse and serialize with `Error::UnsupportedVersion` (RFC 5651 §5.1) (#2766).

---

Published from tag `rmt-flute-v0.10.0`.
