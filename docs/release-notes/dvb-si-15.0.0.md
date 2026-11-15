# dvb-si 15.0.0

_Released 2026-11-15._

Major-class release (14 -> 15). Two changes force it: every fixed-width reserved field is now the shared typed `broadcast_common::reserved` value instead of a raw integer, and the `chrono` feature and the `SiDemux` builder are gone in favour of `jiff` and an `SiDemuxConfig`. Every published bucket also moves to a new caret epoch of `broadcast-common` and `mpeg-ts`, which is a major-class change on its own (docs/RELEASE-AUDIT.md section 2).

Version: 14.0.0 -> 15.0.0.

## Breaking changes and migration

### `SiDemux` is built from an `SiDemuxConfig`

`SiDemux::builder()` is gone. A demux is built from the non-exhaustive `demux::SiDemuxConfig` with `SiDemux::new(config)`, so a future setting is additive. The gate capacity is a `NonZeroUsize` (`GATE_CAPACITY_DEFAULT` is still 65 536), which makes a capacity of zero unrepresentable instead of silently behaving as one.

```rust
// before
let mut demux = SiDemux::builder().pid(pid).follow_pat(true).gate_capacity(1 << 16).build();
// after
use core::num::NonZeroUsize;
let mut demux = SiDemux::new(
    SiDemuxConfig::default()
        .with_pid(pid)
        .with_follow_pat(true)
        .with_gate_capacity(NonZeroUsize::new(1 << 16).unwrap()),
);
```

### Reserved fields are `ReservedOnes<BITS>` / `ReservedZero<BITS>`

Every fixed-width reserved run in `descriptors/` (both halves of the alphabet: `audio_stream` through `vvc_subpictures`, plus the `ait::*` and MPEG-2 `extension::*` bodies), every `tables/` section header (`reserved_b1`/`reserved_b5`) and reserved length/count run, `rnt::rar_over_ip`, `unt::enhanced_message` and `TransportStreamEntry::reserved_future_use` is a `ReservedOnes<BITS>` or `ReservedZero<BITS>` instead of a raw `u8`. A value wider than the field is unrepresentable and serialization writes the stored value with no re-mask. Each `*_DEFAULT`/`RESERVED_*` constant is the typed value. Only the variable-width `TextLabelEntry::reserved` and the dual-meaning `DrcTargetLoudness::no_independent_use_or_reserved` stay `u8`.

```rust
// before
let entry = ApplicationSignallingEntry { reserved_future_use_1: 0b1, reserved_future_use_3: 0b111, .. };
let ones = entry.reserved_future_use_3;
// after
use broadcast_common::reserved::ReservedOnes;
let entry = ApplicationSignallingEntry {
    reserved_future_use_1: ReservedOnes::from_bits(0b1),
    reserved_future_use_3: ReservedOnes::from_bits(0b111),
    ..entry
};
let ones = entry.reserved_future_use_3.get();   // u64
```

### `chrono` feature replaced by `jiff`

The default `chrono` feature is replaced by `jiff` (no alias) and `chrono` is no longer a dependency. Every decoded instant that was `chrono::DateTime<Utc>` is now `jiff::Timestamp` (TDT/TOT `utc_time`/`set_utc_time`, `EitEvent::set_start_time`, `collect::eit` `start_time`, `MjdBcdTime::to_datetime`/`from_datetime`, `LocalTimeOffsetEntry::time_of_change`/`set_time_of_change`, network-change-notify `start_time_of_change_utc`, `epg::EpgEvent::start_time`, `EpgStore::now_and_next`/`schedule`), local time offsets are `jiff::SignedDuration` instead of `chrono::Duration`, and `EitEvent::start_time_chrono` is renamed `start_time_utc`. The MJD + BCD decoding is unchanged, and `serde` serializes those instants through jiff (RFC 3339, `Z`).

```toml
# before
dvb-si = { version = "14", features = ["chrono"] }
# after (jiff is on by default)
dvb-si = "15"
```

```rust
// before
let start: chrono::DateTime<chrono::Utc> = event.start_time_chrono()?;
// after
let start: jiff::Timestamp = event.start_time_utc()?;
let epg_start: jiff::Timestamp = epg_event.start_time;   // epg::EpgEvent
```

## Fixes

- ISO/IEC 8859-2 … -15 text (selectors 0x01–0x0B and `0x10 0x00 nn`) is decoded from tables in the crate, pinned byte for byte against `encoding_rs`, so it no longer needs `encoding_rs` and now also decodes under `no_std` instead of yielding U+FFFD.

## Added

- Feature `legacy-charsets` (on by default) carries `encoding_rs`, now needed only for the EN 300 468 Annex A KS X 1001, GB-2312 and Big5 selectors (0x12–0x14); `std` no longer pulls it in. With the feature off, `text::try_decode_dvb_string` and `DvbText::try_decode` return `Error::CharsetNotEnabled { charset: text::LegacyCharset }` for those three selectors, and the infallible `decode` yields U+FFFD per byte.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `mpeg-ts` 0.8 -> 0.9.

---

Published from tag `dvb-si-v15.0.0`.
