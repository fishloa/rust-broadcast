# st2022

SMPTE ST 2022-6:2012 — SDI-over-IP transport (HBRMT).

ST 2022-6 defines the **High Bit Rate Media Transport (HBRMT)** RTP payload
format for carrying uncompressed SDI signals (SD/HD/3G) over IP networks.
The entire serial digital interface payload — video, embedded audio, VANC,
HANC — is encapsulated as a single RTP stream.

This crate implements the ST 2022-6 HBRMT payload header only. It does
**not** implement ST 2022-7 seamless protection switching (hitless
failover) — see "Planned" below. The `VSID` field this crate parses is the
one a ST 2022-7 merge needs to identify redundant-path copies of the same
datagram. The merge logic itself is **not implemented anywhere in this
workspace yet**: `media-plane::byte_merge` is where it is expected to land,
and that module documents `Hitless2022_7` as deliberately absent rather than
stubbed (`MergePolicy` has no such variant). Tracked as issue #752.

## Wire structures

- **`PayloadHeader`** — the 4/8/12+-byte HBRMT header preceding the media
  payload in each RTP datagram (§6.4).
- **`VideoSourceFormat`** — the MAP/FRAME/FRATE/SAMPLE fields describing the
  SDI signal structure.
- **`ClockFrequency`**, **`FecUsage`**, **`TimestampRef`**, **`Scrambling`**,
  **`VideoSourceId`** — typed field enums.

## Usage

```rust
use st2022::PayloadHeader;
use broadcast_common::Parse;

let bytes: &[u8] = &[/* RTP payload after fixed header */];
let header = PayloadHeader::parse(bytes).unwrap();
println!("Video source: {:?}", header.video_source_format);
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Enables `std::error::Error` impls |
| `serde` | no      | Derives `Serialize`/`Deserialize` |

`no_std` + `alloc` when `default-features = false`.

## Planned

Not implemented yet — spec-grounded in `docs/` ahead of any code (see
`docs/README.md`):

- **ST 2022-7 seamless protection switching** — the redundancy model (§6),
  duplicate-identification rule (§4.3 + Annex A), and receiver
  classification (§7 Table 1). The hitless merge itself is expected to land
  in `media-plane::byte_merge`, consuming the `VSID` this crate already
  parses, rather than in this crate.
- **ST 2022-5 FEC** — the separate FEC wire-format standard ST 2022-6 §7.1
  references for interoperability limits only.

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your
option.
