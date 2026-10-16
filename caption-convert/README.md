# caption-convert

[![Crates.io](https://img.shields.io/crates/v/caption-convert.svg)](https://crates.io/crates/caption-convert)
[![docs.rs](https://img.shields.io/docsrs/caption-convert)](https://docs.rs/caption-convert)

Convert CEA-608/708 and EBU Teletext captions to WebVTT and SRT, and convert
WebVTT <-> SRT. Built on this workspace's already-shipped decoders (`cc-data`,
`dvb-vbi`, `timed-metadata`) rather than re-implementing them.

```toml
[dependencies]
caption-convert = "0.2"
```

## What it does / what it does not

**Does:** take raw caption carriage (CEA-608/708 `cc_data()` bytes, or EBU Teletext
data fields, each tagged with a 90 kHz PTS) and give back a WebVTT or SRT document;
parse and write WebVTT and SRT; convert WebVTT <-> SRT. Used by packagers and
caption-extraction tools.

**Does not:**

- **DVB bitmap subtitles (EN 300 743) are not converted to text.** They carry
  pixels, so text needs OCR, which is out of scope. The matrix marks DVB bitmap to
  WebVTT / SRT / IMSC Text `Unsupported`, never silently empty.
- DVB bitmap to IMSC Image Profile is possible in principle (re-encode the pixels)
  but needs an RLE + CLUT + compositing + PNG pipeline that is not built:
  `Not implemented`.
- **TTML / IMSC is not converted to WebVTT, SRT or Image** (`Not implemented`).
  The only TTML row answered is TTML/IMSC to IMSC Text, the identity case, served
  directly by the `ttml-subtitle` crate.
- 608/708/Teletext to IMSC Text is `Not implemented`; to IMSC Image is `Unsupported`.
- 608/708/Teletext output is lossy: placement, styling and window geometry are
  dropped; cue times are the carrying frame / packet PTS.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ANSI/CTA-608-E, ANSI/CTA-708-E (decoded by `cc-data`) | -E editions as cited by `cc-data` (not vendored; unverified against the document) | CC1 pop-on / roll-up / paint-on; one 708 service | Partial (lossy) | feature `cc-data` |
| ETSI EN 300 706 Enhanced Teletext; ETSI EN 301 775 §4.5 carriage | V1.2.1 each (`private/specs-md/INDEX.md`) | Level-1 subtitle page, one `(magazine, page)` | Partial (lossy) | feature `teletext`; English national option only |
| W3C WebVTT | curated rules `specs/rules/webvtt-rules.md` | write (via `timed-metadata`) and parse cues; cue settings, `NOTE`/`STYLE`/`REGION` flagged lossy on conversion to SRT | Partial | no cue-setting emission |
| SRT (SubRip) | no formal specification; de facto ffmpeg/VLC-compatible form | index, `hh:mm:ss,ttt --> hh:mm:ss,ttt`, blank-line separated | Full for that form | `.` millisecond separator accepted on parse |
| ETSI EN 300 743 DVB subtitling | V1.6.1 (`dvb-subtitle`) | bitmap | Not converted | see above |
| W3C TTML2 / IMSC 1.1 | see `ttml-subtitle` | identity row only | Not implemented as a conversion | |

### Conversion matrix

| from \ to | WebVTT | SRT | IMSC Text | IMSC Image |
|---|---|---|---|---|
| CEA-608 | Lossy | Lossy | Not implemented | Unsupported |
| CEA-708 | Lossy | Lossy | Not implemented | Unsupported |
| Teletext | Lossy | Lossy | Not implemented | Unsupported |
| DVB bitmap subtitle | **Unsupported** | **Unsupported** | **Unsupported** | Not implemented |
| TTML/IMSC | Not implemented | Not implemented | Lossless (identity) | Not implemented |
| WebVTT | identity | Lossless unless cue settings / identifiers / `NOTE` / `STYLE` / `REGION` (reported by a `lossy` flag) | n/a | n/a |
| SRT | Lossless | identity | n/a | n/a |

Lossy means implemented (see module docs for what is dropped); Unsupported means
structurally impossible here; Not implemented means plausible but unbuilt.
`matrix::MATRIX` is the machine-readable source of truth for the first five rows
(a test pins it to the full cross-product); call `matrix::check` before a generic
dispatch.

## Feature flags

| Feature | Default | Description |
|---|---|---|
| `std` | on | Passes `std` through to dependencies; this crate's own logic needs only `alloc`. |
| `cc-data` | on | `Cea608ToWebVtt` / `Cea708ToWebVtt`, over `timed-metadata`'s extractors. |
| `teletext` | on | `TeletextToWebVtt`, over `timed-metadata`'s Teletext extractor. |
| `serde` | off | serde on dependency types. |

## Quick start

```rust
use caption_convert::{SourceFormat, TargetFormat, check, srt_to_webvtt, webvtt_to_srt};

let (srt, lossy) =
    webvtt_to_srt("WEBVTT\n\n00:00:00.000 --> 00:00:02.000\nHello\n").unwrap();
assert!(!lossy);
assert!(srt.starts_with("1\n"));
assert!(srt_to_webvtt(&srt).unwrap().starts_with("WEBVTT"));

// The matrix says up front what converts and what does not.
assert!(check(SourceFormat::Cea608, TargetFormat::Srt).is_ok());
assert!(check(SourceFormat::DvbBitmapSubtitle, TargetFormat::WebVtt).is_err());
```

CEA-608 (feature `cc-data`): `Cea608ToWebVtt::new(Cea608Channel::Cc1)`, then
`push_cc_data(pts_90k, &cc_data_bytes)` per access unit, `finalize(end_pts)`, and
`into_webvtt()` / `into_srt()`. See `examples/cc_to_webvtt.rs`.

## What is implemented

- `Cea608ToWebVtt` / `Cea708ToWebVtt` / `TeletextToWebVtt`: raw-bytes-in,
  WebVTT-or-SRT-out wrappers over `timed_metadata::webvtt` extractors (which own
  the roll-up / pop-on / paint-on cue-boundary detection). Shared finishing
  methods: `finalize`, `cues`, `into_cues`, `into_webvtt`, `into_srt`.
- `parse_webvtt` (returns `ParsedWebVtt` with a `lossy` flag), `write_document`,
  `write_segment`, `cue_block`; `parse_srt`, `write_srt`, `format_srt_timestamp`;
  `webvtt_to_srt`, `srt_to_webvtt`.
- `matrix`: `MATRIX`, `MatrixEntry`, `SourceFormat`, `TargetFormat`, `Support`,
  `lookup` (returns `Option`, never panics), `check` (returns `Error::Unsupported`
  with the reason).
- Hardened parsing: digits-only timestamps, end before start rejected, CR / CRLF
  folded to LF on write; timestamps and odd parity are delegated to
  `timed_metadata::webvtt` / `broadcast_common::parity` (one copy each).

## Design notes

- `no_std` + `alloc` core; `Error` and the format enums are `#[non_exhaustive]`.
- No wire format of its own, so the workspace byte round-trip invariant does not
  apply; behaviour is pinned by real-capture tests (real TS captions, SCC
  reference files, real SRT / WebVTT fixtures) instead.
- Gaps are explicit in types and docs, never degraded silently.

## Related crates

Depends on `timed-metadata` (cue extractors, WebVTT writer), `cc-data`, `dvb-vbi`,
`broadcast-common`. Siblings: `dvb-subtitle` (bitmap segments), `ttml-subtitle`
(TTML/IMSC parsing and validation).

## MSRV and licence

Rust **1.95.0**. MIT OR Apache-2.0.

## Examples

```sh
cargo run -p caption-convert --example cc_to_webvtt
cargo run -p caption-convert --example webvtt_to_srt
```
