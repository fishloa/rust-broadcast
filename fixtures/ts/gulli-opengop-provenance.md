# `gulli-opengop.ts` — open-GOP H.264 (no IDR)

First 8000 TS packets (byte-exact) of a real Gulli DVB-T capture, for the
open-GOP segmentation anchor fix (issue #595). H.264 video (PID 0x100) is
**open-GOP**: zero IDR (NAL type 5); GOPs open with SPS (type 7) + a non-IDR
I-slice + recovery-point SEI (type 6). In this slice: 5 SPS-led GOP starts,
586 SEI NALs, 293 coded slices, 0 IDR.

A player segmenting this for MSE/CMAF gets no `is_sync` anchor until #595
recognizes recovery-point / open-GOP RAPs. Source: `rust-skyfire/fixtures/
gulli-15s.ts` (full 15 s), truncated to whole packets.
