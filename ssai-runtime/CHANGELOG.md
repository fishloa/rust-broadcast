# Changelog — ssai-runtime

All notable changes to this crate. Format: [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

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
