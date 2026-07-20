# `eac3_dvb_0x06.ts` provenance (issue #641)

Derived from the real E-AC-3 syncframes already captured in `eac3.ts` (a real
ffmpeg-encoded E-AC-3 stream declared via native `stream_type 0x87`). DVB
broadcasts routinely signal E-AC-3 via the alternate convention instead:
`stream_type 0x06` (PES private data) + an `enhanced_AC3_descriptor` (tag
`0x7A`, ETSI EN 300 468 Annex D) in the ES_info descriptor loop. `eac3.ts`'s
own native-stream_type packetisation never exercises that path, so this
fixture reproduces it deterministically from real captured audio bytes.

## How it was generated

1. Demuxed `eac3.ts` with `TsDemux` and pulled its 58 real E-AC-3 syncframe
   samples.
2. Re-muxed those same samples as a `CodecConfig::Data { stream_type: 0x06,
   descriptors: [0x7A, 0x01, 0x00], carriage: Pes }` track through
   `transmux::ts_mux::TsMux` (the crate's own spec-correct TS/PES/PAT/PMT
   builder) -- one sample = one PES, byte-identical E-AC-3 payload, just
   declared under the DVB descriptor-disambiguated `stream_type` instead of
   the native one.
   - `[0x7A, 0x01, 0x00]` is the minimal legal `enhanced_AC3_descriptor`:
     tag + length(1) + an all-zero flags byte (every optional field absent).
     Verified against dvb-si's `descriptors::enhanced_ac3` module (this
     crate does not depend on dvb-si).
3. Independently verified structurally valid with TSDuck's `tstables`: PMT
   correctly shows `Elementary stream: type 0x06 (MPEG-2 PES private data)`
   with `Descriptor 0: Enhanced AC-3 (0x7A, 122), 1 bytes`.

The one-off generator script (`transmux/examples/_gen_641_fixture.rs`) is not
committed -- it was run once and deleted; this file documents the recipe.

## Used by

- `transmux/tests/dolby.rs::dvb_0x06_enhanced_ac3_descriptor_classifies_as_eac3`
