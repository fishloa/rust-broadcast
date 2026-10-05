# Changelog

All notable changes to `broadcast-hls` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-10-05
### Added

- New public API (all part of the 0.3.0 surface; most are described in the
  entries below): `DecimalSeconds` / `SignedDecimalSeconds` (checked duration
  newtypes), `AttrValue` (a validated quoted/bare attribute value),
  `parse_attribute_list` / `render_attribute_list`, `MediaSegment::title`,
  `MediaSegment::pre_tags` / `OpenSegment::pre_tags`, and the new `Error`
  variants `InvalidDecimalSeconds`, `InvalidSignedDecimalSeconds`,
  `InvalidQuotedString` and `InvalidUri`.

### Fixed
- **`cenc_ext_x_key` now emits `METHOD=SAMPLE-AES-CTR` for the `cenc`
  scheme instead of returning `None`** (audit BH-W1, #1111): the function
  and the module doc above it claimed "CTR is not a valid HLS METHOD",
  contradicting this crate's own `EncryptionMethod::SampleAesCtr` and RFC
  8216bis §4.4.4.4 — that method is exactly the `cenc` scheme for fMP4, and
  the spec says the `IV` attribute MUST NOT be present for it (none is
  emitted). `cenc`-protected CMAF can now be signalled in HLS at all;
  `CencScheme::Cbcs` is unchanged (`METHOD=SAMPLE-AES`).
- **`#EXT-X-RENDITION-REPORT` now requires `LAST-MSN`** (audit BH-W2,
  #1111): RFC 8216bis §4.4.5.3 makes it REQUIRED, but parse silently
  defaulted it to `0`, so `#EXT-X-RENDITION-REPORT:URI="b.m3u8"` parsed `Ok`
  into a report telling a client to request `_HLS_msn=0` — a long-gone
  segment — and re-rendering fabricated `LAST-MSN=0`. An absent attribute is
  now `Error::HlsParse`.
- **Segment-defining tags now re-render in place instead of being hoisted
  into one playlist-level block before every Media Segment** (audit BH-W5,
  #1111): `#EXT-X-KEY`, `#EXT-X-PROGRAM-DATE-TIME`, `#EXT-X-DATERANGE` and
  the `#EXT-X-CUE-*` family carry no typed field in this crate, so `parse()`
  kept them verbatim — but in a flat `MediaPlaylist::extra_tags` list that
  `to_m3u8` emitted as a single block *before all segments*. Per RFC 8216bis
  §4.4.1 such a tag applies "until the next occurrence of the tag or the end
  of the Playlist", so its position is load-bearing: a playlist with a
  `METHOD=AES-128` key A before segment 1 and key B before segment 50
  re-rendered with both keys at the top, applying key B to every segment and
  leaving segments 1-49 undecryptable, and every `#EXT-X-PROGRAM-DATE-TIME`
  landed at the top so the last one won and the wall-clock timeline was
  wrong. They now attach to the *following* segment (new public
  `MediaSegment::pre_tags` and `OpenSegment::pre_tags`) and are re-emitted
  immediately before it, so a key rotation, a per-segment PDT and a
  mid-playlist DATERANGE survive parse → render byte-for-byte; the open
  segment at the live edge keeps its own. Playlist-level unmodeled tags
  (`#EXT-X-MEDIA`, `#EXT-X-DEFINE`, …) still go to
  `MediaPlaylist::extra_tags`.
- **The strict RFC 8216bis §4.2 decimal grammar now gates every numeric
  attribute parse** (audit BH-W4, #1111): `PART-TARGET`, `PART-HOLD-BACK`,
  `HOLD-BACK`, `CAN-SKIP-UNTIL` and `EXT-X-PART DURATION` already went
  through it, but `EXT-X-VERSION`, `EXT-X-TARGETDURATION`,
  `EXT-X-MEDIA-SEQUENCE`, `EXT-X-DISCONTINUITY-SEQUENCE`, `EXT-X-BITRATE`,
  `BANDWIDTH`, `RESOLUTION`, `BYTERANGE`/`BYTERANGE-START`/
  `BYTERANGE-LENGTH`, `LAST-MSN`, `LAST-PART` and `SKIPPED-SEGMENTS` still
  used raw `from_str`, which accepts a leading `+` (`BANDWIDTH=+7` parsed)
  on integer fields; `#EXTINF`-style float fields were already strict.
  All integer attributes now share one strict `decimal-integer` lexer
  (no `+`, no exponent, no `nan`/`inf`, no decimal point), and the float
  and integer lexers are the only numeric entry points in the crate.
- **A Multivariant Playlist whose `#EXT-X-STREAM-INF` has no following URI
  line is now an error** (audit BH-W9, #1111): RFC 8216bis §4.4.6.2 requires
  the tag to be followed by the URI line of the Variant it describes. A
  second `#EXT-X-STREAM-INF` before that line silently **overwrote** the
  pending Variant, and EOF with one pending silently **dropped** it — either
  way a truncated multivariant playlist lost a variant with no error, and no
  downstream validator could flag it because the information was already
  gone from the parsed struct. Both now return `Error::HlsParse`.
### Changed
- **Docs only — no behavior change.** `MasterPlaylist::extra_tags` was
  documented as emitting its verbatim lines "after the variant/I-frame-variant
  entries"; `to_m3u8` actually emits them **first**, before
  `#EXT-X-INDEPENDENT-SEGMENTS`/`DEFINE`/`START`, the `SESSION-*` tags and the
  variant list (audit BH-W10, #1111). A caller relying on the old wording to
  place a `#EXT-X-DEFINE` (carried in `extra_tags`) after the variants that
  use it got the actual, opposite behaviour.
- **`#EXTINF` titles now survive the round trip** (audit BH-W11,
  #1111): the title after the duration's comma (RFC 8216bis §4.4.4.1)
  was split off and discarded, so any playlist carrying one lost it on
  re-render. `MediaSegment` gains a `title: Option<String>` field —
  `None` (the common case) renders the bare `#EXTINF:<dur>,` exactly
  as before, so existing output is unchanged; a title is validated
  like any other quoted-string (a CR or LF in it is rejected, per
  BH-W7).
### Changed (breaking)
- `broadcast_hls::Error` no longer derives `Eq` (now `PartialEq` only): the new `InvalidDecimalSeconds`/`InvalidSignedDecimalSeconds` variants carry an `f64`. Code requiring `Error: Eq` must relax the bound.
- **`Variant::codecs` is now `Option<String>`** (audit BH-W6, #1111): the
  `CODECS` attribute of `#EXT-X-STREAM-INF` (RFC 8216bis §4.4.6.2) is
  optional, but an absent one was parsed into `codecs: String = ""` and the
  renderer emitted the invalid empty `CODECS=""` — a malformed codec list a
  strict client (Apple `mediastreamvalidator`, AVPlayer) rejects. `None` now
  round-trips: the attribute is simply not rendered, matching
  `IFrameVariant::codecs`, which already modelled it this way.
- **`LowLatencyConfig::part_target`/`part_hold_back` are now
  `Option<DecimalSeconds>` and `effective_part_hold_back()` returns
  `Option<DecimalSeconds>`** (audit BH-W3, #1111): a Media Playlist may
  carry `#EXT-X-SERVER-CONTROL` (blocking reload or delta updates) without
  any Partial Segments, which RFC 8216bis §4.4.3.8 allows. Previously such
  a playlist parsed into `low_latency = Some` with both fields defaulted
  to `0`, and `to_m3u8` unconditionally re-rendered a bogus
  `#EXT-X-PART-INF:PART-TARGET=0` and `PART-HOLD-BACK=0`, telling clients
  the stream was LL-HLS with a zero part target. The two tags now render
  only when actually present (parse or explicitly set).
- **New checked newtypes `DecimalSeconds`/`SignedDecimalSeconds` replace
  `f64`/`Option<f64>` on every public duration field** (issue #1140,
  coordinator follow-up to BH-W4/T12): `MediaSegment::duration`,
  `PartSpec::duration`, `StartPoint::time_offset`,
  `LowLatencyConfig::part_target`/`part_hold_back`/`can_skip_until`/
  `hold_back`. Each is validated once at construction —
  `DecimalSeconds::new`/`SignedDecimalSeconds::new` (`-> Result<Self,
  Error>`, new `Error::InvalidDecimalSeconds`/`InvalidSignedDecimalSeconds`
  variants) reject NaN/infinite (and, for the unsigned form, negative)
  values; `get()` reads the validated `f64` back. This closes the last gap
  BH-W4's parser-side fix left open: a struct built directly (bypassing
  `MediaPlaylist::parse`) could still hold a NaN/inf/negative duration,
  which the (necessarily infallible) `to_m3u8` renderer previously had to
  silently clamp to `"0"` rather than reject — the same
  never-silently-mangle discipline `AttrValue` applies to quoted
  strings, now applied to decimals. `effective_part_hold_back()` returns
  `DecimalSeconds`, not `f64`. `AttrValue` has hand-written
  `Serialize`/`Deserialize` (feature `serde`) that goes through
  `quoted`/`bare` on deserialize, so a downstream crate's `#[derive(...)]`
  struct holding `Vec<(String, AttrValue)>` can't deserialize an
  unvalidated value either.
- **`to_m3u8()` is now fallible — `Result<String>`** (audit BH-W7,
  #1111): a model field rendered inside an RFC 8216 §4.2 quoted-string
  (`CODECS`, `KEYFORMAT`, `SERVER-URI`, `DATA-ID`/`VALUE`/`LANGUAGE`,
  `#EXT-X-DEFINE` `NAME`/`VALUE`, …) or emitted as a URI (segment,
  part, map, variant, rendition-report, preload-hint) that contains
  `"`, CR or LF is now rejected (`Error::InvalidQuotedString` /
  `Error::InvalidUri`) instead of being written out raw. Those three
  characters are forbidden in a quoted-string, and a newline in an
  unmodeled or caller-supplied URI (`"seg.m4s\n#EXT-X-ENDLIST"`)
  injected a whole tag line into the rendered playlist while an
  embedded `"` broke out of the attribute list and truncated the
  value. The same characters are now rejected by the per-tag
  renderers (the private `push_part_line`/`push_map_line`/`push_define_line`/
  `push_session_data_line`/`push_session_key_line`, internally now
  `-> Result<()>`; not public API). `transmux::ts_hls::StreamingTsHlsSegmenter::playlist`
  and `multimux::catchup::render_playlist` are fallible in step.
- **`LowLatencyConfig::extra_attrs` is removed** (audit BH-W8, #1111):
  the struct carried each unmodeled `#EXT-X-SERVER-CONTROL` /
  `#EXT-X-PART-INF` / `#EXT-X-PRELOAD-HINT` attribute twice — once in
  the per-tag `sc_extra_attrs`/`pi_extra_attrs`/`ph_extra_attrs` lists
  the renderer actually reads, and once in a playlist-wide
  `extra_attrs` union. The §8 row-12 `REQ-` version scan read only the
  union, so a config built directly with e.g.
  `sc_extra_attrs = [("REQ-X","1")]` rendered `REQ-X` while leaving
  `#EXT-X-VERSION` at its lowered value, an under-declaration §8
  forbids. The three per-tag lists are now the single source: the scan
  covers all three and the union is gone. `parse` is unaffected (it
  always populated all four identically).
- **Every `extra_attrs: Vec<(String, String)>` field is now
  `Vec<(String, AttrValue)>`, and the new `AttrValue` is an opaque struct**
  (issue #1045 / audit BH-C1, T12): quoting is recorded losslessly from
  the real wire token at parse time instead of being guessed on render
  from a small table of known RFC 8216bis attribute names. Previously,
  rendering an unmodeled quoted-string attribute (`AUDIO`, `VIDEO`,
  `SUBTITLES`, `CLOSED-CAPTIONS` unless `NONE`, `PATHWAY-ID`,
  `STABLE-VARIANT-ID`, `STABLE-RENDITION-ID`, `SUPPLEMENTAL-CODECS`,
  `REQ-VIDEO-LAYOUT`, `ALLOWED-CPC`, or any attribute this crate hasn't
  been taught about — e.g. a private `X-` extension) could silently drop
  its surrounding `"`: `AUDIO="a1"` round-tripped to the invalid
  `AUDIO=a1`, which a strict client (Apple `mediastreamvalidator`,
  AVPlayer) rejects or misinterprets. Fixed for every attribute name, known
  or not. Verified against a committed real Apple HLS fixture, a private
  `X-` attribute, and `mediastreamvalidator` directly.
  - `AttrValue` is new in 0.3.0 (it did not exist at 0.2.1) and is an opaque
    struct with no public `Quoted`/`Bare` variants. Construct it through `AttrValue::quoted(s)` or `AttrValue::bare(s)`
    (both `-> Result<Self, Error>`), or `AttrValue::for_attr(name, value)`
    (a convenience choosing `quoted`/`bare` from the same known-name
    table the old fix used, for a caller that doesn't already know the
    correct kind — also fallible). Read a value back with `as_str()`/
    `is_quoted()`.
  - **T12**: a quoted value containing `"`, CR or LF (forbidden by RFC
    8216 §4.2), or a bare value containing a character that would
    require quoting (`,`, `"`, CR, LF, whitespace — reachable from
    caller-supplied data, e.g. a third-party ad-decision service feeding
    `ssai-runtime`) is now **rejected at construction** (`Err`) rather
    than silently mangled (e.g. percent-encoded) or emitted raw, closing
    a line/attribute-list-injection vector without ever letting an
    invalid value exist. Rendering (`to_m3u8`) stays infallible — the
    checks live entirely in the `AttrValue` constructors, so a value that
    reaches rendering is already known-valid; the attribute-list renderer's
    signature is unchanged (see below for its rename), and callers across
    the workspace (`hls-runtime`, `multimux`, `ssai-runtime`, `transmux`,
    `media-doctor`) are unaffected since they only ever populate
    `extra_attrs` from parsed values or `Vec::new()`.
- **The private attribute-list tokenizer/renderer are now one public
  API — `parse_attribute_list`/`render_attribute_list`** (issue #1140,
  audit T12/r14-SSAI-O1): `ssai-runtime` and `timed-metadata` each carried
  their own copy of the quoted-comma-honouring splitter, and both
  `ssai-runtime`'s interstitial `DATERANGE` and `timed-metadata`'s base
  `DATERANGE` hand-formatted `,NAME="VALUE"` with no validation of the
  interpolated content — an injection vector when that content is
  ad-decision-service or upstream-SCTE-35-`segmentation_upid` data. Both
  crates now build a `Vec<(String, AttrValue)>` and call
  `render_attribute_list`, so a `"`/CR/LF in any value is rejected before
  it ever reaches the wire, not just for attributes this crate models.
  (Renamed from the former private `parse_attr_list`/`push_extra_attrs`.)
- **`cenc_ext_x_key` now returns `Result<Option<String>>`** (was
  `Option<String>`): `key_uri` is validated through `AttrValue::quoted`
  and rendered through `render_attribute_list` instead of hand-formatting
  the tag (issue #1140 / audit r05-W10 — the sibling copy in `transmux`'s
  `ExtXKey::to_tag` had the same bug; both now share this validation
  discipline), so a `"`/CR/LF in a key-server URL is now `Err` instead of
  producing a malformed or injectable tag line.
- **`EXTINF`/`PART-TARGET`/`PART-HOLD-BACK`/`CAN-SKIP-UNTIL`/`HOLD-BACK`/
  `EXT-X-PART DURATION`/`TIME-OFFSET` now parse a strict RFC 8216bis §4.2
  decimal-floating-point grammar** (issue #1140 / audit BH-W4), rejecting
  `nan`/`inf`/`infinity`/exponent notation/a leading `+`/an out-of-place
  `-` that `f64::from_str` alone accepts but the spec does not.
  `#EXTINF:nan,` previously parsed `Ok` into a NaN duration that a
  downstream range check silently treated as in-range, or that panicked a
  later `Duration::from_secs_f64`. `format_secs`/`format_extinf` also now
  clamp a non-finite or negative value to `"0"` rather than ever emitting
  `"NaN"`/`"inf"`/a stray `-`, for the (parser-unreachable) case of a
  struct built directly with an invalid `f64` field.

## [0.2.1] - 2026-08-14
### Fixed
- `MediaPlaylist::parse` now rejects `BYTERANGE` (on segments, `EXT-X-MAP`,
  and `EXT-X-PART`) whose `offset + length` overflows `u64`, and
  `EXT-X-PRELOAD-HINT` whose `BYTERANGE-START + BYTERANGE-LENGTH` overflows.
  Previously these parsed cleanly and any downstream arithmetic on the pair
  could wrap in release builds or panic in debug (issue #958).

## [0.2.0] - 2026-08-11

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.

## [0.1.1] - 2026-08-08

### Fixed
- **Docs only — no code behavior changed.** `README.md`'s "Round-trip
  fidelity" section described `#EXT-X-VERSION` backwards (issue #935):
  it claimed the tag is "always emitted, even if the input omitted it" and
  that this crate "renders the `version` field as given" without computing
  a §8 minimum. Both were the *inverse* of the actual (and correct, per
  issue #871) behavior: `to_m3u8()` computes the tag from playlist content
  (`computed_version()`) and omits it entirely when nothing triggers a
  version requirement; `version` is a floor, not the rendered value. The
  README now matches `src/lib.rs`'s module doc.
- **"All 32 tags parse and serialize" corrected to the true split.**
  `README.md` claimed all 32 RFC 8216bis §4.4 tags parse *and* serialize
  through a typed field. Four — `#EXT-X-KEY`, `#EXT-X-PROGRAM-DATE-TIME`,
  `#EXT-X-DATERANGE`, `#EXT-X-MEDIA` — have no typed field and are carried
  opaquely in `extra_tags` (recognized, round-tripped verbatim, but not
  exposed as structured data). The README now states 28 typed / 4 opaque.
- **`tests/hls_tag_completeness.rs`'s drift guard is now behavioral.** The
  original guard only grepped `src/` for each of the 32 tag names — a
  presence check satisfiable by a doc comment, unable to detect a missing
  or deleted typed `parse` handler (it admitted this in its own doc
  comment). Two new tests parse a fixture playlist carrying every one of
  the 32 tags and assert: for each of the 28 typed tags, the corresponding
  struct field is actually populated; for the 4 opaque tags, the tag line
  survives verbatim in `extra_tags`. Verified to bite: temporarily removing
  the `#EXT-X-GAP` parse arm turns `typed_tags_populate_their_struct_field_on_parse`
  red while the old presence-only test stays green.

## [0.1.0] - 2026-08-02

### Added
- `LowLatencyConfig::hold_back` (`Option<f64>`) — the `HOLD-BACK` attribute
  of `#EXT-X-SERVER-CONTROL` (RFC 8216bis §4.4.3.8). Parsed from and rendered
  to the `#EXT-X-SERVER-CONTROL` line; absent when `None`.
- `LowLatencyConfig::can_skip_dateranges` (`bool`) — the
  `CAN-SKIP-DATERANGES` attribute of `#EXT-X-SERVER-CONTROL`
  (RFC 8216bis §4.4.3.8). Rendered only when `can_skip_until` is `Some`.

### Changed
- **`EXT-X-SESSION-KEY` now rejects `METHOD=NONE` at parse time**
  (RFC 8216bis §4.4.6.5 MUST NOT). Previously accepted and round-tripped
  unchanged; the module doc previously documented this as an intentional gap.

### Fixed
- **An exactly-whole `#EXTINF` duration now renders as an integer** (`4.0` ->
  `#EXTINF:4,`, was `#EXTINF:4.000,`) — a conformance fix, found while
  testing issue #873's classic TS-HLS output. RFC 8216bis §8 row 3 requires
  `EXT-X-VERSION` >= 3 for a playlist that *contains* floating-point
  `EXTINF` values, and §4.4.4.1 conversely requires durations to be integers
  when the compatibility version is below 3. `to_m3u8` rendered `4.000` — a
  floating-point value — while `computed_version()` reported no requirement,
  so the emitted playlist declared itself version-1 compatible and then
  handed a v1/v2 client a duration it cannot parse.
  - `is_fractional_duration` (the §8 row-3 predicate) is now **defined as**
    "does `format_extinf` emit a decimal point", so the renderer and the
    version derivation cannot diverge again. They had diverged in both
    directions: the integral case above, and — since the sub-millisecond
    precision fix — a `4.0004` that rendered at full precision while the
    integer-millisecond predicate still called it integral.
  - Issue #882's precision work is unaffected: `9.9766` still renders as
    `9.9766` and `2.00004` still does not collapse to `2`. Only the
    exactly-integral case changed.
  - `#EXT-X-PART`'s `DURATION` never had the bug — it already routed through
    `format_secs`, which renders a whole value as an integer.
- `EXT-X-VERSION` is now **computed** from the playlist's actual content
  (RFC 8216bis §8), not chosen ahead of time (issue #871): `to_m3u8` takes
  the `max()` of the minimums the content triggers, per the feature-to-row
  table transcribed at `docs/version-compatibility.md`. A playlist that
  triggers nothing emits no `EXT-X-VERSION` tag at all (per §8's opening
  rule). This fixes a real over-declaration bug — `hls-runtime`'s LL-HLS
  origin previously baked in a hardcoded version 9 even though none of the
  low-latency tags it emits carry any version requirement; the true minimum
  for its fMP4 playlist is 6, and over-declaring locked out every client on
  protocol version 6/7/8 (RFC 8216 §7: a client MUST NOT play back a
  version it does not support).
- `MediaPlaylist::version`/`MasterPlaylist::version` stay settable as an
  explicit floor rather than becoming computed-only: `0` means "no explicit
  floor"; a nonzero value is raised — never lowered — to the computed
  minimum, so an explicit value can never silently under-declare an invalid
  playlist. New `MediaPlaylist::computed_version`/
  `MasterPlaylist::computed_version` expose the derived minimum directly.
- `MasterPlaylist` gained its own `extra_tags: Vec<String>` (mirroring
  `MediaPlaylist::extra_tags`): `parse` now preserves an unrecognized
  `#EXT-...` tag (e.g. `#EXT-X-MEDIA`, `#EXT-X-DEFINE`) verbatim instead of
  silently dropping it, and `to_m3u8` re-renders it. This closes a
  previously-documented round-trip gap and is also the substrate
  `computed_version` scans for the §8 rows this crate does not (yet) model
  with typed fields (rows 7/8/11/12/13).
- **Stale documentation corrected in README and MANIFEST.md** (issue #890):
  `MasterPlaylist::extra_tags` was documented as non-existent ("no
  `extra_tags` escape hatch on `MasterPlaylist`") despite being fully
  wired since the previous release. The README's round-trip-fidelity
  section now correctly distinguishes "reordered" (tag ordering is
  canonical rather than input-preserving) from "dropped" (no longer true —
  every parsed tag survives). MANIFEST.md's §9.10/§9.11 Findings section
  was similarly corrected.
- **Unknown attributes on modeled tags are now retained** (issue #884).
  Every tag struct that carries an attribute list now holds an
  `extra_attrs: Vec<(String, String)>` for attribute names this crate
  does not model. These survive parse → serialize and feed the RFC
  8216bis §8 row 12 `REQ-` version-derivation check, which previously
  only fired on unmodeled tags in `extra_tags`.

### Testing
- `tests/spec_fixture_version.rs` cross-checks the derivation against the
  RFC's own §9 example playlists (`fixtures/hls/spec/`, issue #877) — the
  only independent check, since every other version test compares the code
  against our own reading of the transcription. All three §9 Media Playlist
  examples that declare a version (9.1/9.2/9.3) compute exactly the `3` the
  spec authors declared, and all five Multivariant examples
  (9.4/9.5/9.6/9.7/9.12) compute `None`, agreeing with the authors' choice
  to leave them untagged. Compares `computed_version()` rather than rendered
  output, because a parsed playlist's `version` field acts as a floor and
  would make a rendered comparison circular.
- Two hand-built fixtures covering the tag combinations demonstrated by the
  RFC's abridged §9 examples (issue #890):
  `handbuilt/daterange-scte35-media.m3u8` (EXT-X-DATERANGE with SCTE35-OUT
  and SCTE35-IN, the DATERANGE lines landing in `extra_tags` because the tag
  is not structurally modeled — derived from §9.10 by supplying the Media
  Segments the spec elides with `...`) and
  `handbuilt/low-latency-parts-preload-report.m3u8` (EXT-X-PART,
  EXT-X-PRELOAD-HINT, EXT-X-RENDITION-REPORT, EXT-X-DISCONTINUITY, and
  EXT-X-MAP — all as typed fields — derived from §9.11 by replacing its
  leading `...` with a real LL-HLS header block). Both round-trip and their
  `computed_version()` matches the derivation recorded in MANIFEST.md.
- `tests/hls_fixture_corpus.rs::unmodeled_ext_x_media_survives_master_parse_round_trip`
  proves an unmodeled `EXT-X-MEDIA` tag on a Multivariant Playlist survives
  parse→serialize→re-parse; a mutation that disables the `extra_tags`
  retention arm makes it fail (issue #890).

### Added
- Initial release. HLS (M3U8) playlist syntax (RFC 8216 / RFC 8216bis)
  extracted from `transmux/src/hls.rs` (issue #878): `MediaPlaylist`,
  `MasterPlaylist`, `MediaSegment`, `Variant`, `IFrameVariant`,
  `LowLatencyConfig`, `OpenSegment`, `PartSpec`, `MapTag`, `ByteRange`,
  `PreloadHintType`, `RenditionReport`, `SkipInfo`, `mark_init_discontinuities`,
  `cenc_ext_x_key`. `#![no_std]` + `alloc`; depends only on `broadcast-common`;
  builds for `thumbv7em-none-eabi`.
- This is a pure move plus one adaptation forced by the dependency direction
  (`transmux` now depends on this crate, not the reverse): a crate-local
  `Error` type, replacing `transmux::Error::HlsParse`. No parsing or rendering
  behaviour changed.
- `CencScheme` (which `cenc_ext_x_key` takes) is **re-exported from
  `broadcast-common` 9.2**, not redefined here — it is the very same type
  `transmux` uses, so nothing converts at the boundary. CENC is *Common*
  Encryption, a container-independent scheme identity, so it lives below both
  crates rather than once per crate (issues #564, #878). `hex_encode` comes
  from `broadcast_common::hex` for the same reason.
- Requires `broadcast-common` **9.2** for those two items.
- The remaining 9 of RFC 8216bis §4.4's 32 tags (issue #872), all parsing
  *and* serializing with a round-trip test:
  `#EXT-X-INDEPENDENT-SEGMENTS` (`MediaPlaylist`/`MasterPlaylist`
  `independent_segments: bool`), `#EXT-X-START` (`StartPoint`,
  `MediaPlaylist`/`MasterPlaylist` `start`), `#EXT-X-DEFINE` (`Define`,
  `MediaPlaylist`/`MasterPlaylist` `defines`), `#EXT-X-PLAYLIST-TYPE`
  (`PlaylistType`: `Vod`/`Event`, `MediaPlaylist::playlist_type`),
  `#EXT-X-GAP` (`MediaSegment::gap`), `#EXT-X-BITRATE`
  (`MediaSegment::bitrate`, carry-forward + dedup-render like
  `MediaSegment::map`), `#EXT-X-SESSION-DATA` (`SessionData`,
  `SessionDataContent`, `SessionDataFormat`, `MasterPlaylist::session_data`),
  `#EXT-X-SESSION-KEY` (`SessionKey`, `EncryptionMethod`,
  `MasterPlaylist::session_keys`), `#EXT-X-CONTENT-STEERING`
  (`ContentSteering`, `MasterPlaylist::content_steering`). All 32 §4.4 tags
  now parse; `tests/hls_tag_completeness.rs` is a drift-guard enumerating
  all 32 by name so a future spec revision (or a regression) surfaces as a
  red test. Three new hand-built fixtures under `fixtures/hls/handbuilt/`
  (authored from the confirmed attribute grammar, for the tags the spec's
  own §9 examples don't cover as complete playlists) join the corpus in
  `tests/hls_fixture_corpus.rs`, which now also asserts a
  parse → serialize → re-parse round trip for **every** passing fixture in
  every tier. Round-trip divergences (unmodeled `#EXT-X-MEDIA`, canonical
  tag ordering, always-emitted `#EXT-X-VERSION`, dropped whitespace/
  comments) are enumerated in the README.

- **Integration with the §8 version derivation (#871/#880).** The rows that
  read tags #872 made typed now read the typed data instead of
  string-matching `extra_tags`: **row 11** (`EXT-X-DEFINE` with
  `QUERYPARAM`) reads `defines`, and **row 8** (variable substitution) also
  scans the typed string fields (`EXT-X-DEFINE` values, `EXT-X-SESSION-DATA`
  `VALUE`/`URI`, `EXT-X-SESSION-KEY` `URI`, `EXT-X-CONTENT-STEERING`
  `SERVER-URI`, plus the already-typed `EXT-X-MAP`/`EXT-X-PART`/preload-hint/
  rendition-report URIs). Without this, row 11 would have silently stopped
  firing the moment `EXT-X-DEFINE` became typed, since a parsed tag no longer
  reaches `extra_tags`. Rows 7/12/13 still scan `extra_tags` — they are
  attributes of `EXT-X-MEDIA`, which this crate still does not model.
  `fixtures/hls/MANIFEST.md`'s per-fixture version derivations are now
  asserted by `tests/spec_fixture_version.rs`, making that table executable.
- **Known gap (documented, tested):** §8 row 12 (`REQ-` attribute) is matched
  only on tags that reach `extra_tags`. A `REQ-` attribute on a tag this
  crate models with typed fields is discarded at parse time and cannot reach
  the check. Closing it needs unknown-attribute retention on every modeled
  tag — an API change beyond this issue — so it is pinned by
  `req_attribute_on_a_modeled_tag_is_a_known_gap` rather than left implicit.

### Fixed
- **Sub-millisecond durations were silently corrupted on render.**
  `to_m3u8()` emitted `#EXTINF` via a hardcoded `{:.3}` and every other
  seconds value (`EXT-X-PART:DURATION`, `PART-TARGET`, `PART-HOLD-BACK`,
  `CAN-SKIP-UNTIL`, `EXT-X-START:TIME-OFFSET`) via integer-millisecond
  math, so any finer value was rounded away: Apple's real
  `#EXTINF:9.9766` came back as `9.977`, and RFC 8216bis §9.11's
  `DURATION=2.00004` as `2`. Rendering is now lossless — the compact
  historical form is kept whenever it re-parses bit-exactly (so ordinary
  ms-granular output is byte-for-byte unchanged), otherwise the shortest
  exactly-round-tripping decimal is emitted. Caught by round-tripping the
  real `fixtures/hls/real/` Apple playlists; no hand-made fixture in the
  repo could have surfaced it, since all were authored at exactly the
  3-decimal precision the bug preserved.
