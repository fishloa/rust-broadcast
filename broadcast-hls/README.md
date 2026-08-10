# broadcast-hls

[![Crates.io](https://img.shields.io/crates/v/broadcast-hls.svg)](https://crates.io/crates/broadcast-hls)
[![docs.rs](https://img.shields.io/docsrs/broadcast-hls)](https://docs.rs/broadcast-hls)

> **Extracted from `transmux`** (issue #878). `transmux/src/hls.rs` — the M3U8
> playlist syntax — moved here so a crate that only needs to parse/render HLS
> playlists (e.g. `media-doctor`, an HLS-pull client, a fuzz target) no longer
> has to pull in an entire any-to-any container muxing hub to parse a text
> file. `transmux`'s own HLS/LL-HLS **segmenters** (`ts_hls`, `ll_hls` — they
> produce container bytes, not playlist syntax) stayed put.

## Install

```toml
[dependencies]
broadcast-hls = "0.1"
```

HLS (M3U8) playlist syntax — RFC 8216 / RFC 8216bis (Low-Latency HLS): typed
Media/Master Playlist parse + serialize.

Implements:

- **`MediaPlaylist`** / **`MasterPlaylist`** — `#EXTM3U` Media and
  Multivariant (Master) Playlists; `to_m3u8()` renders, `parse()` is its
  symmetric inverse.
- **`MediaSegment`** / **`Variant`** / **`IFrameVariant`** — per-segment and
  per-variant entries, including RFC 8216 §4.3.4.2 I-frame-only trick-play
  signalling.
- **Low-Latency HLS (RFC 8216bis)** — `LowLatencyConfig`, `PartSpec`,
  `OpenSegment`, `MapTag`, `ByteRange`, `PreloadHintType`,
  `RenditionReport`, `SkipInfo` — the partial-segment / blocking-reload /
  playlist-delta-update directives, all strictly opt-in so a plain playlist
  is byte-for-byte unchanged.
- **`#EXT-X-DISCONTINUITY[-SEQUENCE]`** signalling, including
  `mark_init_discontinuities` (auto-detect an init-segment change across a
  segment run).
- **CENC/CBCS DRM signalling** (ISO/IEC 23001-7, issue #564) —
  `cenc_ext_x_key` renders the `#EXT-X-KEY` tag for a `cbcs`-protected CMAF
  track (`cenc`/AES-CTR has no valid HLS `METHOD`, so it returns `None`).
- **28 of the 32 RFC 8216bis §4.4 tags have a typed struct field** (issue
  #872) — including `#EXT-X-INDEPENDENT-SEGMENTS`, `#EXT-X-START`
  (`StartPoint`), `#EXT-X-DEFINE` (`Define`), `#EXT-X-PLAYLIST-TYPE`
  (`PlaylistType`: `VOD`/`EVENT`), `#EXT-X-GAP`, `#EXT-X-BITRATE`,
  `#EXT-X-SESSION-DATA` (`SessionData`), `#EXT-X-SESSION-KEY`
  (`SessionKey`, `EncryptionMethod`) and `#EXT-X-CONTENT-STEERING`
  (`ContentSteering`) parse *into* that field and serialize *from* it. The
  remaining **4** — `#EXT-X-KEY`, `#EXT-X-PROGRAM-DATE-TIME`,
  `#EXT-X-DATERANGE`, `#EXT-X-MEDIA` — have no typed field; `parse()`
  recognizes them (no error) and preserves the tag line verbatim in
  `extra_tags`, so they round-trip losslessly, but a caller cannot read or
  build one through a struct field (see "Round-trip fidelity" below and the
  module doc's "Known, documented gaps" list in `src/lib.rs`). Both claims
  are enforced *behaviorally* by `tests/hls_tag_completeness.rs`: it parses
  a fixture carrying every one of the 32 tags and asserts the typed ones
  populate their field and the 4 opaque ones survive verbatim — not merely
  that each tag's name appears somewhere in `src/`.

`#![no_std]` + `alloc`; depends only on `broadcast-common`. Builds for
`thumbv7em-none-eabi`.

## Round-trip fidelity

Per this workspace's text-format rule (`docs/CRATE-ACCEPTANCE.md` §1), a
playlist is not required to be byte-identical after a parse → serialize
round trip, only for a *second* parse of the rendered output to equal the
first parse. Known ways rendered output can differ from arbitrary input
text:

- **Unmodeled tags are preserved, not dropped, in a `MasterPlaylist`.**
  `#EXT-X-MEDIA` (alternate audio/video/subtitle renditions) has no
  corresponding typed field but is preserved verbatim into
  `MasterPlaylist::extra_tags`, the Multivariant-Playlist counterpart of
  `MediaPlaylist::extra_tags`. A fixture containing `#EXT-X-MEDIA` therefore
  round-trips under this crate's invariant (the *parsed struct* is stable
  across a second parse); the rendered text differs in tag ordering (see
  below) but no tag is silently lost.
- **Tag ordering is canonical on re-emit, not input-preserving.** `to_m3u8()`
  always emits tags in a fixed order relative to the variant list:
  `extra_tags` first, then `INDEPENDENT-SEGMENTS` / `DEFINE` / `START`,
  then `SESSION-KEY` / `SESSION-DATA` / `CONTENT-STEERING`, then the
  variant and I-frame-variant entries — regardless of where those tags
  appeared in the source text. The ordering divergence is a formatting
  difference, not data loss: every parsed tag (typed or verbatim) survives
  the round trip; it is just re-sequenced on output.

- **Unknown attributes on modeled tags are retained and re-emitted** (issue
  #884). Every tag struct that carries an attribute list (
  `ContentSteering`, `Variant`, `IFrameVariant`, `MapTag`, `PartSpec`,
  `RenditionReport`, `SkipInfo`, `SessionData`, `SessionKey`, `StartPoint`,
  `Define`, `LowLatencyConfig`) now holds an `extra_attrs:
  Vec<(String, String)>` for attribute names this crate does not model.
  These survive parse → serialize and feed the §8 row 12 `REQ-` check.
  They are always emitted *after* each tag's known attributes, so a tag
  with unknown attrs will have them appended in sorted-by-name order.
- **Tag ordering is canonical, not preserved.** `to_m3u8()` always emits
  tags in a fixed order (e.g. `#EXT-X-INDEPENDENT-SEGMENTS` /
  `#EXT-X-DEFINE` / `#EXT-X-START` right after `#EXT-X-VERSION`;
  `#EXT-X-SESSION-KEY` / `#EXT-X-SESSION-DATA` / `#EXT-X-CONTENT-STEERING`
  before the variant list), regardless of where those tags appeared in the
  source text.
- **Line-continuation backslashes are never round-tripped.** Some RFC
  8216bis §9 examples use a trailing `\` to wrap a long attribute list
  across lines for readability in the spec text itself (not literal m3u8
  syntax — see `docs/examples.md`); this crate's fixtures reflow those onto
  single lines before committing them, and `to_m3u8()` never emits a
  continuation of its own.
- **`#EXT-X-BITRATE`'s "does not apply to a byte-ranged segment" rule is not
  enforced.** The tag is carried forward and dedup-rendered exactly like
  `#EXT-X-MAP`; a segment carrying its own `#EXT-X-BYTERANGE` still renders
  (and round-trips) a carried-forward bitrate value, though the spec says
  the tag does not semantically apply to it.
- **Cross-tag/cross-file MUST-constraints are not enforced** — e.g.
  `#EXT-X-DEFINE`'s `IMPORT`/`QUERYPARAM` resolution against a parent
  Multivariant Playlist or request URI, `#EXT-X-SESSION-KEY`'s "`METHOD`
  MUST NOT be `NONE`", or any "MUST NOT appear more than once" rule. Only
  each tag's own attribute grammar is validated; broader semantic checks are
  left to a higher-level tool (e.g. `media-doctor`).
- **`#EXT-X-VERSION` is *computed*, and is often omitted rather than
  always emitted** (issue #871). `to_m3u8()` never renders a version you
  handed it verbatim: it derives the minimum version the playlist's actual
  content requires per RFC 8216bis §8 (`MediaPlaylist::computed_version`/
  `MasterPlaylist::computed_version`, transcribed in
  `docs/version-compatibility.md`), and renders exactly that. A playlist
  that triggers no §8 rule (fully compatible with version 1) gets **no**
  `#EXT-X-VERSION` tag at all — including one parsed from input that had no
  version tag either, so that case round-trips with the tag still absent,
  not gaining one.
  [`MediaPlaylist::version`]/[`MasterPlaylist::version`] is a *floor*, not
  the rendered value: `0` (the field's `Default`, and what a version-less
  input parses to) means "no explicit floor — render exactly the computed
  value, or nothing"; a nonzero value is raised, never lowered, to the
  computed minimum, so you can deliberately over-declare but can never
  silently under-declare an invalid playlist. In short: a caller can rely
  on the rendered `#EXT-X-VERSION` (when present) being *at least* what the
  content requires — it is not the caller's job to compute that minimum.
- **Whitespace and comments are not preserved.** Blank lines, trailing
  whitespace (real playlists do carry it — Apple's BipBop `#EXTINF` lines
  end in a tab), and non-`#EXT` `#` comment lines are dropped at parse time
  and never re-emitted.

Durations are **not** in this list: `#EXTINF` and every seconds-valued
attribute round-trip bit-exactly, including sub-millisecond values such as
Apple's `9.9766` and RFC 8216bis §9.11's `2.00004`. (They were lossy until
issue #872 — see `fixtures/hls/MANIFEST.md`, Tier 2 "Finding".) Because
that rendering is faithful to the exact `f64` you supply, a duration that
came from an unrounded division (e.g. `11.0 / 30.0`) renders with all the
digits needed to reproduce it — `0.36666666666666664`. That is correct and
round-trips, but if you want compact playlists, **round your own durations
before building the playlist**; this crate will not round them for you,
because it cannot tell a deliberate `2.00004` from an artefact.

## Quick start

```rust
use broadcast_hls::{MediaPlaylist, MediaSegment};

let playlist = MediaPlaylist {
    version: 3,
    target_duration: 10,
    segments: vec![MediaSegment {
        uri: "seg0.m4s".into(),
        duration: 9.009,
        ..Default::default()
    }],
    endlist: true,
    ..Default::default()
};
let m3u8 = playlist.to_m3u8();
assert_eq!(MediaPlaylist::parse(&m3u8).unwrap(), playlist);
```

## Examples

```sh
cargo run -p broadcast-hls --example build_playlist
cargo run -p broadcast-hls --example parse_playlist
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | no      | `serde::Serialize` derives on public types. |

## Minimum Supported Rust Version

1.95.0

## License

MIT OR Apache-2.0
