---
status: DRAFT
---

# rist-runtime 0.5.0

_Released 2026-11-07._

Breaking-class release (0.4 -> 0.5): the error enum wraps the shared kinds, `InvalidVersion` becomes `UnsupportedVersion`, `Error::TrailingData` is removed, and the RTCP types now come from `rtp-packet`'s `rtcp` module.

Version: 0.4.0 -> 0.5.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- error variants `BufferTooShort`, `OutputBufferTooSmall` now wrap `broadcast_common::error::{BufferTooShort, OutputBufferTooSmall}`; `BufferTooShort` gains the shared `what` field ("RTCP common header", "Generic NACK", "Range NACK", "RTT Echo", "RTCP sub-packet"); `InvalidVersion(u8)` is replaced by `UnsupportedVersion(broadcast_common::error::UnsupportedVersion)` (`what: "RTCP"`, `found`).
- `Error::TrailingData`, never constructed: TR-06-1 §5.2 does not fix a closed sub-packet list, and unmatched sub-packets are kept in `unknown`.

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
    -> Error::UnsupportedVersion(e)  // e.what == "RTCP", e.found
Error::Rtcp(rtcp_packet::Error)
    -> Error::Rtcp(rtp_packet::rtcp::Error)
```

### Behaviour change: padded sub-packets

A padded sub-packet with an empty body is now `InvalidPaddingCount` instead of reading the length byte as the count.

## Other changes

- Depends on `rtp-packet` (its `rtcp` module) instead of the absorbed `rtcp-packet` crate; the types are unchanged, but `Error::Rtcp` now wraps `rtp_packet::rtcp::Error`.
- New `librist_capture` test: every datagram of the librist 0.2.18 Simple Profile capture parses as RTP (PT 33, contiguous sequence, 1316-byte TS payload) and re-serializes byte-exactly; adds a dev-dependency on `rtp-packet`.

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11
- `rtp-packet` 0.5 -> 0.6

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `rist-runtime-v0.5.0`.
