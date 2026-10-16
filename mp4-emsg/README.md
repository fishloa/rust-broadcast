# mp4-emsg

[![Crates.io](https://img.shields.io/crates/v/mp4-emsg.svg)](https://crates.io/crates/mp4-emsg)
[![docs.rs](https://img.shields.io/docsrs/mp4-emsg)](https://docs.rs/mp4-emsg)

ISO BMFF / DASH Event Message Box (`emsg`, ISO/IEC 23009-1): parse and serialize inband DASH/CMAF timed events. Formerly `dvb-emsg`, which now re-exports this crate.

```toml
[dependencies]
mp4-emsg = "0.4"
```

## What it does / what it does not

**Does:** parse and build one `emsg` box, version 0 (segment-relative `presentation_time_delta`) and version 1 (representation-relative `presentation_time`), into typed fields: `scheme_id_uri`, `value`, `timescale`, the presentation time, `event_duration`, `id` and the opaque `message_data`. `size` and `version` are derived on serialize from the typed fields, never passed through. `EmsgBox::is_scte35` recognises the SCTE 35 scheme URI, so you know when `message_data` is a `splice_info_section`.

**Does not:** walk an MP4 file to find the boxes (use `transmux`), decode `message_data` (use `scte35-splice` for SCTE 35, or an ID3 parser), convert events to HLS or MPD signalling (use `timed-metadata`), or write `InbandEventStream` into a manifest.

Typical users: a DASH/CMAF packager injecting ad-cue events, a player or monitor reading them, and `timed-metadata`/`transmux` internally.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ISO/IEC 23009-1 "DASH" §5.10.3.3 Event Message Box | Edition not established (paid, not vendored) | `EventMessageBox` v0 and v1 byte layout | Full (from public structure) | The byte layout and v0/v1 field order come from the widely reproduced public structure plus DASH-IF Part 10, not from the paid text. This is softer footing than the other crates and is flagged on purpose |
| DASH-IF IOP Part 10 "Events and Timed Metadata" | v5.0.0 (vendored `dashif_iop_part10_v5.0.0_emsg`) | §6.1 and Table 6-2 field semantics | Full | Transcribed in `docs/emsg.md` |
| ANSI/SCTE 35 and SCTE 214 scheme URIs | Not vendored | Only the `urn:scte:scte35` prefix test | Typed-opaque | `message_data` is returned as bytes |

Verified against a real version 1 `emsg` extracted from the DASH-IF livesim2 simulator (`fixtures/shared/emsg_v1_scte35_livesim.bin`) and a constructed version 0 box.

## Features

| Feature | Default | Description |
|---|---|---|
| `std` | yes | Links the standard library (`std::error::Error`). Without it the crate is `#![no_std]` and needs only `alloc`. |
| `serde` | no | `serde::Serialize` on the public types. |

## Quick start

Read the real livesim2 version 1 box, then build a version 0 box from typed fields:

```rust
use mp4_emsg::{EMSG_FLAGS, EmsgBox, EmsgVersion, PresentationTime};

// A genuine v1 emsg from a live DASH segment (SCTE 35 splice).
let wire = std::fs::read("../fixtures/shared/emsg_v1_scte35_livesim.bin").unwrap();
let real = EmsgBox::parse(&wire).unwrap();
assert_eq!(real.version(), EmsgVersion::RepresentationRelative);
assert!(real.is_scte35());
assert_eq!(real.message_data[0], 0xFC); // splice_info_section table_id
assert_eq!(real.to_vec().unwrap(), wire); // byte-exact

// Build version 0 from fields; the variant of PresentationTime picks the version.
let splice = [0xFCu8, 0x30, 0x11];
let b = EmsgBox {
    flags: EMSG_FLAGS,
    scheme_id_uri: "urn:scte:scte35:2013:bin",
    value: "",
    timescale: 90_000,
    presentation_time: PresentationTime::Delta(0),
    event_duration: 0xFFFF_FFFF,
    id: 1,
    message_data: &splice,
};
let bytes = b.to_vec().unwrap();
assert_eq!(EmsgBox::parse(&bytes).unwrap(), b);
```

Runnable programs: `cargo run -p mp4-emsg --example build_emsg` and `--example parse_emsg`.

## What is implemented

- `EmsgBox`: the `emsg` FullBox with both bodies. Version 0 puts the two strings first; version 1 puts the integers first and the strings last. Both orders parse and serialize.
- `PresentationTime::{Delta, Absolute}`: selecting the variant selects the box version.
- `EmsgVersion` with `name()` and `Display`.
- Constants `EMSG_BOX_TYPE`, `EMSG_FLAGS`, `FULLBOX_HEADER_LEN`, `SCTE35_SCHEME_PREFIX`, `STRING_TERMINATOR`, `VERSION_0`, `VERSION_1`.
- `Error` (`#[non_exhaustive]`) covers `BufferTooShort`, `OutputBufferTooSmall`, `NotEmsg`, `InvalidSize`, `UnsupportedVersion`, `InvalidString` and `FieldTooWide`.

## Design notes

Zero-copy (`EmsgBox<'a>` borrows strings and `message_data`), derive-on-serialize with no raw passthrough, round-trip tested against the real fixture, serializer length checks (no wrapping casts), `no_std` + `alloc`.

## Related crates

Depends on `broadcast-common` only. `timed-metadata` converts these events to and from SCTE 35 and HLS `EXT-X-DATERANGE`; `scte35-splice` decodes the payload; `transmux` finds `emsg` boxes inside fMP4/CMAF segments.

## MSRV and licence

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
