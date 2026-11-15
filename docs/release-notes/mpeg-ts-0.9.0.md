# mpeg-ts 0.9.0

_Released 2026-11-15._

Minor-class release (0.8 -> 0.9) that types the TS framing: the header's scrambling, adaptation-control and continuity-counter fields, the PID constructor and the PCR are validated values instead of masked integers, and a packet's adaptation field is located by one shared code path. A caller that reads or builds headers, PCRs, sections or `SiMux` schedules must migrate.

Version: 0.8.0 -> 0.9.0.

## Breaking changes and migration

### `TsHeader` carries typed fields

`scrambling: u8` is `ScramblingControl`, `has_adaptation`/`has_payload` are one `adaptation_field_control: AdaptationFieldControl` (with `has_adaptation()`/`has_payload()`, so the reserved `00` is named rather than expressed as `(false, false)`), and `continuity_counter: u8` is the 4-bit `ContinuityCounter` (`try_new`, `from_wire_masked`, `next()`, `forward_distance_from()`, `value()`). The accessors `TsHeader::scrambling_control()`/`adaptation_field_control()` are gone because the fields themselves are typed, and `serialize_into` no longer masks, so an out-of-range counter cannot be written.

```rust
// before
let header = TsHeader { pid, has_adaptation: true, has_payload: true, scrambling: 0, continuity_counter: 3 };
let cc = header.continuity_counter;
// after
let header = TsHeader {
    pid,
    adaptation_field_control: AdaptationFieldControl::AdaptationAndPayload,
    scrambling: ScramblingControl::NotScrambled,
    continuity_counter: ContinuityCounter::try_new(3)?,
    ..header
};
let cc = header.continuity_counter.value();
if header.adaptation_field_control.has_payload() { /* .. */ }
```

`OwnedTsPacket::serialize_with_payload(.., cc, ..)`, `null_packet(cc)`, `set_continuity_counter(.., cc)`, `SectionPacketiser::with_continuity(pid, cc)` and `SectionReassembler::feed_with_continuity` take a `ContinuityCounter`; `continuity_counter()` returns one.

```rust
// before
let packet = OwnedTsPacket::null_packet(0);
// after
let packet = OwnedTsPacket::null_packet(ContinuityCounter::ZERO);
```

### `Pid::new` is gone

Constants use `Pid::from_const` (a `const fn` that fails compilation above 0x1FFF), caller values the fallible `Pid::try_new`, and parse sites `Pid::from_wire_masked` (the explicit H.222.0 §2.4.3.2 masking). Nothing masks silently any more.

```rust
// before
let pid = Pid::new(0x0100);
// after
let pid = Pid::from_const(0x0100);   // constant
let pid = Pid::try_new(value)?;      // caller value
```

### `Pcr::from_27mhz` returns `Result`

`Pcr` wraps the validated `broadcast_common::clock33::Clock27`: `Pcr::from_parts(base, extension)` and `Pcr::from_27mhz(ticks)` return `Result`, with out-of-range parts reported as `Error::ClockOutOfRange`; the `base`/`extension` fields are the `base()`/`extension()` accessors, and `clock()` yields the shared value. Parsing stays lossless (a 9-bit wire extension of 300..=511 is kept exactly as written and re-serializes byte-identically), and `Pcr::to_clock27()` is the checked view that reports such a value.

```rust
// before
let pcr = Pcr::from_27mhz(ticks);
let base = pcr.base;
// after
let pcr = Pcr::from_27mhz(ticks)?;
let base = pcr.base();
let clock = pcr.to_clock27()?;   // rejects an off-clock extension
```

### `Section` no longer stores a CRC or a `section_length`

The CRC is recomputed on every serialize, so a section whose payload was edited can no longer carry a stale one; `section_length` is derived from `payload` (`section_length()`), so a long payload is reported as `Error::FieldOverflow` instead of being truncated. `crc32` is the read-only `crc32()` accessor, `section::validate_crc(&raw)` is the free function over wire bytes, `Section::validate_crc()` takes no argument, and `Error::SectionPayloadLengthMismatch` is removed. The byte 1 / byte 5 reserved bits are parsed and round-tripped (`byte1_reserved`/`byte5_reserved`) instead of forced to `11`.

```rust
// before
let crc = section.crc32;
section.validate_crc(section.crc32);
// after
let crc = section.crc32();
section.validate_crc()?;                       // or section::validate_crc(&raw)
```

### Adaptation-field extension and PCR reserved bits are checked, not masked

