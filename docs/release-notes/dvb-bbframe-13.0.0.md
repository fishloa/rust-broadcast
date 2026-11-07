
# dvb-bbframe 13.0.0

_Released 2026-11-07._

Lockstep major (12.1.0 -> 13.0.0). The breaking changes and their migration snippets are in the lockstep note [v13.0.0](v13.0.0.md); this note lists what changed in `dvb-bbframe`.

## Removed
- BREAKING: Unused public items, with no use in the workspace beyond their own unit tests: `bufstat_bytes`.
## Added
- `From<broadcast_common::len::BufferTooShort>` and `From<broadcast_common::len::OutputBufferTooSmall>` for the crate error type, producing the identical existing variants (c3 S1).
## Changed
- Error enums derive `Clone`, `PartialEq` and `Eq` wherever every field allows it (c6 C6-14).
- Length pre-checks go through `broadcast_common::len::{need_in, need_out}`; the error values are unchanged (c3 S1).
- BREAKING: error variants `BufferTooShort`, `OutputBufferTooSmall` now wrap `broadcast_common::error` types (tuple variants); hand-written `From` impls removed; messages follow the common wording.
## Dependencies
- Sibling crates move to a new epoch in this release wave: `broadcast-common` 10 -> 11. The `Cargo.toml` requirements name the new versions; no other manifest change.

---

Published from tag `v13.0.0`.
