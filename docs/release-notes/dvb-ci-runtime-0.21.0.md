# dvb-ci-runtime 0.21.0

_Released 2026-11-15._

### Changed
- BREAKING: `managed::CaError::NotSent` is a struct variant `NotSent { detail: String }` (the stack's diagnostic named, not a bare positional `String`) (#2734).
- BREAKING: `resource::DateTime::with_clock` takes a `fn() -> Option<[u8; UTC_TIME_LEN]>` clock; `None` means an instant the 5-byte `UTC_time` field cannot express (issue #2723).
- BREAKING: `SlotInfo` is now `#[non_exhaustive]`; it is built by the device layer and field-assigned, never by an external struct literal (#2664).
- BREAKING: `CaPmtBuilt` serialization (via `dvb-ci`) is now fallible; call sites handle `Result` (#2659).

### Fixed
- The `date_time` resource no longer truncates the MJD to 16 bits (after 2038-04-22 the date was sent wrapped) and no longer sends 1970-01-01 for a pre-epoch clock: an unrepresentable instant sends no reply and reports the new `Notification::ClockUnrepresentable` (issue #2723).

---

Published from tag `dvb-ci-runtime-v0.21.0`.
