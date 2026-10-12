# transmux 0.25.2

_Released 2026-10-12._

### Fixed

- `TsDemux`/`StreamingTsDemux`: audio frames that straddle a PES boundary are no longer dropped, for **MPEG-1/2 audio, ADTS AAC, AC-3, E-AC-3 and DTS core** (#1160). One bounded per-PID carry (at most one frame, <= 16384 bytes) holds the unfinished tail and joins it to the next PES of the same PID; the carried frame keeps the frame-exact cursor position (the PTS of the PES it started in). A carry is accepted only when a valid header follows the completed frame, is cleared on a continuity gap / dropped PES, and a dangling tail at end of stream emits nothing. AC-3/E-AC-3/DTS now resync past a PES that starts mid-frame like MPEG audio/ADTS already did (previously such a PES yielded no frames at all). Codec-config recovery now also works when the first PES starts mid-frame or a header straddles PES boundaries, and the ADTS config re-probe skips the mid-frame bytes of a carried frame (they could read as a bogus header). Verified on a trimmed CC BY 3.0 Sintel AC-3/E-AC-3 stream re-packetised in-test so frames straddle PES, and on the real `m6-single.ts` capture, whose three E-AC-3 tracks gain 4/5/1 -> 7/6/3 samples; the committed `mpeg2_mp2_pes_misaligned.ts` now yields all 39 frames (was ~15). Golden hashes for those two fixtures changed accordingly.

### Behaviour change worth knowing

A PES that starts mid-frame now resyncs for AC-3/E-AC-3/DTS (as MP2/ADTS already did) instead of yielding nothing, and codec-config recovery works across a PES boundary. Real-world effect: muxers that do not align PES to audio frames (e.g. Rai) no longer lose ~8% of MP2 frames or nearly all AAC frames.

### Verification

Synthetic misaligned-PES matrix (PES sizes down to 1 byte) for all five codecs; aligned vs re-packetised ("split") streams give byte-identical frames and timing, batch equals streaming at 17 feed sizes (1..1000 bytes); CC BY 3.0 Sintel AC-3/E-AC-3 fixtures (attribution in `tests/fixtures/audio_pes_straddle/`); revert-verified.

---

Published from tag `transmux-v0.25.2`.
