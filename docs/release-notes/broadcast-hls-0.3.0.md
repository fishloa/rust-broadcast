# broadcast-hls 0.3.0

_Released 2026-10-05._

Breaking (0.x minor) release of the HLS (M3U8) playlist syntax crate. It fixes a run of audit defects (BH-W1 to BH-W11, #1111; BH-C1, #1045; #1140) in which the parser or renderer silently mangled a playlist: lost or hoisted key and date tags, fabricated `LAST-MSN=0` and `PART-TARGET=0`, dropped quotes around attribute values, and accepted `NaN` durations or CR/LF in URIs. The fix is to make invalid data unrepresentable, and that changes the public types. **Breaking for** every caller that reads or builds `MediaSegment`, `PartSpec`, `LowLatencyConfig`, `Variant` or any `extra_attrs` field, and for everyone who calls `to_m3u8()` (now fallible) or `cenc_ext_x_key`. Playlists that previously rendered with a corrupt or injected line now return `Err`, and playlists that carried per-segment `EXT-X-KEY`/`EXT-X-PROGRAM-DATE-TIME` now re-render those tags in place rather than at the top. Callers in this workspace that must move with it: `transmux` ([transmux-0.25.0.md](transmux-0.25.0.md)), `hls-runtime` ([hls-runtime-0.7.0.md](hls-runtime-0.7.0.md)), `ssai-runtime` ([ssai-runtime-0.2.0.md](ssai-runtime-0.2.0.md)), `timed-metadata` ([timed-metadata-0.6.0.md](timed-metadata-0.6.0.md)), `multimux` ([multimux-0.11.0.md](multimux-0.11.0.md)) and `media-doctor` ([media-doctor-0.9.0.md](media-doctor-0.9.0.md)). The only manifest change is `broadcast-common` 9.3 to 9.4; MSRV and features are unchanged.

## Breaking changes

All code below uses names that exist in 0.3.0 (checked against `broadcast-hls/src/lib.rs`).

### `to_m3u8()` and the tag renderers are fallible

`MediaPlaylist::to_m3u8` and `MasterPlaylist::to_m3u8` return `Result<String>` (was `String`) (BH-W7). A model field rendered inside a quoted-string (`CODECS`, `KEYFORMAT`, `SERVER-URI`, `DATA-ID`/`VALUE`/`LANGUAGE`, `EXT-X-DEFINE` `NAME`/`VALUE`, ...) or emitted as a URI (segment, part, map, variant, rendition-report, preload-hint) that contains `"`, CR or LF is rejected with `Error::InvalidQuotedString` or `Error::InvalidUri`. Before, a URI such as `"seg.m4s\n#EXT-X-ENDLIST"` injected a whole tag line and an embedded `"` truncated the attribute list. The internal per-tag renderers (`push_part_line`, `push_map_line`, `push_define_line`, `push_session_data_line`, `push_session_key_line`) are `-> Result<()>` for the same reason; they are not public, so this affects you only through `to_m3u8`.

```rust
// 0.2
let text: String = playlist.to_m3u8();
// 0.3
let text: String = playlist.to_m3u8()?;
```

### Durations are `DecimalSeconds` / `SignedDecimalSeconds`

`MediaSegment::duration`, `PartSpec::duration`, `LowLatencyConfig::{part_target, part_hold_back, can_skip_until, hold_back}` and `StartPoint::time_offset` no longer hold a raw `f64`. `DecimalSeconds::new(f64) -> Result<Self>` rejects NaN, infinite and negative values (`Error::InvalidDecimalSeconds`); `SignedDecimalSeconds::new` rejects NaN and infinite (`Error::InvalidSignedDecimalSeconds`); `get()` returns the checked `f64`. Parsing already rejected these (below); the newtypes close the hole for structs built directly, where `to_m3u8` previously had to clamp a bad value to `"0"`.

```rust
// 0.2
let seg = MediaSegment { duration: 6.0, /* ... */ };
// 0.3
let seg = MediaSegment { duration: DecimalSeconds::new(6.0)?, /* ... */ };
let secs: f64 = seg.duration.get();
```

### `LowLatencyConfig` optional part fields; `extra_attrs` removed

`LowLatencyConfig::part_target` and `part_hold_back` are `Option<DecimalSeconds>`, and `effective_part_hold_back()` returns `Option<DecimalSeconds>` (BH-W3). A playlist may carry `#EXT-X-SERVER-CONTROL` (blocking reload, delta updates) without any Partial Segments (RFC 8216bis §4.4.3.8). It used to parse into `low_latency = Some` with both fields `0` and re-render a bogus `#EXT-X-PART-INF:PART-TARGET=0` and `PART-HOLD-BACK=0`; those tags now render only when present. `LowLatencyConfig::extra_attrs` is removed (BH-W8): the three per-tag lists `sc_extra_attrs`, `pi_extra_attrs` and `ph_extra_attrs` are the single source, and the `REQ-` version scan now covers all three (before, a directly built `sc_extra_attrs = [("REQ-X","1")]` rendered while `#EXT-X-VERSION` stayed lowered, an under-declaration the spec forbids). `parse` is unaffected.

### `extra_attrs` hold `AttrValue`

Every `extra_attrs` field (and `sc_/pi_/ph_extra_attrs`) changed from `Vec<(String, String)>` to `Vec<(String, AttrValue)>` (#1045, BH-C1). `AttrValue` is new in this release (0.2.x held plain `String` pairs). The old renderer guessed from a short list of known names whether to re-quote a value, so an unmodeled quoted attribute such as `AUDIO="a1"`, `PATHWAY-ID`, `SUPPLEMENTAL-CODECS` or a private `X-` extension re-rendered as the invalid `AUDIO=a1`, which Apple's `mediastreamvalidator` and AVPlayer reject. Quoting is now recorded from the wire token at parse time, for every attribute name. `AttrValue` is an opaque struct: build it with `AttrValue::quoted(s)` or `AttrValue::bare(s)` (both `-> Result<Self>`), or `AttrValue::for_attr(name, value)` when you do not know the kind; read it with `as_str()` and `is_quoted()`. A quoted value containing `"`, CR or LF, or a bare value containing `,`, `"`, CR, LF or whitespace, is rejected at construction, so such a value can never reach the renderer.

```rust
// 0.2
extra_attrs.push(("X-CUSTOM".to_string(), "v".to_string()));
// 0.3
extra_attrs.push(("X-CUSTOM".to_string(), AttrValue::quoted("v")?));
```

With the `serde` feature, `AttrValue` has hand-written `Serialize`/`Deserialize` that re-validate through `quoted`/`bare` on deserialize.

### Other type changes

- `Variant::codecs` is `Option<String>` (BH-W6). `CODECS` is optional (RFC 8216bis §4.4.6.2); an absent one used to parse into `""` and render the invalid `CODECS=""`. `None` now round-trips and renders nothing.
- `cenc_ext_x_key` returns `Result<Option<String>>` (was `Option<String>`): `key_uri` goes through `AttrValue::quoted` and `render_attribute_list`, so a `"`, CR or LF in a key-server URL is `Err` instead of a malformed tag (#1140, r05-W10).
- The private attribute-list helpers are now one public pair, replacing the three copies in this workspace: `parse_attribute_list(&str) -> (BTreeMap<String, String>, BTreeSet<String>)` (name to value map, plus the names that were quoted on the wire) and `render_attribute_list(&mut String, &[(String, AttrValue)])`. `ssai-runtime` and `timed-metadata` now use them.
- `broadcast_hls::Error` no longer implements `Eq` (it derives only `PartialEq`), because `InvalidDecimalSeconds` and `InvalidSignedDecimalSeconds` carry an `f64`. Code that needs `Eq` on it, or on a type that embeds it (`transmux::Error` is one now), must drop the bound.
- New public surface that goes with the above: `MediaSegment::title`, `MediaSegment::pre_tags`, `OpenSegment::pre_tags`, and the `Error` variants `InvalidDecimalSeconds`, `InvalidSignedDecimalSeconds`, `InvalidQuotedString`, `InvalidUri`.

## Behaviour changes (output and parsing)

- **Segment-defining tags re-render in place (BH-W5).** `#EXT-X-KEY`, `#EXT-X-PROGRAM-DATE-TIME`, `#EXT-X-DATERANGE` and the `#EXT-X-CUE-*` family have no typed field, so `parse()` kept them verbatim in `MediaPlaylist::extra_tags`, which `to_m3u8` emitted as one block before all segments. Per RFC 8216bis §4.4.1 such a tag applies until the next occurrence, so position matters: an `AES-128` key A before segment 1 and key B before segment 50 re-rendered with both at the top, applying key B to every segment, and every PDT landed at the top so the last one won. These tags now attach to the following segment (`MediaSegment::pre_tags`; the open live-edge segment keeps its own in `OpenSegment::pre_tags`) and re-emit immediately before it, so key rotations, per-segment PDT and mid-playlist DATERANGE survive parse and render byte for byte. Playlist-level unmodeled tags (`#EXT-X-MEDIA`, `#EXT-X-DEFINE`, ...) still go to `extra_tags`. If you pushed such tags into `extra_tags` expecting them at the top, move them to the segment they apply to.
- **`#EXTINF` titles round-trip (BH-W11).** `MediaSegment::title: Option<String>`; `None` renders the bare `#EXTINF:<dur>,` exactly as before. A title is validated like any quoted-string.
- **`cenc_ext_x_key` emits `METHOD=SAMPLE-AES-CTR` for the `cenc` scheme (BH-W1, #1111)** where it used to return `None` on a claim that CTR is not a valid HLS METHOD, contradicting `EncryptionMethod::SampleAesCtr` and RFC 8216bis §4.4.4.4. No `IV` is emitted for it, as the spec requires. `CencScheme::Cbcs` is unchanged (`METHOD=SAMPLE-AES`).
- **`#EXT-X-RENDITION-REPORT` requires `LAST-MSN` (BH-W2).** An absent attribute is `Error::HlsParse`; it used to default to `0` and re-render a fabricated `LAST-MSN=0`.
- **Strict numeric grammar everywhere (BH-W4, #1140).** All integer attributes (`EXT-X-VERSION`, `EXT-X-TARGETDURATION`, `EXT-X-MEDIA-SEQUENCE`, `EXT-X-DISCONTINUITY-SEQUENCE`, `EXT-X-BITRATE`, `BANDWIDTH`, `RESOLUTION`, the `BYTERANGE` family, `LAST-MSN`, `LAST-PART`, `SKIPPED-SEGMENTS`) share one strict `decimal-integer` lexer: no leading `+` (`BANDWIDTH=+7` parsed before), no exponent, no `nan`/`inf`, no decimal point. `EXTINF`, `PART-TARGET`, `PART-HOLD-BACK`, `CAN-SKIP-UNTIL`, `HOLD-BACK`, `EXT-X-PART DURATION` and `TIME-OFFSET` parse the RFC 8216bis §4.2 decimal-floating-point grammar, rejecting `nan`, `inf`, exponent notation, a leading `+` and an out-of-place `-`. `#EXTINF:nan,` used to parse into a NaN that later panicked `Duration::from_secs_f64`. `format_secs` and `format_extinf` clamp a non-finite or negative value to `"0"` instead of emitting `NaN`/`inf`.
- **A Multivariant Playlist whose `#EXT-X-STREAM-INF` has no following URI line is an error (BH-W9).** A second `STREAM-INF` before the URI used to overwrite the pending variant, and EOF with one pending dropped it, silently losing a variant of a truncated playlist. Both are `Error::HlsParse`.

## Documentation corrected

`MasterPlaylist::extra_tags` was documented as emitted "after the variant entries"; `to_m3u8` emits them first, before `#EXT-X-INDEPENDENT-SEGMENTS`/`DEFINE`/`START`, the `SESSION-*` tags and the variant list (BH-W10). The behaviour is unchanged; a caller who followed the old wording to place a carried `#EXT-X-DEFINE` after the variants that use it was already getting the opposite.

## Migration checklist

1. Add `?` (or handle `Err`) at every `to_m3u8` and `cenc_ext_x_key` call.
2. Wrap duration literals with `DecimalSeconds::new` / `SignedDecimalSeconds::new` and read with `.get()`.
3. Replace `LowLatencyConfig::extra_attrs` with the per-tag lists; wrap `part_target`/`part_hold_back` in `Some(..)`.
4. Replace string pairs in `extra_attrs` with `AttrValue::quoted`/`bare`/`for_attr`.
5. Set `Variant::codecs` to `Some(..)` or `None`; handle the `cenc` scheme now returning a tag.
6. Re-check any code that placed `EXT-X-KEY`, `EXT-X-PROGRAM-DATE-TIME` or `EXT-X-DATERANGE` in `extra_tags`.

---

Published from tag `broadcast-hls-v0.3.0`.
