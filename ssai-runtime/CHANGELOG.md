# Changelog — ssai-runtime

All notable changes to this crate. Format: [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]
### Changed
- BREAKING: `SessionStore::advance` takes a `broadcast_common::stage::Timestamp` (the caller's own monotonic clock, as every sans-IO engine takes) instead of `now_us: u64`; `expire_idle`'s TTL stays in microseconds.

## [0.6.0] - 2026-11-14
### Added
- `SessionStore` is bounded: `DEFAULT_MAX_SESSIONS` (65 536) by default, `with_max_sessions` to change it, `Error::SessionStoreFull` when `begin_break` is called for a new session at the bound. `advance(now_us)` feeds the store a clock and `expire_idle(ttl_us)` evicts sessions untouched for longer than the TTL. `Error::BreakAlreadyResumed` (`Error` is `#[non_exhaustive]`).

### Fixed
- `SessionStore::resume` on an already-resumed break returns `Error::BreakAlreadyResumed` and keeps the first resumption point; it overwrote it silently.
- BREAKING: `InterstitialDateRange::parse_tag_line` keeps an unknown `X-SNAP`/`X-RESTRICT` identifier verbatim in the new `SnapMode::Other` / `RestrictMode::Other` (it dropped it, so a parse then render changed the tag); `token()` returns the wire token. Both enums lose `Copy` (they carry a `String` now). It also rejects a quoted `DURATION`/`X-RESUME-OFFSET`/`X-PLAYOUT-LIMIT` or an unquoted `ID`/`START-DATE`/`X-ASSET-URI`/`X-ASSET-LIST`/`X-SNAP`/`X-RESTRICT`, as typed in `private/specs-verified/hls/ietf/rfc8216bis-interstitials.md` (Appendix D attribute table) and RFC 8216bis section 4.2.
- `X-RESUME-OFFSET` is a signed-decimal-floating-point (RFC 8216bis, `private/specs-verified/hls/ietf/rfc8216bis-interstitials.md`): a negative value now renders and parses instead of failing with `Error::InvalidDuration`. `DURATION` and `X-PLAYOUT-LIMIT` stay non-negative.
- A `DURATION` of exactly 2^63 seconds no longer renders as `i64::MAX`; the integer shortcut is used only below 2^53.
- `X-SNAP` and `X-RESTRICT` tokens are trimmed and empty tokens are dropped when parsing (`X-SNAP=" OUT , IN "` is `[Out, In]`, `X-SNAP="OUT,"` is `[Out]`); unknown tokens are still kept as `Other`."

### Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `broadcast-hls` 0.6 -> 0.7.

## [0.5.0] - 2026-11-07
### Changed
- Error enums derive `Clone`, `PartialEq` and `Eq` wherever every field allows it (c6 C6-14).
- The crate-root docs are the README (`#![doc = include_str!("../README.md")]`) instead of a hand-synced `//!` copy (c3 DOC1).
- `AdBreakDecision::asset_list` and `SessionPlaylistBase::render_plain` are now test-only: nothing outside the unit tests used them (c2 TH1).
### Dependencies
- Sibling crates move to a new epoch in this release wave: `broadcast-common` 10 -> 11, `broadcast-hls` 0.5 -> 0.6. The `Cargo.toml` requirements name the new versions; no other manifest change.

## [0.4.0] - 2026-11-01

### Changed (BREAKING: epoch)
- Builds against new caret epochs of workspace siblings (`broadcast-hls` 0.4 -> 0.5). A caret-epoch change is major-class for a 0.x crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` section 2), whatever its own API diff, so this release is a minor bump; consumers must resolve a single copy of each of those crates.
### Changed
- `InterstitialDateRange::parse_tag_line` now propagates malformed attribute-list errors from `broadcast_hls::parse_attribute_list_strict` (unterminated quoted-string, duplicate attribute name) as `Error::HlsAttrValue` instead of silently half-parsing them (#1485 R15).


## [0.3.0] - 2026-10-17
### Documentation
- README and crate-root docs rewritten to the cluster standard: what it does / does not, standards table (HLS Interstitials draft, SCTE 35), feature table, compiling quick start.

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `broadcast-hls` 0.3 -> 0.4, `mp4-emsg` 0.4 -> 0.5, `scte35-splice` 3 -> 4). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

## [0.2.1] - 2026-10-13
### Changed

- `PTS_MODULUS_33` is now an alias of `clock33::WRAP_33BIT`; added `tests/clock33_guard.rs` lexical tripwire (the linear `condition_splice_point` `abs_diff` is the one allowlisted, documented exception).

## [0.2.0] - 2026-10-05
### Added
- `splice::condition_splice_point_wrapping` + `PTS_MODULUS_33`: circular
  distance/direction on a wrapping clock, so a cue at `2^33 - 100` snaps to a
  boundary at `50` (150 ticks `After`) instead of being dropped as
  `NoAlignedBoundary` once per ~26.5 h; new `Error::PtsOutOfRange`. Two
  equidistant candidates resolve to the one `After` the cue (also in
  `condition_splice_point`), whatever the slice order (#1125).
- `playlist::SessionPlaylistBase`: renders the base playlist once and splices
  each viewer's tag line into the text (no per-viewer deep clone of every
  segment; pinned by a counting-allocator test). The line goes before the
  first `#EXT-X-PART:` or `#EXTINF:`, so for an LL-HLS base it lands before the
  first segment's parts, never between a segment's parts and its `EXTINF`
  (#1125).

### Changed (breaking)
- `render_session_playlist` (and `SessionPlaylistBase::render_into`) now
  return the new `Error::MissingProgramDateTime` when a break is requested but
  the base playlist has no `#EXT-X-PROGRAM-DATE-TIME` (RFC 8216bis §4.4.5.1
  requires one alongside any `EXT-X-DATERANGE`); previously the tag was
  emitted unconditionally (#1125).
- **`InterstitialDateRange::to_tag_line` now returns `Result<String>`**
  (was `String`), and `render_session_playlist` now returns
  `Result<MediaPlaylist>` (was `MediaPlaylist`) — issue #1140 / audit
  r14-SSAI-W1/W4/O1 (T12):
  - Every attribute value (`ID`, `START-DATE`, the `X-ASSET-URI`/
    `X-ASSET-LIST` from an `AdDecisionProvider` — typically a third-party
    ad server — plus `X-SNAP`/`X-RESTRICT`) now goes through
    `broadcast_hls::AttrValue`'s checked constructors and the shared
    `broadcast_hls::render_attribute_list`, instead of hand-formatting
    `,NAME="VALUE"` with no validation. A `"`, CR or LF in an ad-decision
    value previously terminated the attribute list and injected arbitrary
    tag lines into every viewer's session playlist; it is now `Err` from
    the rendering entry point.
  - `DURATION`/`X-RESUME-OFFSET`/`X-PLAYOUT-LIMIT` are now rejected (the
    new `Error::InvalidDuration`) if NaN, infinite, or negative, both on
    parse and on render — previously `NaN as i64 == 0` let a NaN duration
    render as the bare token `NaN`.
  - The module's own quoted-comma attribute splitter is replaced by the
    shared `broadcast_hls::parse_attribute_list` (the same tokenizer
    `timed-metadata` now also uses — audit r14-SSAI-O1 found the same
    algorithm duplicated three times across the workspace).
  - New public error variants (`Error` is `#[non_exhaustive]`):
    `Error::HlsAttrValue(broadcast_hls::Error)`, `Error::InvalidDuration`,
    `Error::TagParse` and `Error::MissingProgramDateTime`. **`HlsAttrValue`
    puts `broadcast_hls::Error` in this crate's public API**, so a
    `broadcast-hls` caret-epoch move is a major-class change here too (this
    release builds against `broadcast-hls` 0.3).

## [0.1.0] - 2026-08-11

### Added
- Initial release (issue #929): sans-IO SCTE-35 SSAI session core.
  - `session::SessionStore` / `session::BreakState` — per-session ad-break
    state (session id -> decision -> conditioned splice points), not a
    per-viewer media cursor.
  - `decision::AdDecisionProvider` trait, `decision::AdBreakDecision`,
    `decision::BreakContext` — the pluggable ad-decision extension point
    (no HTTP client, no VAST/VMAP in this crate).
  - `splice::condition_splice_point` — splice-point conditioning against
    real candidate boundaries, verified against a real, non-IDR-aligned
    SCTE-35 cue (`fixtures/scte35-ssai/`, DASH-IF `livesim2`, Apache-2.0).
  - `playlist::InterstitialDateRange` / `playlist::render_session_playlist` —
    per-session HLS Interstitial (`EXT-X-DATERANGE
    CLASS="com.apple.hls.interstitial"`) playlist rendering over
    `broadcast-hls`.
