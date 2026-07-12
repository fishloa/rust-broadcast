# rdd29

[![Crates.io](https://img.shields.io/crates/v/rdd29.svg)](https://crates.io/crates/rdd29)
[![docs.rs](https://img.shields.io/docsrs/rdd29)](https://docs.rs/rdd29)

SMPTE RDD 29:2019 ("Dolby Atmos® Bitstream Specification") frame/element
framing plus bed/object rendering metadata — `no_std`.

RDD 29 defines a frame-based bitstream carrying, per frame, a single
`ATMOSFrame` element containing:

- **[`BedDefinition1`]** — a channel-based audio bed's channel-to-audio-asset
  mapping (§2.2/§4.3): which loudspeaker each channel plays to, and which
  `AudioDataDLC` element carries its audio.
- **[`ObjectDefinition1`]** — one panned audio object's per-sub-block
  rendering metadata (§2.3/§4.4): 3D position, snap-to-speaker, per-zone
  gain, spread, decorrelation, and an optional text description.
- **[`AudioDataDlc`]** — one track's audio-essence pointer + opaque payload
  (§2.4/§4.5).

See `docs/rdd29.md` for the curated SMPTE RDD 29:2019 §1-§5 transcription
this crate implements field-for-field (fetched directly from
`pub.smpte.org/pub/rdd29/rdd29-2019.pdf`), including the "Scope decisions"
section documenting two genuine gaps/inconsistencies in the source
disclosure document itself (the `Plex(8)` escape-nesting pseudocode's
internal inconsistency, and the undocumented `AudioDescription` field
semantics) and how this crate resolves each honestly.

**What this crate is not**: an audio codec. `AudioDataDlc`'s payload is the
Dolby Lossless Coding (DLC) codec's own bit-packed bitstream (linear-
predictive + Rice-Golomb entropy-coded residual audio samples) — this crate
treats it as opaque bytes, the same "parse the container, not the codec"
discipline this workspace's `transmux`/`st337` crates use for media
containers and AES3 non-PCM bursts respectively.

`#![no_std]` + `alloc`; depends only on `broadcast-common`.

## Quick start

```rust
use broadcast_common::{Parse, Serialize};
use rdd29::{AtmosFrame, AudioDataDlc, BedChannel, BedDefinition1, BitDepth, ChannelId, FrameRate, SampleRate};

let bed = BedDefinition1::new(1, vec![BedChannel { channel_id: ChannelId::LeftScreen, audio_data_id: 10 }]);
let dlc = AudioDataDlc::new(10, &[0xDE, 0xAD, 0xBE, 0xEF]).unwrap();

let frame = AtmosFrame::new(
    SampleRate::Hz48000,
    BitDepth::Bits24,
    FrameRate::Fps24,
    1,
    vec![rdd29::AnyElement::BedDefinition1(bed), rdd29::AnyElement::AudioDataDlc(dlc)],
);

let bytes = frame.to_bytes();
assert_eq!(AtmosFrame::parse(&bytes).unwrap(), frame);
```

## Examples

```sh
cargo run -p rdd29 --example build_frame
cargo run -p rdd29 --example parse_frame
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | no      | `serde::Serialize`/`Deserialize` derives on public types. |

## Minimum Supported Rust Version

1.86

## License

MIT OR Apache-2.0
