# caption-convert

[![Crates.io](https://img.shields.io/crates/v/caption-convert.svg)](https://crates.io/crates/caption-convert)
[![docs.rs](https://img.shields.io/docsrs/caption-convert)](https://docs.rs/caption-convert)

Convert CEA-608/708 and EBU Teletext captions to WebVTT, and convert WebVTT
<-> SRT. Built on this workspace's already-shipped decoders (`cc-data`,
`dvb-vbi`, `timed-metadata`) rather than re-implementing them.

## Scope: the conversion matrix

| from \ to            | WebVTT          | SRT             | IMSC Text          | IMSC Image          |
|-----------------------|-----------------|-----------------|---------------------|----------------------|
| CEA-608                | Lossy           | Lossy           | Not implemented     | Unsupported          |
| CEA-708                | Lossy           | Lossy           | Not implemented     | Unsupported          |
| Teletext               | Lossy           | Lossy           | Not implemented     | Unsupported          |
| DVB bitmap subtitle    | **Unsupported** | **Unsupported** | **Unsupported**     | Not implemented      |
| TTML/IMSC              | Not implemented | Not implemented | Lossless (identity) | Not implemented      |

- **Lossy** = implemented; see each module's docs for exactly what is
  dropped (placement, styling, window geometry, ...).
- **Unsupported** = structurally impossible here (DVB bitmap subtitles are
  *pixels*; text extraction needs OCR, out of scope).
- **Not implemented** = plausible future work not built in this cut, most
  notably DVB bitmap -> IMSC 1.1 Image Profile, which needs a full
  RLE-decode + CLUT-resolve + compositing + PNG-encode rendering pipeline
  this crate does not (yet) provide.

[`matrix::MATRIX`] is the machine-readable source of truth (with a test
pinning it to the full cross-product); the crate root docs carry the same
table with the reasoning behind every cell.

## What this crate adds over its dependencies

- [`Cea608ToWebVtt`] / [`Cea708ToWebVtt`] (feature `cc-data`, default on): a
  raw-`cc_data()`-bytes-in / WebVTT-or-SRT-string-out wrapper over
  `timed_metadata::webvtt::Cea608CueExtractor`/`Cea708CueExtractor` (issue
  #568), which owns the roll-up/pop-on/paint-on cue-boundary detection.
- [`TeletextToWebVtt`] (feature `teletext`, default on): the same shape over
  `timed_metadata::webvtt::TeletextCueExtractor` (issue #666).
- [`parse_webvtt`]: **new** -- `timed-metadata` only writes WebVTT; this
  crate adds the reader, needed to make WebVTT -> SRT a real conversion.
- [`srt`] module: **new** -- SRT has no formal specification (documented,
  not invented); this crate implements the de facto ffmpeg/VLC-compatible
  format (sequential index, `hh:mm:ss,ttt --> hh:mm:ss,ttt`, blank-line
  separated blocks).
- [`webvtt_to_srt`] / [`srt_to_webvtt`]: the WebVTT <-> SRT conversion pair,
  the former returning whether anything was dropped.

## Quick start

```rust
use caption_convert::Cea608ToWebVtt;
use cc_data::decode::Cea608Channel;

let mut conv = Cea608ToWebVtt::new(Cea608Channel::Cc1);
// Feed raw cc_data() byte strings tagged with their 90 kHz PTS...
// conv.push_cc_data(pts, &cc_data_bytes)?;
conv.finalize(0);
let vtt = conv.into_webvtt();
assert!(vtt.starts_with("WEBVTT"));
```

```rust
let (srt, lossy) = caption_convert::webvtt_to_srt(
    "WEBVTT\n\n00:00:00.000 --> 00:00:02.000\nHello\n",
)?;
assert!(!lossy);
# Ok::<(), caption_convert::Error>(())
```

## `no_std`

`#![no_std]` + `alloc` at the core. The `std` feature (default on) only
passes `std` through to dependencies; nothing in this crate's own parsing/
writing logic needs it.

## Examples

```sh
cargo run -p caption-convert --example cc_to_webvtt
cargo run -p caption-convert --example webvtt_to_srt
```
