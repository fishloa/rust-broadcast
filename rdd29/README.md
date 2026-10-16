# rdd29

[![Crates.io](https://img.shields.io/crates/v/rdd29.svg)](https://crates.io/crates/rdd29)
[![docs.rs](https://img.shields.io/docsrs/rdd29)](https://docs.rs/rdd29)

SMPTE RDD 29:2019 Dolby Atmos bitstream: frame and element framing plus
bed/object rendering metadata, `no_std` + `alloc`.

## What it does / what it does not

RDD 29 is the disclosure document for the frame-based Dolby Atmos bitstream
used to move a cinema Atmos program: per frame, one `ATMOSFrame` element
holding bed definitions (channel layouts), object definitions (3D pan
metadata) and audio-data elements. Use this crate to read or build that
framing and metadata, for example in tooling that inspects or rewrites object
positions.

It does **not**:

- decode audio: `AudioDataDlc`'s payload is the Dolby Lossless Coding (DLC)
  codec's own bitstream, kept as opaque bytes after the two byte-aligned
  fields `AudioDataID` and `DLCSize` (`DLCSampleRate`, `ShiftBits`, predictor
  and residual fields are deliberately not modelled);
- render or mix (no panner, no speaker layouts beyond the channel-ID names);
- cover any other Dolby Atmos carriage (this is RDD 29's cinema bitstream
  only);
- implement RDD 29's section 3.3 Amplitude Gain (`A10`) coding: it is
  transcribed but no implemented field uses it.

## Standards supported

| Standard | Edition | Scope / clauses | Status | Notes |
|----------|---------|-----------------|--------|-------|
| SMPTE RDD 29 - Dolby Atmos Bitstream Specification | 2019 (vendored, 20 pp.; revision of RDD 29:2014) | Sections 2-5: `ATMOSFrame` (4.2), `BedDefinition1` (4.3), `ObjectDefinition1` (4.4), `AudioDataDLC` (4.5), Plex coding (3.4), relative distance coding (3.2), element IDs (Table 1) | Partial | DLC payload opaque; section 3.3 gain coding not implemented; reserved/unknown `ElementID`s round-trip as `AnyElement::Unknown` |

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | no      | `serde::Serialize`/`Deserialize` derives on public types. |

## Quick start

```rust
use broadcast_common::{Parse, Serialize};
use rdd29::{
    AtmosFrame, AudioDataDlc, BedChannel, BedDefinition1, BitDepth, ChannelId, FrameRate,
    SampleRate,
};

let bed = BedDefinition1::new(
    1,
    vec![BedChannel {
        channel_id: ChannelId::LeftScreen,
        audio_data_id: 10,
    }],
);
let dlc = AudioDataDlc::new(10, &[0xDE, 0xAD, 0xBE, 0xEF]).unwrap();

let frame = AtmosFrame::new(
    SampleRate::Hz48000,
    BitDepth::Bits24,
    FrameRate::Fps24,
    1,
    vec![
        rdd29::AnyElement::BedDefinition1(bed),
        rdd29::AnyElement::AudioDataDlc(dlc),
    ],
);

let bytes = frame.try_to_bytes().unwrap();
assert_eq!(AtmosFrame::parse(&bytes).unwrap(), frame);
```

More: `cargo run -p rdd29 --example build_atmos_frame`, `--example parse_atmos_frame`.

## What is implemented

- `AtmosFrame` (`ATMOS_VERSION`, `SampleRate`, `BitDepth`, `FrameRate`) holding
  a list of `AnyElement`s; `ElementId` with the Table 1 IDs
  (`ELEMENT_ID_ATMOS_FRAME`, `..._BED_DEFINITION1`, `..._OBJECT_DEFINITION1`,
  `..._AUDIO_DATA_DLC`).
- `BedDefinition1`: `BedChannel` list mapping a `ChannelId` (14 named
  channels plus `Reserved(u8)`) to an `AudioDataID`.
- `ObjectDefinition1`: `PanSubBlock`s (`PanInfo` position, snap, `ZoneGain` per
  `ZoneId`, `ObjectSpreadMode`, decorrelation via `DecorCoefPrefix`) and an
  optional `AudioDescription` text. The number of sub-blocks is derived from
  `FrameRate`.
- `AudioDataDlc`: `AudioDataID`, `DLCSize`, opaque payload.
- `distance` module: `distance_xy`, `distance_z`, read-only derived views of
  the section 3.2 relative-distance codes (the raw `u16` stays the
  round-tripped value; shifts and core `f64` only, so identical under
  `no_std`).
- Plex coding (`PLEX_MAX_VALUE`), with non-minimal escapes rejected
  (`Error::NonMinimalPlex`).

## Design notes

- Symmetric `Parse` / `Serialize`; every reserved field is a hard-validated
  constant so a corrupt frame cannot re-serialize differently. The fuzz target
  asserts re-serialization equals the input bytes.
- Real-data caveat: RDD 29 has no capturable file format outside a licensed
  encoder, so the committed fixture (`tests/fixtures/eac3_frame0.bin`) is a
  real E-AC-3 syncframe used as an opaque `AudioDataDLC` payload inside
  framing built through this crate's own typed API. There is no real Atmos
  capture behind the tests.
- Public enums are `#[non_exhaustive]` with `name()` + `Display` labels.
- `no_std` + `alloc`; one `thiserror` error enum.

## Related crates

Depends only on `broadcast-common`. Shares the "container, not codec" boundary
with `st337` (AES3 non-PCM bursts) and `st377-1` (MXF).

## Minimum Supported Rust Version

1.95.0

## License

MIT OR Apache-2.0
