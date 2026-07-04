# transmux 0.14.0 — 2026-07-04

Fix release. Correct random-access-point detection for **open-GOP H.264**.

## Fixed — open-GOP AVC segmentation anchors (#595)
`is_sync` was set only for H.264 IDR / HEVC IRAP. Broadcast H.264 is frequently
**open-GOP** — no IDR at all; GOPs open with SPS + a non-IDR I-slice and a
recovery-point SEI. Such streams got `is_sync=false` on every access unit, so
`Segmenter` never found an anchor and buffered the whole stream into one segment.

Now an AVC access unit is a random-access point (`is_sync=true`) on an IDR **or**
a recovery-point SEI (SEI type 6, payloadType 6 — ITU-T H.264 D.2.7) **or** an
SPS-led GOP start. `TsDemux` and `StreamingTsDemux` both use it. Closed-GOP/IDR
streams are unchanged; HEVC/VVC already handled CRA/BLA. New `access_unit_is_rap`
/ `recovery_point_sei` public helpers; `is_keyframe_nal` keeps strict IDR
semantics. Open-GOP segments open on a non-IDR RAP (DASH-IF-acceptable).

## Compatibility
Requires broadcast-common ≥ 8.4. MSRV 1.86. No breaking changes (additive
helpers + a segmentation-correctness fix).
