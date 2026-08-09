# Changelog — ssai-runtime

All notable changes to this crate. Format: [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

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
