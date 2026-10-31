# Changelog — caption-convert

All notable changes to this crate. Format: [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

## [0.4.0] - 2026-10-31
### Added
- `TeletextToWebVtt::push_pes_data_field` parses a teletext PES data field with `dvb_vbi::DataField` and feeds its EBU teletext data units, plus `take_closed_cues` and `open_cue` for incremental cue output. Cues taken with `take_closed_cues` no longer appear in `cues()` or the `into_*` renderers.

### Fixed
- Teletext to WebVTT/SRT conversion now uses the page's national option (French, German, Italian and the others) and only the boxed subtitle text, so cues no longer carry "#" or "\\" for accented letters or leading spaces. Covered by a real-capture comparison against TSDuck's SRT text for `test-012.ts` (#1585).

## [0.3.0] - 2026-10-17
### Documentation
- README and crate-root docs restructured: explicit what-it-does / does-not (DVB bitmap not converted to text), standards table, full conversion matrix including WebVTT <-> SRT, feature table, compiled quick-start doctest.

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `cc-data` 0.6 -> 0.7, `dvb-vbi` 0.4 -> 0.5, `timed-metadata` 0.6 -> 0.7). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Changed
- Internal: the five finishing methods shared by `Cea608ToWebVtt`, `Cea708ToWebVtt` and `TeletextToWebVtt` come from one `converter_tail!` macro instead of three copies; public API unchanged (#1218).

### Fixed
- 608 -> WebVTT/SRT: captions loaded with a repeated RCL before EDM/EOC are converted instead of dropped, and same-PTS caption updates no longer yield zero-length cues (fixes in `cc-data` / `timed-metadata`, covered here by real-capture tests) (#1220).

### Changed (BREAKING)
- `matrix::lookup` returns `Option<&'static MatrixEntry>` instead of panicking (`.expect`) when the matrix has no row for a `(from, to)` pair; both format enums are `#[non_exhaustive]`, so a newly added format could reach the old panic. `check` maps the absent row to `Error::Unsupported { support: NotImplemented, .. }` (#1202).

### Changed
- Internal (behaviour preserved): `time.rs` (the SRT/WebVTT timestamp parser, `normalize_line_endings`, `to_hms_ms`, its own `TICKS_PER_MS`) and the local `unescape_payload` are deleted in favour of `timed_metadata::webvtt::{parse_timestamp, format_timestamp_with, normalize_line_endings, payload_lines, unescape_payload}`; `Error::InvalidTimestamp` still carries the offending text (#1210).
- Internal: odd-parity encode/check comes from `broadcast_common::parity` instead of a local copy (`timed_metadata::webvtt::teletext::{encode,decode}_odd_parity` stay as thin wrappers, public API unchanged); no behaviour change (#1212).

### Fixed
- `parse_srt` accepts the `.` millisecond separator (`00:00:01.000 --> ...`) again alongside `,`: the move to the shared `timed_metadata` timestamp grammar (#1210) had silently started rejecting it, which 0.2.0 accepted. `write_srt` still writes `,`; `srt_with_dot_millisecond_separator_still_parses` fails on the regressed code.
- `write_srt` now folds a lone CR (and CRLF) inside a cue's text to LF exactly like the WebVTT writer does (it used `str::lines()`, which only knows LF/CRLF, so a lone CR was written as a literal control character inside a payload line); `write_srt_folds_a_lone_cr_like_the_webvtt_writer` fails on the old code (#1210).

### Fixed
- Timestamps are digits-only: a `+` sign in any field (accepted by
  `str::parse`) is now rejected (#1109 CV-O1).
- `parse_webvtt` / `parse_srt` reject a cue whose end time precedes its start
  time (#1109 CV-O2).
- Timestamp parsing splits on the millisecond separator in place instead of
  allocating a normalised `String` per timestamp (#1109 CV-O3).

### Changed
- The conversion matrix no longer claims CEA-608/708/Teletext cue timing
  "carries over exactly": cue times are the carrying frame/packet PTS (#1109
  CV-W2).

## [0.2.0] - 2026-10-05
### Fixed
- **#1109** (reopens #974): `parse_webvtt` failed the *whole document* when
  the `WEBVTT` header contained any line other than `X-TIMESTAMP-MAP`
  (e.g. a `Kind:`/`Language:` metadata hint some encoders emit) — the line
  fell into the cue-block grouper and was misread as a cue identifier with
  no timing line after it. Now the entire header block (every line
  directly after the signature up to the first blank line, per W3C WebVTT
  SS4.1) is skipped and the document is flagged `lossy`, matching how any
  other unrepresentable construct is already handled.

## [0.1.1] - 2026-08-30

### Fixed
- **#974**: `parse_webvtt` rejected its own `write_segment` output — the
  `X-TIMESTAMP-MAP` header line was misread as a cue identifier. Now
  recognised and skipped during header parsing.
- **#975**: `parse_srt` rejected UTF-8 BOM-prefixed files. BOM is now
  stripped before parsing, matching `parse_webvtt`'s existing behaviour.
- **#976**: `write_srt` emitted self-rejecting output for cues containing
  empty interior lines (`"line1\n\nline3"`). The `\n\n` was interpreted as a
  block delimiter on re-parse. Empty lines within cue text are now skipped.

## [0.1.0] - 2026-08-11

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
  - Fixture-driven tests throughout (`fixtures/cc/cea608_cc1_synthetic.txt`,
    `fixtures/teletext/teletext_subtitle_synthetic.txt`,
    `fixtures/sub/cap.vtt` — all three **synthetic/hand-authored**, see
    `fixtures/PROVENANCE.md`), with mutation coverage: flipping the EOC wire
    bytes drops the expected cue, XORing a Teletext parity bit yields
    U+FFFD, and corrupting the `WEBVTT` signature returns a typed
    `Error::InvalidWebVtt` rather than empty output.
- Not reached, reported rather than implied: the DVB bitmap -> IMSC Image
  pipeline, TTML/IMSC source conversions, and the file/stream service wrapper
  issue #931 also mentions.
- A **real** fixture, `fixtures/sub/sintel-en.srt` -- the official English
  dialogue subtitles for Blender Foundation's *Sintel* (CC BY 3.0; see
  `fixtures/PROVENANCE.md` for the exact Wikimedia Commons source/revision) --
  plus a new `tests/srt_real_fixture.rs` exercising it (real-structure
  assertions, a `write_srt`/`parse_srt` round trip, a lossless SRT<->WebVTT
  conversion, and a timestamp mutation bite).
- Two `cargo-fuzz` targets, `caption_convert_webvtt` and `caption_convert_srt`
  (`fuzz/fuzz_targets/`), asserting the text-format round-trip invariant
  (parse -> write -> re-parse yields an equal `Cue` list) on arbitrary input,
  not just no-panic. Fuzzing them found and fixed four real bugs in the
  `webvtt.rs`/`srt.rs`/`time.rs` parsers:
  - A lone CR (W3C WebVTT SS4's third line-terminator form, alongside LF and
    CRLF) survived as a literal control character embedded in a cue's text
    and then silently vanished on the next write; both parsers now
    normalise all three terminator forms up front (`time::normalize_line_endings`).
  - `parse_srt`'s block-edge `str::trim()` ate a meaningful trailing/leading
    space on a payload's boundary line, not just the blank-line padding it
    was meant for; narrowed to `trim_matches('\n')`.
  - `parse_webvtt`'s block-boundary check (`line.trim().is_empty()`) treated
    any whitespace-only line as the blank line ending a cue, silently
    truncating a cue with a legitimate whitespace-only interior line;
    narrowed to the spec's actual rule, `line.is_empty()`.
  - `parse_timestamp`'s `h*3600+m*60+s` (then `*90` ticks) widening
    multiplication had no overflow check: WebVTT's hour field has no
    digit-count cap, so a validly-parsed huge `u64` hour overflowed it --
    panicking under debug-assertions, silently wrapping to a bogus timestamp
    in release. Now checked arithmetic, returning `Error::InvalidTimestamp`.

### Changed
- MSRV raised to **1.95.0** (issue #949), as a workspace-wide uplift; no
  functional or API change (one `collapsible_if` site adopted a let-chain).
