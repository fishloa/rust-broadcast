# Changelog

All notable changes to `broadcast-hls` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - Unreleased

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
