
# dvb-ci 0.11.0

_Released 2026-11-07._

Breaking-class release (0.10 -> 0.11): the error enum wraps the shared kinds (including `LengthMismatch`), the multi-stream CA_PMT types reuse the single-stream stream types, and `dvb-si` moves to 13.

Version: 0.10.0 -> 0.11.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- error variants `BufferTooShort`, `OutputBufferTooSmall`, `LengthMismatch` now wrap `broadcast_common::error` types (tuple variants; `LengthMismatch` message is now "length mismatch for <what>: declared N, actual M"). Hand-written `From` impls removed.
- Unused public items, with no use in the workspace beyond their own unit tests: `is_minimal`, `retain_caids`.
- `ci_plus::ca_support::MsCaPmtStream` and `MsCaPmtReplyStream`: the multi-stream `MsCaPmt` and `MsCaPmtReply` now hold `objects::ca_pmt::CaPmtStream` and `objects::ca_pmt_reply::CaPmtReplyStream` (identical fields).

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
ci_plus::ca_support::MsCaPmtStream
    -> objects::ca_pmt::CaPmtStream
ci_plus::ca_support::MsCaPmtReplyStream
    -> objects::ca_pmt_reply::CaPmtReplyStream
Error::LengthMismatch { what, declared, computed }
    -> Error::LengthMismatch(e)  // e.what, e.declared, e.actual
```

## Added

- `From<broadcast_common::len::BufferTooShort>` and `From<broadcast_common::len::OutputBufferTooSmall>` for the crate error type.

## Other changes

- The DSM-CC download message header parse and write are one function pair for the transaction-id and `DownloadId` messages.
- The seven fixed-length SPDUs parse through one `parse_spdu_fixed`; a wrong value length now reports `value length is fixed by the object definition`.

## Documentation

- Fixed documentation links/citations to resolve to existing paths (docs/tools hygiene).

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11
- `dvb-si` 12 -> 13
- `mpeg-ts` 0.6 -> 0.7

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `dvb-ci-v0.11.0`.
