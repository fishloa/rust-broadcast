# Changelog

All notable changes to `broadcast-hls` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - Unreleased

### Fixed
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
