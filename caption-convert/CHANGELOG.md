# Changelog — caption-convert

All notable changes to this crate. Format: [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Added
- Initial release (issue #931): caption/subtitle format conversion — CEA-608/
  708 and EBU Teletext to WebVTT/SRT, and WebVTT <-> SRT — wrapping the
  extractors `timed-metadata` already implements (issues #568/#666) rather
  than reimplementing cue-boundary handling (roll-up/pop-on/paint-on).
  - `Cea608ToWebVtt`/`Cea708ToWebVtt` (feature `cc-data`, layered on
    `cc-data`'s decode-only 608/708 model) and `TeletextToWebVtt` (feature
    `teletext`, layered on `dvb-vbi`'s carriage-only `TeletextDataField`),
    each with `into_webvtt()`/`into_srt()`.
  - A WebVTT subset reader (`timed-metadata` only writes WebVTT; this crate
    adds the missing parse direction) and a WebVTT <-> SRT pair, making that
    round trip genuine rather than write-only. SRT has no formal
    specification — documented as such rather than citing one that does not
    exist; the de facto ffmpeg/VLC format is implemented.
  - **The conversion matrix is the point of the crate.** `matrix::MATRIX` is
    the single source of truth for all 20 `SourceFormat` x `TargetFormat`
    pairs (pinned by a test asserting the cross-product is complete);
    `matrix::check(from, to)` returns a typed `Error::Unsupported { from, to,
    support, reason }` for any pair this crate does not implement — never
    silent empty output:

    | from \ to           | WebVTT | SRT    | IMSC Text      | IMSC Image     |
    |----------------------|--------|--------|-----------------|-----------------|
    | CEA-608              | Lossy  | Lossy  | Not implemented | Unsupported     |
    | CEA-708              | Lossy  | Lossy  | Not implemented | Unsupported     |
    | Teletext             | Lossy  | Lossy  | Not implemented | Unsupported     |
    | DVB bitmap subtitle  | **Unsupported** | **Unsupported** | **Unsupported** | Not implemented |
    | TTML/IMSC            | Not implemented | Not implemented | Lossless (identity) | Not implemented |

    DVB bitmap -> text is **Unsupported** (permanently out of scope — that
    conversion needs OCR); DVB bitmap -> IMSC Image is **NotImplemented**
    (would need RLE pixel decode, CLUT->RGBA, region/page compositing and a
    PNG/DEFLATE encoder from scratch, since `dvb-subtitle` is deliberately
    carriage-only); TTML/IMSC source conversions are **NotImplemented**,
    outside this cut. All reported here rather than half-built or silently
    dropped.
  - `no_std` + `alloc` (default features add `std`); `cc-data`/`teletext`
    features gate the two extractor backends independently.
  - Real fixtures throughout (`fixtures/cc/cea608_cc1_synthetic.txt`,
    `fixtures/teletext/teletext_subtitle_synthetic.txt`,
    `fixtures/sub/cap.vtt`), with mutation coverage: flipping the EOC wire
    bytes drops the expected cue, XORing a Teletext parity bit yields
    U+FFFD, and corrupting the `WEBVTT` signature returns a typed
    `Error::InvalidWebVtt` rather than empty output.
- Not reached, reported rather than implied: the DVB bitmap -> IMSC Image
  pipeline, TTML/IMSC source conversions, and the file/stream service wrapper
  issue #931 also mentions.

### Changed
- MSRV raised to **1.95.0** (issue #949), as a workspace-wide uplift; no
  functional or API change (one `collapsible_if` site adopted a let-chain).
