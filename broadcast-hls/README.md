# broadcast-hls

[![Crates.io](https://img.shields.io/crates/v/broadcast-hls.svg)](https://crates.io/crates/broadcast-hls)
[![docs.rs](https://img.shields.io/docsrs/broadcast-hls)](https://docs.rs/broadcast-hls)

HLS (M3U8) playlist syntax, RFC 8216 and RFC 8216bis (Low-Latency HLS): typed Media and Multivariant (Master) Playlist parse and serialize, `no_std`.

```toml
[dependencies]
broadcast-hls = "0.9"
```

## What it does / what it does not

**Does:** parse a playlist into `MediaPlaylist` or `MasterPlaylist` structs and render it back with `to_m3u8()`, covering every one of the 32 tags in RFC 8216bis §4.4 (28 as typed struct fields, 4 preserved verbatim), the Low-Latency additions (partial segments, blocking reload, delta updates, preload hints, rendition reports), I-frame trick-play, discontinuity signalling, and the `EXT-X-KEY` line for CENC/CBCS protected CMAF. `EXT-X-VERSION` is computed from the content, never trusted from a caller. It was extracted from `transmux` (issue #878) so a crate that only needs playlist syntax (`media-doctor`, an HLS-pull client, a fuzz target) does not pull in a container-muxing hub.

**Does not:** fetch anything or run a client (that is `hls-runtime`), produce segments (`transmux`'s `ts_hls` and `ll_hls` segmenters stay there because they emit container bytes), validate cross-file semantic rules (`media-doctor`), or model `EXT-X-KEY` as a typed playlist field (it round-trips losslessly as a tag line; read the key in effect with `MediaPlaylist::segment_keys`). `EXT-X-MEDIA` is typed (`MasterPlaylist::media`). `EXT-X-DATERANGE` stays a verbatim segment tag line in `MediaSegment::pre_tags` when parsing, and `DateRange` (`parse_tag_line`/`to_tag_line`) gives typed access to it and builds new ones, including HLS Interstitial (`CLASS="com.apple.hls.interstitial"`) ranges. `ssai-runtime` assembles the interstitial playlists; the interstitial spec (`private/specs-verified/hls/ietf/rfc8216bis-interstitials.md`) is transcribed but only the generic DATERANGE attributes and SCTE-35 cues are typed here.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| IETF HLS 2nd edition, `draft-pantos-hls-rfc8216bis` | `-22`, 1 May 2026 (private repo: `private/specs-verified/ietf/specs/ietf_draft_pantos_hls_rfc8216bis.txt`) | §4.4 tags (all 32 recognised), §8 protocol version derivation, §9 examples (fixtures), Low-Latency directives | Partial | 28 of 32 tags typed; cross-tag and cross-file MUSTs are not enforced; a draft, not a ratified RFC |
| IETF RFC 8216 HLS | RFC 8216 (private repo: `private/specs-verified/ietf/specs/ietf_rfc8216_hls.txt`) | §4.2 attribute-list grammar and quoted strings, §4.3 tags including I-frame variants | Full for the syntax | Superseded in intent by the draft above |
| Apple HLS timed metadata | private repo: `private/specs-verified/hls/apple/apple_hls_timed_metadata.html` | `EXT-X-DATERANGE`: typed `DateRange` (ID/CLASS/START-DATE/END-DATE/DURATION/PLANNED-DURATION/END-ON-NEXT, `X-` client attributes, SCTE-35 `Cue`); a parsed playlist keeps the line verbatim | Partial | `timed-metadata` and `ssai-runtime` build ranges through `DateRange` |
| ISO/IEC 23001-7 Common Encryption | Edition not in the vendored index | `cenc_ext_x_key`: `cbcs` becomes `METHOD=SAMPLE-AES`, `cenc` becomes `METHOD=SAMPLE-AES-CTR` with no `IV` (draft §4.4.4.4) | Partial | Scheme identity is `broadcast_core::cenc::CencScheme`, shared with `transmux` |

## Features

| Feature | Default | Description |
|---|---|---|
| `std` | yes | Links the standard library. Without it the crate is `#![no_std]` and needs only `alloc` (it builds for `thumbv7em-none-eabi`). |
| `serde` | no | `serde::Serialize` on the public types. |

## Quick start

Parse a real Apple BipBop media playlist (a committed fixture) and render it back:

```rust
use broadcast_hls::MediaPlaylist;

let text = std::fs::read_to_string("../fixtures/hls/real/bipbop-ts/gear1-video.m3u8").unwrap();
let playlist = MediaPlaylist::parse(&text).unwrap();
assert!(!playlist.segments.is_empty());

// to_m3u8 is the symmetric inverse of parse: a second parse equals the first.
let rendered = playlist.to_m3u8().unwrap();
assert_eq!(MediaPlaylist::parse(&rendered).unwrap(), playlist);
```

Build a playlist from fields (`DecimalSeconds` rejects NaN and infinity, `to_m3u8` is fallible):

```rust
use broadcast_hls::{DecimalSeconds, MediaPlaylist, MediaSegment};

let playlist = MediaPlaylist {
    target_duration: 10,
    segments: vec![MediaSegment {
        uri: "seg0.m4s".into(),
        duration: DecimalSeconds::new(9.009).unwrap(),
        ..Default::default()
    }],
    endlist: true,
    ..Default::default()
};
let m3u8 = playlist.to_m3u8().unwrap();
assert!(m3u8.starts_with("#EXTM3U"));
// The renderer computed `#EXT-X-VERSION` from the content; parsing reads it back.
let reparsed = MediaPlaylist::parse(&m3u8).unwrap();
assert_eq!(reparsed.segments, playlist.segments);
assert_eq!(reparsed.to_m3u8().unwrap(), m3u8);
```

Runnable programs: `cargo run -p broadcast-hls --example build_playlist` and `--example parse_playlist`.

## What is implemented

- **`MediaPlaylist` and `MediaSegment`:** segments with `title`, `map` (carry-forward on parse, deduplicated on render), `bitrate`, `gap`, `discontinuous`, byte ranges, and `pre_tags` (segment-defining tags kept in place), plus playlist-level `extra_tags` for unrecognised tags.
- **`MasterPlaylist`, `Variant`, `IFrameVariant`:** multivariant playlists with `EXT-X-I-FRAME-STREAM-INF` trick-play (RFC 8216 §4.3.4.3) and `MasterPlaylist::extra_tags`.
- **The 28 typed tags:** including `EXT-X-INDEPENDENT-SEGMENTS`, `EXT-X-START` (`StartPoint`), `EXT-X-DEFINE` (`Define`), `EXT-X-PLAYLIST-TYPE` (`PlaylistType`), `EXT-X-GAP`, `EXT-X-BITRATE`, `EXT-X-SESSION-DATA` (`SessionData`), `EXT-X-SESSION-KEY` (`SessionKey`, `EncryptionMethod`) and `EXT-X-CONTENT-STEERING` (`ContentSteering`). `tests/it/hls_tag_completeness.rs` parses a fixture carrying all 32 and asserts typed tags populate their field and the four opaque ones survive verbatim.
- **Typed `EXT-X-DATERANGE`:** `DateRange` (`parse_tag_line`, `to_tag_line`, validating setters and `with_*` builders), `Cue`/`CueTrigger` and `Scte35Kind` for the SCTE-35 attributes, `ClientAttrName` for `X-` attributes. Values keep their exact wire spelling, so a parsed tag re-renders byte-for-byte.
- **Vendor tags:** `CustomTagName` (validated `EXT-` name that may not shadow a standard tag) and `CustomTag` (attribute list via `parse_tag_line`/`to_tag_line`). `ParseOptions` with `MediaPlaylist::parse_with_options` names vendor tags that apply to the following segment, so they stay in that segment's `pre_tags` position instead of `extra_tags`.
- **Append-only writers:** `MediaPlaylist::render_header`, `render_segment(index)` and `render_endlist` emit a playlist piecewise; pin `version` so the header stays stable as segments are appended.
- **Truncated input:** `ParseDiagnosticKind::DanglingSegmentTags` reports an `#EXTINF`/`#EXT-X-BYTERANGE` with no closing URI line (a playlist caught mid-write). The open segment's `OpenSegment::trailing_tags` (`OpenTag`) keep a key rotation between parts in its original position.
- **Low-Latency HLS** (opt-in, so a plain playlist is unchanged): `LowLatencyConfig`, `PartSpec`, `OpenSegment` (the in-progress segment at the live edge), `MapTag`, `ByteRange`, `PreloadHintType`, `RenditionReport`, `SkipInfo`. `PART-HOLD-BACK` must be at least three part targets.
- **Discontinuities:** `EXT-X-DISCONTINUITY` and `EXT-X-DISCONTINUITY-SEQUENCE`, and `mark_init_discontinuities` (detect an init-segment change across a run).
- **Version derivation:** `MediaPlaylist::computed_version` and `MasterPlaylist::computed_version` apply the draft's §8 table (`private/specs-verified/hls/ietf/rfc8216bis-version-compatibility.md`). A playlist needing nothing gets no `EXT-X-VERSION` tag. The `version` field is a floor: raised to the computed value, never lowered.
- **Attribute lists:** `parse_attribute_list`, `render_attribute_list`, `AttrValue` (validated quoted or bare value), and unknown attributes on every modelled tag kept in `extra_attrs` and re-emitted after the known ones.
- **Durations:** `DecimalSeconds` and `SignedDecimalSeconds` round-trip bit-exactly (including `9.9766` and `2.00004`). Round your own durations before building if you want compact playlists, since an unrounded division renders every digit.
- **CENC:** `cenc_ext_x_key`, `CENC_KEYFORMAT`, `CENC_KEYFORMATVERSIONS`.
- **Segment keys (RFC 8216 §4.3.2.4):** `MediaPlaylist::segment_keys` resolves the `#EXT-X-KEY` in effect for each closed segment (carry-forward until the next key of the same `KEYFORMAT`, `METHOD=NONE` resets, only the `identity` format participates) as `MediaKey` (an alias of `SessionKey`), and `open_segment_key` does the same for the open segment. A key with no `URI` is `Error::KeyWithoutUri`; a segment covered only by a non-identity `KEYFORMAT` is `Error::UnsupportedKeyFormat`.
- **Codec and range signalling:** `SUPPLEMENTAL-CODECS` (round-trips, including the HDR10+ `/cdm4` brand) and the typed `VideoRange` (`SDR`, `HLG`, `PQ`), strict attribute-list parsing (`parse_attribute_list_strict`) that reports a malformed list instead of half-parsing it.
- **Master groups:** `MasterPlaylist::dangling_group_references()` lists the `AUDIO`/`VIDEO`/`SUBTITLES`/`CLOSED-CAPTIONS` groups an `#EXT-X-STREAM-INF` names that no `#EXT-X-MEDIA` of the same type declares; `to_m3u8` no longer rejects such a playlist and keeps the reference, so a non-conformant third-party playlist round-trips byte-identical.

## Round-trip fidelity

The workspace text-format rule is that a second parse of the rendered output equals the first parse, not that the text is byte-identical. What changes on re-emit:

- Tag order is canonical, not input order (for example `INDEPENDENT-SEGMENTS`, `DEFINE`, `START` follow the version line; `SESSION-KEY`, `SESSION-DATA`, `CONTENT-STEERING` precede the variants). No tag is lost.
- Blank lines, trailing whitespace and `#` comment lines are dropped. Line-continuation backslashes (used for wrapping in spec examples) are never emitted.
- `to_m3u8()` is fallible: a quoted-string or URI field containing `"`, CR or LF (`Error::InvalidQuotedString`, `Error::InvalidUri`) is rejected instead of rendered, which prevents tag-line injection. The same applies to an `EXT-X-SKIP` removed-daterange id.
- Not enforced: `EXT-X-BITRATE` not applying to a byte-ranged segment, `EXT-X-DEFINE` `IMPORT` and `QUERYPARAM` resolution, `EXT-X-SESSION-KEY` `METHOD` not being `NONE`, "must not appear more than once" rules. Those need cross-file context and belong to `media-doctor`.
- A non-segment-defining unrecognised tag lands in `extra_tags` and loses its interleaved position (it renders as one block before the segments). Segment-defining tags (`EXT-X-KEY`, `EXT-X-DATERANGE`, `EXT-X-CUE-*`) attach to the following segment's `pre_tags` and keep their place. `#EXT-X-PROGRAM-DATE-TIME` is now a typed field (`MediaSegment::program_date_time`) rather than an opaque pre-tag.
- A hand-built playlist whose segments' `map` fields do not form a valid carry-forward sequence cannot round-trip, because the wire has no "stop applying the map" short of a discontinuity plus a new map.

## Design notes

Zero dependencies beyond `broadcast-common` and `thiserror`, `no_std` + `alloc`, symmetric parse and render, `#[non_exhaustive]` on `Error` and the open enums, label-coverage tests for spec enums, fixture corpus (`tests/it/hls_fixture_corpus.rs`) over Apple's BipBop playlists and every RFC 8216bis §9 example (provenance in `fixtures/hls/PROVENANCE.md`), and a fuzz target.

## Related crates

Depends on `broadcast-common`. Consumed by `hls-runtime` (client parses, origin renders), `transmux` (the HLS segmenters), `ssai-runtime` (interstitial playlists), `media-doctor` (conformance checks), `timed-metadata` (daterange lines) and `multimux`.

## MSRV and licence

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
