# ssai-runtime 0.1.0

**Release date:** 2026-08-10

First publish. A sans-IO SCTE-35 server-side ad insertion (SSAI) session core
(issue #929): per-session ad-break state, a pluggable ad-decision extension
point, splice-point conditioning against real segment/keyframe boundaries, and
per-session HLS Interstitial playlist rendering. No ad-decision client, no
media manipulation — this is the driveable core an HTTP-facing adapter layers
on top of, mirroring the `rtsp-runtime`/`hls-runtime` client-vs-adapter split
used elsewhere in the workspace.

## What it is

- `session::SessionStore` / `session::BreakState` — per-session ad-break state
  (session id -> decision -> conditioned splice points), not a per-viewer
  media cursor. Every viewer in a break watches one of a small set of
  pre-conditioned ad assets; what differs per session is one small
  `BreakState` record, never a `media_plane::Trunk` cursor.
- `decision::AdDecisionProvider` trait, `decision::AdBreakDecision`,
  `decision::BreakContext` — the pluggable ad-decision extension point. This
  crate never makes a network call; implementing the trait (and doing
  whatever HTTP/VAST/VMAP round-trip that requires) is entirely the caller's
  job.
- `splice::condition_splice_point` — splice-point conditioning against real
  candidate boundaries, verified against a real, non-IDR-aligned SCTE-35 cue
  (`fixtures/scte35-ssai/`, DASH-IF `livesim2`, Apache-2.0).
- `playlist::InterstitialDateRange` / `playlist::render_session_playlist` —
  per-session HLS Interstitial (`EXT-X-DATERANGE
  CLASS="com.apple.hls.interstitial"`, draft-pantos-hls-rfc8216bis Appendix D)
  playlist rendering over `broadcast_hls::MediaPlaylist`.

## What this crate does not do

- **No HTTP client, no VAST/VMAP.** `decision::AdDecisionProvider` is the only
  extension point for "which ad to play"; there is no bundled decision-service
  client and no network dependency in this crate.
- **No media manipulation.** Splicing the actual CMAF/TS bytes at the
  conditioned splice point is `transmux`'s job (its splice/SSAI IR transforms
  and PTS/DTS rebase); this crate only decides *where* the splice point is and
  *what* the per-session manifest should say about it.
- **No `multimux` HTTP wiring.** That adapter is future work, not this crate.

`no_std` + `alloc`.

## Migration

New crate; no migration.
