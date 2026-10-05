# broadcast-hls 0.3.0

_Released 2026-10-05._

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
  never-silently-mangle discipline `AttrValue` already applies to quoted
  strings, now applied to decimals. `effective_part_hold_back()` returns
  `DecimalSeconds`, not `f64`. `AttrValue` gains hand-written
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
  renderers (`push_part_line`/`push_map_line`/`push_define_line`/
  `push_session_data_line`/`push_session_key_line`, all now
  `-> Result<()>`), so a caller that renders a tag line directly gets
  the same guarantee. `transmux::ts_hls::StreamingTsHlsSegmenter::playlist`
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
  `Vec<(String, AttrValue)>`, and `AttrValue` is now an opaque struct**
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
  - `AttrValue` no longer exposes public `Quoted`/`Bare` variants.
    Construct it through `AttrValue::quoted(s)` or `AttrValue::bare(s)`
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

---

Published from tag `broadcast-hls-v0.3.0`.
