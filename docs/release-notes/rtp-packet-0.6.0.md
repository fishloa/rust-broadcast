
# rtp-packet 0.6.0

_Released 2026-11-07._

Breaking-class release (0.5 -> 0.6): the crate now contains the former `rtcp-packet` crate as module `rtp_packet::rtcp`, the error enum wraps the shared kinds, and `InvalidVersion` becomes `UnsupportedVersion`.

Version: 0.5.0 -> 0.6.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- error variants `BufferTooShort`, `OutputBufferTooSmall`, `InvalidValue` now wrap `broadcast_common::error` types (tuple variants); `InvalidVersion(u8)` is replaced by `UnsupportedVersion(broadcast_common::error::UnsupportedVersion)` (`what` = "RTP"). Hand-written `From` impls removed.
- Unused public items, with no use in the workspace beyond their own unit tests: `is_dynamic_payload_type`.

### Shared error kinds from `broadcast_common::error`

`broadcast-common` 11 defines the common failure kinds once, as `Copy` structs. This crate's error enum wraps them with `#[error(transparent)] Kind(#[from] broadcast_common::error::Kind)`, so the variants `BufferTooShort` and `OutputBufferTooSmall` (and any other kind named in the list above) changed from struct variants to tuple variants holding the shared struct. The fields are the same, so only the pattern changes, and `From` impls exist, so `?` and `.into()` still build them.

```rust
// before
match err {
    Error::BufferTooShort { need, have, what } => eprintln!("{what}: need {need}, have {have}"),
    _ => {}
}
// after
match err {
    Error::BufferTooShort(e) => eprintln!("{}: need {}, have {}", e.what, e.need, e.have),
    _ => {}
}
```

The display text of these variants now follows the common wording shared by every crate in the wave.

### Renamed and removed items

```text
Error::InvalidVersion(v)
    -> Error::UnsupportedVersion(e)  // e.what == "RTP", e.found
rtp_packet::is_dynamic_payload_type(pt)
    -> (removed; unused)
```

### Merged crate: `rtcp-packet` -> `rtp_packet::rtcp`

`rtcp-packet` 0.6.0 is not released again. Its RFC 3550 section 6 RTCP types (SR, RR, SDES, BYE, APP and the compound packet) live in `rtp_packet::rtcp` with unchanged names below that module; the `std` and `serde` features of `rtp-packet` now cover them.

```toml
# before
rtcp-packet = "0.6"
# after
rtp-packet = "0.6"
```
```rust
// before
use rtcp_packet::{SenderReport, CompoundPacket};
// after
use rtp_packet::rtcp::{SenderReport, CompoundPacket};
```

## Added

- absorbs the former `rtcp-packet` crate as module `rtp_packet::rtcp` (RFC 3550 section 6 RTCP SR/RR/SDES/BYE/APP and the compound packet; the types and their paths below `rtcp` are unchanged, so `rtcp_packet::X` becomes `rtp_packet::rtcp::X`). The `serde` and `std` features of `rtp-packet` now also cover the RTCP types.
- `From<broadcast_common::len::BufferTooShort>` and `From<broadcast_common::len::OutputBufferTooSmall>` for the crate error type.

## Fixed

- Serializers report a too-small output buffer as the new `Error::OutputBufferTooSmall { need, have }` (with `From<broadcast_common::len::OutputBufferTooSmall>`) instead of the input-side `Error::BufferTooShort`.

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `rtp-packet-v0.6.0`.
