# caption-convert 0.1.0

**Release date:** 2026-08-10

First release. Converts CEA-608/708 and EBU Teletext captions to WebVTT and
SRT, and WebVTT ↔ SRT. It wraps decoders this workspace already ships
(`cc-data`, `dvb-vbi`, `timed-metadata`) instead of reimplementing cue-boundary
handling — roll-up, pop-on and paint-on are the hard part, and they are already
solved in `timed-metadata` (issues #568/#666).

## The conversion matrix is the crate

`matrix::MATRIX` is the single source of truth for all 20 `SourceFormat` ×
`TargetFormat` pairs, pinned by a test asserting the cross-product is complete.
`matrix::check(from, to)` returns a typed
`Error::Unsupported { from, to, support, reason }` for any pair this crate does
not implement — **never silent empty output**, which is the failure mode a
caption converter is most likely to have and the hardest for a user to notice.

| from \ to | WebVTT | SRT | IMSC Text | IMSC Image |
|---|---|---|---|---|
| CEA-608 | Lossy | Lossy | Not implemented | Unsupported |
| CEA-708 | Lossy | Lossy | Not implemented | Unsupported |
| Teletext | Lossy | Lossy | Not implemented | Unsupported |
| DVB bitmap subtitle | **Unsupported** | **Unsupported** | **Unsupported** | Not implemented |
| TTML/IMSC | Not implemented | Not implemented | Lossless (identity) | Not implemented |

- **Lossy** — implemented; each module's docs state exactly what is dropped
  (placement, styling, window geometry).
- **Unsupported** — structurally impossible here. DVB bitmap subtitles are
  *pixels*; extracting text needs OCR, which is out of scope permanently.
- **Not implemented** — plausible future work, not built in this cut. DVB
  bitmap → IMSC Image would need RLE pixel decode, CLUT→RGBA, region/page
  compositing and a PNG/DEFLATE encoder written from scratch, because
  `dvb-subtitle` is deliberately carriage-only.

Read that table before assuming a pair converts. Roughly half of it does not.

## API

- `Cea608ToWebVtt` / `Cea708ToWebVtt` (feature `cc-data`) and
  `TeletextToWebVtt` (feature `teletext`), each with
  `into_webvtt()` / `into_srt()`.
- A WebVTT subset **reader**. `timed-metadata` only writes WebVTT; adding the
  parse direction is what makes the WebVTT ↔ SRT round trip genuine rather
  than write-only.
- SRT has no formal specification. This crate says so rather than citing one
  that does not exist, and implements the de facto ffmpeg/VLC format.

`no_std` + `alloc`; `std` is on by default. The two extractor backends are
gated independently, so a consumer that only needs Teletext does not build the
608/708 path.

## Fuzzing found four real defects before this shipped

Two libFuzzer targets (`caption_convert_webvtt`, `caption_convert_srt`) assert
the round-trip invariant — parse → write → re-parse yields an equal cue list —
rather than merely the absence of a panic. Each of these was found by them and
has a regression test:

- A lone `CR`, the third line terminator W3C WebVTT permits, was not
  recognised. It survived parsing as literal text and then vanished silently on
  write.
- `parse_srt`'s `block.trim()` removed meaningful leading and trailing spaces
  from a payload's boundary line, not just blank-line padding.
- `parse_webvtt` treated any whitespace-only line as a cue boundary where the
  spec says an *empty* one, silently truncating cues.
- `parse_timestamp`'s `h*3600 + m*60 + s` had no overflow check. WebVTT's hour
  field is uncapped, so a validly parsed value could overflow it: a panic under
  debug assertions, a silent wraparound in release.

## Fixtures

`fixtures/sub/sintel-en.srt` is real: the Blender Foundation's official English
subtitles for *Sintel*, CC BY 3.0, fetched verbatim from Wikimedia Commons with
the source URL, revision id and quoted licence recorded in
`fixtures/PROVENANCE.md`. Attribution: © Blender Foundation, www.sintel.org.

The three CEA-608 / Teletext / WebVTT fixtures are **synthetic**, and
`PROVENANCE.md` now says so — an earlier draft of this crate's CHANGELOG
described them as real. No permissively licensed real capture of CEA-608/708 or
Teletext carriage bytes is available; that has been investigated before in this
workspace and the dead end is documented there.

Tests carry mutation coverage: flipping the EOC wire bytes drops the expected
cue, XORing a Teletext parity bit yields U+FFFD, and corrupting the `WEBVTT`
signature returns a typed `Error::InvalidWebVtt`.

## Compatibility

Edition 2024, MSRV 1.95.0. Builds `--no-default-features` as `no_std` +
`alloc`, cross-compiled for `thumbv7em-none-eabi` by CI.
