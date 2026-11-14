---
name: dvb-si-basics
description: Use when writing Rust code against the dvb-si crate to parse or build DVB SI / MPEG-2 PSI (PAT, PMT, SDT, NIT, EIT, TDT, descriptors, DSM-CC carousels) from transport streams or section bytes.
---

# dvb-si basics

`dvb-si` is a typed parser **and** builder for DVB Service Information
(ETSI EN 300 468) and MPEG-2 PSI. Every table and descriptor it implements is
a Rust struct with a symmetric wire codec (see *Building*), so a parse →
serialize round trip is byte-identical.

## Scope

**It does:** SI/PSI table sections (31 typed section parsers), descriptor loops
(116 auto-dispatched tags + AIT/INT/UNT/RNT namespaces + private-tag opt-ins),
EN 300 468 Annex A text decoding, DSM-CC data/object carousel reassembly
(DSI/DII/DDB, BIOP, `CarouselFs`), multi-section collection, an EPG store.

**It does not:** frame TS packets or reassemble sections itself (that lives in
`mpeg-ts`, used by `dvb-si`'s `demux` module), and it never decodes or
inspects media payloads. Samples/ES bytes are untouched.

## The core loop: TS packets in, typed sections out

`SiDemux` (feature `ts`, on by default) takes 188-byte TS packets and yields a
`SectionEvent` per **changed** complete section. CRC-bearing sections are
CRC-checked before they are gated; failures are dropped and counted in
`demux.stats()`.

```rust
use dvb_si::demux::SiDemux;
use dvb_si::descriptors::AnyDescriptor;
use dvb_si::tables::AnyTableSection;

let mut demux = SiDemux::builder().build();

// In real code `packet` is each aligned 188-byte packet from your TS source
// (file, UDP, tuner).
for event in demux.feed(&packet) {
    match event.table_section() {
        Ok(AnyTableSection::SdtSection(sdt)) => {
            for service in &sdt.services {
                for item in service.descriptors.iter().flatten() {
                    if let AnyDescriptor::Service(svc) = item {
                        // DvbText decodes EN 300 468 Annex A → UTF-8.
                        println!("service: {}", svc.service_name.decode());
                    }
                }
            }
        }
        Ok(AnyTableSection::PatSection(pat)) => {
            println!("PAT v{} on {}", event.version().unwrap_or(0), event.pid());
            assert_eq!(pat.entries.len(), 1);
        }
        Ok(other) => { let _ = other; }
        Err(_) => {} // malformed section
    }
}
```
(doctest in `dvb-si/src/lib.rs`, section "Pump TS packets")

Builder knobs: `follow_pat(bool)` (default true — a changed PAT auto-watches
each program's PMT PID), `dvb_si_pids(bool)` (default true — pre-watch the
well-known SI PIDs), `pid(p)` (extra PIDs), `emit_repeats(bool)` (default
false — bypass the version gate), `gate_capacity(n)`.

Full runnable version reading a file: `dvb-si/examples/list_services.rs`.

You already have complete section bytes? Skip the demux:
`AnyTableSection::parse(&bytes)` (or `EitSection::parse(..)` etc.).

A table that spans sections (NIT/BAT/SDT/EIT): feed `event.bytes()` to
`collect::SectionSetCollector`, which returns a set only when it is complete:

```rust
let mut c = SectionSetCollector::new();
let result = c.push_section(&sec).unwrap();
assert!(result.is_some());
```
(`dvb-si/src/collect/mod.rs` unit test `collect_single_section_is_complete`)

## Descriptor loops

Tables expose their raw loop as a borrowed `DescriptorLoop`; walk it typed:

```rust
let loop_bytes = [
    0x4D, 0x07, b'e', b'n', b'g', 0x02, b'H', b'i', 0x00, // short_event
    0xA7, 0x02, 0xCA, 0xFE,                               // unknown 0xA7
];
let items: Vec<_> = parse_loop(&loop_bytes).collect();
assert_eq!(items.len(), 2);
match items[0].as_ref().unwrap() {
    AnyDescriptor::ShortEvent(se) => {
        assert_eq!(se.language_code.as_str(), "eng");
        assert_eq!(se.event_name.decode(), "Hi");
    }
    other => panic!("expected ShortEvent, got {other:?}"),
}
assert!(matches!(items[1].as_ref().unwrap(), AnyDescriptor::Unknown { tag: 0xA7, .. }));
```
(`dvb-si/src/descriptors/any.rs` module docs)

`DescriptorLoop::iter()` is the same walk over a table's loop field. For a
body-agnostic scan use the structural walk `DescriptorLoop::raw_tags()` /
`contains_tag(tag)` (it never type-parses a body, so a malformed body cannot
hide a tag).

## Building: parse ↔ serialize

`Parse` borrows from the input (`Struct<'a>`, zero-copy); `Serialize` writes
into a caller buffer of exactly `serialized_len()` bytes. Build a section from
typed fields with `new(..)`, then serialize:

```rust
let pat = PatSection::new(
    1,
    0,
    true,
    0,
    0,
    vec![
        PatEntry::new(1, Pid::new(0x0100)),
        PatEntry::new(2, Pid::new(0x0200)),
    ],
);

let mut bytes = vec![0u8; pat.serialized_len()];
pat.serialize_into(&mut bytes).expect("buffer is sized");

let parsed = PatSection::parse(&bytes).expect("valid PAT");
assert_eq!(parsed, pat);
```
(`dvb-si/examples/build_and_parse_pat.rs`)

- CRC-32 is appended automatically by the serializer.
- Over-wide field values are `Error::FieldOverflow`, never silently masked.
- Packetise the section bytes with `mpeg-ts`, not here.
- `serde` (opt-in) is **serialize-only**; re-parse from wire bytes, never from
  JSON.

## Carousels

DSM-CC: frame with `tables::dsmcc::DsmccSection::parse`, type the payload with
`carousel::UnMessage::parse` (`Dsi`/`Dii`) and `DownloadDataBlock::parse`, then
`ModuleReassembler::note_dii(&dii)` / `feed_ddb(&ddb)` — `Some(Module)` when
the last block arrives. It is version-aware, out-of-order tolerant and
memory-capped. Layering: `dvb-si/src/carousel/mod.rs`.

## Features

| Feature | Default | Effect |
|---|---|---|
| `std` | on | off → `#![no_std]` + `alloc` |
| `legacy-charsets` | on | KS X 1001 / GB-2312 / Big5 text via `encoding_rs`; off → `DvbText::try_decode` is `Error::CharsetNotEnabled` for those |
| `ts` | on | `demux::SiDemux`; off → bring your own section bytes |
| `jiff` | on | MJD+BCD → `jiff::Timestamp` helpers, `epg` module |
| `serde` | off | serialize-only JSON support |
| `yoke` | off | `owned::Owned<T>` to keep a parsed view past its buffer |
| `flate2` | off | zlib decompression of compressed carousel modules |

`no_std`: `dvb-si = { version = "…", default-features = false }`.

## Pitfalls

- **Borrowed views cannot outlive the input.** `PmtSection<'a>`,
  `SdtSection<'a>`, `DescriptorLoop<'a>` and every parsed struct borrow the
  section bytes. Do not return them past the buffer's lifetime; with `yoke`,
  `owned::Owned::try_new(bytes, |b| PmtSection::parse(b))?` gives a `'static`
  value (`dvb-si/src/owned.rs`). A `SectionEvent` itself owns its bytes.
- **Not every descriptor tag is known.** Unknown/private tags become
  `AnyDescriptor::Unknown { tag, body }` — never a panic. Private/regional tags
  are never auto-dispatched: opt in with a `DescriptorRegistry` (scoped by the
  `private_data_specifier_descriptor`) and walk with `iter_with(&registry)`;
  the typed value then appears as `AnyDescriptor::Other` and needs a
  `downcast_ref`.
- **The demux only reports changed sections.** The version gate suppresses a
  repeat with the same version *and* CRC (short-form sections use a CRC hash).
  A steady carousel produces one event, not one per cycle; that is not a
  parser bug. Use `emit_repeats(true)` if you really want every repeat.
- **Times are MJD + BCD, not UNIX time.** `start_time_raw` is a 16-bit MJD
  followed by BCD `HHMMSS`; decode with `start_time_utc()` (feature
  `jiff`) or `broadcast_common::time::decode_mjd_bcd`, and encode with
  `set_start_time(..)`/`set_duration(..)` (which return `ValueOutOfRange` when
  the value does not fit).
- **Text is Annex A, not UTF-8.** Always go through `DvbText::decode()` (or
  `Display`) rather than `from_utf8` on the raw bytes.
- **RFU bits are preserved, not reset.** Parsing accepts any reserved-bit
  value and serialization re-emits it; constructed values carry the edition
  default.
