# dvb-ci-runtime 0.21.0

_Released 2026-11-15._

Minor-class release (0.20 -> 0.21). The `date_time` resource no longer truncates a date after 2038 or answers 1970-01-01 for a pre-epoch clock (it reports the new `Notification::ClockUnrepresentable` instead), and the `ca_pmt` serialization it reaches through `dvb-ci` is fallible at its call sites.

Version: 0.20.0 -> 0.21.0.

## Breaking changes and migration

### `SlotInfo` is `#[non_exhaustive]`

`device::SlotInfo` is built by the device layer and field-assigned, never by an external struct literal, so a field a device learns to report later is additive.

```rust
// before
let info = SlotInfo { num: 1, module_ready: true, module_present: true };
// after
let info: SlotInfo = device.slot_info()?;   // filled by the device layer (CaDevice::slot_info)
let (num, present) = (info.num, info.module_present);
```

### `CaError::NotSent` names its diagnostic

`managed::CaError::NotSent` is a struct variant `NotSent { detail: String }` (the stack's diagnostic named, not a bare positional `String`).

```rust
// before
if let CaError::NotSent(ref reason) = err { eprintln!("{reason}"); }
// after
if let CaError::NotSent { detail } = err { eprintln!("{detail}"); }
```

### The `date_time` clock may report "no instant"

`resource::DateTime::with_clock` takes a `fn() -> Option<[u8; UTC_TIME_LEN]>` clock; `None` means an instant the 5-byte `UTC_time` field cannot express, and the resource then sends no reply and reports `Notification::ClockUnrepresentable`.

```rust
// before
let resource = DateTime::with_clock(|| Some(utc_time_bytes_now()));
// after
let resource = DateTime::with_clock(|| Some(utc_time_bytes_now()));   // None for an unrepresentable instant
```

### `CaPmtBuilt` serialization is fallible

`CaPmtBuilt::to_bytes` (through `dvb-ci`) returns `Result<Vec<u8>>`; call sites handle the `Result`.

```rust
// before
let bytes = built.to_bytes();
// after
let bytes = built.to_bytes()?;   // a filtered descriptor loop that cannot be framed is an error
```

## Fixes

- The `date_time` resource no longer truncates the MJD to 16 bits (after 2038-04-22 the date was sent wrapped) and no longer sends 1970-01-01 for a pre-epoch clock: an unrepresentable instant sends no reply and reports the new `Notification::ClockUnrepresentable`.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `dvb-ci` 0.12 -> 0.13, `dvb-si` 14 -> 15, `mpeg-ts` 0.8 -> 0.9.

---

Published from tag `dvb-ci-runtime-v0.21.0`.