`Ltw::ltw_offset` is the new 15-bit `LtwOffset`, `AdaptationFieldExtension::piecewise_rate` the new 22-bit `PiecewiseRate`, and `SeamlessSplice::to_marked_field`/`AdaptationField::serialize_into` reject an out-of-range `splice_type`, `DTS_next_AU` (`SeamlessSplice::DTS_NEXT_AU_BITS`), marker bits, `ltw_offset`, `piecewise_rate` or `pcr_reserved`/`opcr_reserved` with `Error::FieldOverflow` instead of writing a different valid value.

```rust
// before
extension.ltw_offset = 0x7FFF;
// after
extension.ltw_offset = LtwOffset::new(0x7FFF)?;
```

### `SiMux` schedules complete sections keyed by `SectionKey`

`SiMux` schedules one complete `Section` at a time, keyed by `SectionKey` `(pid, table_id, section_number)`, and `poll`/`poll_into` take the caller's `broadcast_common::stage::Timestamp` instead of a `Duration`. The packetiser derives `section_length` and the CRC from the section itself, so an empty `Vec` can no longer collide with the PAT entry on PID 0 and mixed-table sections cannot share one entry. A section that cannot be scheduled is returned and kept for `SiMux::rejected` (`RejectedSection`/`SectionRejection`). The convenience registrations (`upsert_pat`, `upsert_pmt`, `upsert_sdt_actual`, `upsert_nit`, `upsert_tdt`, `upsert_tot`) return `Result` and refuse another table's section; the six `TABLE_ID_*` constants name the `table_id` each one expects.

```rust
// before
mux.upsert_pat(pat_section);
let packets = mux.poll(Duration::from_millis(40));
// after
mux.upsert_pat(pat_section)?;
let packets = mux.poll(Timestamp::from_micros(now_us));   // or poll_into(now, &mut out)
```

`SiMux::upsert` takes a `SectionInterval` (`SectionInterval::try_new(Duration)` or `from_const`), enforcing the EN 300 468 §5.1.4.1 25 ms floor at construction rather than by a `debug_assert!` release builds compiled out; the `*_MAX_INTERVAL` constants are `SectionInterval`s.

```rust
// before
mux.upsert(section, Duration::from_millis(5));
// after
mux.upsert(section, SectionInterval::try_new(Duration::from_millis(5))?);   // below the 25 ms floor
```

### `OwnedTsPacket::set_pcr` reports three distinct causes

It used to fabricate one `BufferTooShort { need: 6, have: 0 }`; it now reports `Error::NoAdaptationField` (no adaptation field, or a zero-length one), `Error::AdaptationFieldTooShort { need, have }` and `Error::NoClockReference { which }`. It keeps the packet's existing PCR reserved bits instead of forcing them to `1`, so an in-place rewrite of a captured packet changes the 6 PCR bytes and nothing else.

```rust
// before
packet.set_pcr(pcr);
// after
match packet.set_pcr(pcr) {
    Err(mpeg_ts::Error::NoClockReference { .. }) => { /* the packet's PCR flag is clear */ }
    other => other?,
}
```

## Fixes

- `TsResync` reports the bytes before the first lock as a `ResyncLoss` too: they were counted in `ResyncStats::dropped_bytes` but never surfaced, so a capture that starts mid-packet disagreed with `take_losses()`. Every byte in `dropped_bytes` is now accounted for by a loss.
- `TsPacket::parse`, `extract_ts_payload`, `OwnedTsPacket::adaptation_field`, `OwnedTsPacket::set_pcr`/`set_opcr` and `ts_dup`'s PCR exemption locate the adaptation field and its PCR/OPCR through one shared locator instead of four hand-rolled copies; a truncated slice still reports the field as unreachable rather than reading past it.
- The wire PCR/ESCR extension is decoded as a `u16` directly, so bit 8 of a 9-bit extension is no longer dropped by a `u8` shift.

## Added

- `OwnedTsPacket::set_opcr` rewrites the OPCR in place, and `Pcr::to_field_bytes_with_reserved` encodes the 6-byte field keeping a packet's own reserved bits.
- `TsHeader::to_bytes`: the 4 header bytes, infallibly encoded. `serialize_into` copies from it, and the packet builders use it directly.
- `ContinuityCounter`, `AdaptationFieldControl`, `ScramblingControl`, `LtwOffset`, `PiecewiseRate`, `SectionKey`, `SectionInterval` and `SectionRejection`.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `mp4-emsg` 0.7 -> 0.8 (dev-dependency).
- The `serde` feature enables `broadcast-common/serde`, since `Pcr` serializes the shared clock value.

---

Published from tag `mpeg-ts-v0.9.0`.
