# broadcast-hls

[![Crates.io](https://img.shields.io/crates/v/broadcast-hls.svg)](https://crates.io/crates/broadcast-hls)
[![docs.rs](https://img.shields.io/docsrs/broadcast-hls)](https://docs.rs/broadcast-hls)

HLS (M3U8) playlist syntax, RFC 8216 and RFC 8216bis (Low-Latency HLS): typed Media and Multivariant (Master) Playlist parse and serialize, `no_std`.

```toml
[dependencies]
broadcast-hls = "0.4"
```

## What it does / what it does not

**Does:** parse a playlist into `MediaPlaylist` or `MasterPlaylist` structs and render it back with `to_m3u8()`, covering every one of the 32 tags in RFC 8216bis §4.4 (28 as typed struct fields, 4 preserved verbatim), the Low-Latency additions (partial segments, blocking reload, delta updates, preload hints, rendition reports), I-frame trick-play, discontinuity signalling, and the `EXT-X-KEY` line for CENC/CBCS protected CMAF. `EXT-X-VERSION` is computed from the content, never trusted from a caller. It was extracted from `transmux` (issue #878) so a crate that only needs playlist syntax (`media-doctor`, an HLS-pull client, a fuzz target) does not pull in a container-muxing hub.

**Does not:** fetch anything or run a client (that is `hls-runtime`), produce segments (`transmux`'s `ts_hls` and `ll_hls` segmenters stay there because they emit container bytes), validate cross-file semantic rules (`media-doctor`), or give typed access to `EXT-X-KEY`, `EXT-X-PROGRAM-DATE-TIME` and `EXT-X-DATERANGE` (they round-trip losslessly as tag lines, but you cannot read or build them through a field). `EXT-X-MEDIA` is now typed (`MasterPlaylist::media`). HLS Interstitials (`CLASS="com.apple.hls.interstitial"` date ranges) are therefore built by the caller (`ssai-runtime` renders them) as `EXT-X-DATERANGE` lines, and the interstitial spec (`docs/interstitials.md`) is transcribed but not typed here.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| IETF HLS 2nd edition, `draft-pantos-hls-rfc8216bis` | `-22`, 1 May 2026 (vendored `specs/ietf_draft_pantos_hls_rfc8216bis.txt`); transcribed in `docs/` | §4.4 tags (all 32 recognised), §8 protocol version derivation, §9 examples (fixtures), Low-Latency directives | Partial | 28 of 32 tags typed; cross-tag and cross-file MUSTs are not enforced; a draft, not a ratified RFC |
| IETF RFC 8216 HLS | RFC 8216 (vendored `specs/ietf_rfc8216_hls.txt`) | §4.2 attribute-list grammar and quoted strings, §4.3 tags including I-frame variants | Full for the syntax | Superseded in intent by the draft above |
| Apple HLS timed metadata | Vendored `specs/apple_hls_timed_metadata.html` | `EXT-X-DATERANGE` carried as opaque tag lines | Typed-opaque | `timed-metadata` builds the lines |
| ISO/IEC 23001-7 Common Encryption | Edition not in the vendored index | `cenc_ext_x_key`: `cbcs` becomes `METHOD=SAMPLE-AES`, `cenc` becomes `METHOD=SAMPLE-AES-CTR` with no `IV` (draft §4.4.4.4) | Partial | Scheme identity is `broadcast_common::cenc::CencScheme`, shared with `transmux` |

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
- **`MasterPlaylist`, `Variant`, `IFrameVariant`:** multivariant playlists with `EXT-X-I-FRAME-STREAM-INF` trick-play (RFC 8216 §4.3.4.2) and `MasterPlaylist::extra_tags`.
- **The 28 typed tags:** including `EXT-X-INDEPENDENT-SEGMENTS`, `EXT-X-START` (`StartPoint`), `EXT-X-DEFINE` (`Define`), `EXT-X-PLAYLIST-TYPE` (`PlaylistType`), `EXT-X-GAP`, `EXT-X-BITRATE`, `EXT-X-SESSION-DATA` (`SessionData`), `EXT-X-SESSION-KEY` (`SessionKey`, `EncryptionMethod`) and `EXT-X-CONTENT-STEERING` (`ContentSteering`). `tests/hls_tag_completeness.rs` parses a fixture carrying all 32 and asserts typed tags populate their field and the four opaque ones survive verbatim.
- **Low-Latency HLS** (opt-in, so a plain playlist is unchanged): `LowLatencyConfig`, `PartSpec`, `OpenSegment` (the in-progress segment at the live edge), `MapTag`, `ByteRange`, `PreloadHintType`, `RenditionReport`, `SkipInfo`. `PART-HOLD-BACK` must be at least three part targets.
- **Discontinuities:** `EXT-X-DISCONTINUITY` and `EXT-X-DISCONTINUITY-SEQUENCE`, and `mark_init_discontinuities` (detect an init-segment change across a run).
- **Version derivation:** `MediaPlaylist::computed_version` and `MasterPlaylist::computed_version` apply the draft's §8 table (`docs/version-compatibility.md`). A playlist needing nothing gets no `EXT-X-VERSION` tag. The `version` field is a floor: raised to the computed value, never lowered.
- **Attribute lists:** `parse_attribute_list`, `render_attribute_list`, `AttrValue` (validated quoted or bare value), and unknown attributes on every modelled tag kept in `extra_attrs` and re-emitted after the known ones.
- **Durations:** `DecimalSeconds` and `SignedDecimalSeconds` round-trip bit-exactly (including `9.9766` and `2.00004`). Round your own durations before building if you want compact playlists, since an unrounded division renders every digit.
- **CENC:** `cenc_ext_x_key`, `CENC_KEYFORMAT`, `CENC_KEYFORMATVERSIONS`.

## Round-trip fidelity

The workspace text-format rule is that a second parse of the rendered output equals the first parse, not that the text is byte-identical. What changes on re-emit:

- Tag order is canonical, not input order (for example `INDEPENDENT-SEGMENTS`, `DEFINE`, `START` follow the version line; `SESSION-KEY`, `SESSION-DATA`, `CONTENT-STEERING` precede the variants). No tag is lost.
- Blank lines, trailing whitespace and `#` comment lines are dropped. Line-continuation backslashes (used for wrapping in spec examples) are never emitted.
- `to_m3u8()` is fallible: a quoted-string or URI field containing `"`, CR or LF (`Error::InvalidQuotedString`, `Error::InvalidUri`) is rejected instead of rendered, which prevents tag-line injection. The same applies to an `EXT-X-SKIP` removed-daterange id.
- Not enforced: `EXT-X-BITRATE` not applying to a byte-ranged segment, `EXT-X-DEFINE` `IMPORT` and `QUERYPARAM` resolution, `EXT-X-SESSION-KEY` `METHOD` not being `NONE`, "must not appear more than once" rules. Those need cross-file context and belong to `media-doctor`.
- A non-segment-defining unrecognised tag lands in `extra_tags` and loses its interleaved position (it renders as one block before the segments). Segment-defining tags (`EXT-X-KEY`, `EXT-X-DATERANGE`, `EXT-X-CUE-*`) attach to the following segment's `pre_tags` and keep their place. `#EXT-X-PROGRAM-DATE-TIME` is now a typed field (`MediaSegment::program_date_time`) rather than an opaque pre-tag.
- A hand-built playlist whose segments' `map` fields do not form a valid carry-forward sequence cannot round-trip, because the wire has no "stop applying the map" short of a discontinuity plus a new map.

## Design notes

Zero dependencies beyond `broadcast-common` and `thiserror`, `no_std` + `alloc`, symmetric parse and render, `#[non_exhaustive]` on `Error` and the open enums, label-coverage tests for spec enums, fixture corpus (`tests/hls_fixture_corpus.rs`) over Apple's BipBop playlists and every RFC 8216bis §9 example (provenance in `fixtures/hls/MANIFEST.md`), and a fuzz target.

## Related crates

Depends on `broadcast-common`. Consumed by `hls-runtime` (client parses, origin renders), `transmux` (the HLS segmenters), `ssai-runtime` (interstitial playlists), `media-doctor` (conformance checks), `timed-metadata` (daterange lines) and `multimux`.

## MSRV and licence

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
