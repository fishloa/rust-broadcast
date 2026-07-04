# timed-metadata 0.3.0 — 2026-07-04

Additive (minor). Adds CEA-608/708 → WebVTT conversion.

## WebVTT (#568) — feature `cc-data`, off by default
- `Cea608CueExtractor` (CC1 pop-on/roll-up/paint-on) + `Cea708CueExtractor`
  (service 1) → cues, wrapping `cc-data`'s decode-only 608/708 models.
- WebVTT writer: cue serialization + segmented `X-TIMESTAMP-MAP=MPEGTS:,LOCAL:`
  (RFC 8216 §3.5, 33-bit wrap), aligned to media segments.
- Losses (placement/styling/708 scope, SEI-user_data input path) documented;
  lossy by design (no round-trip claim).

## Compatibility
New optional dependency: `cc-data` (≥ 0.3, feature-gated). MSRV 1.86.
