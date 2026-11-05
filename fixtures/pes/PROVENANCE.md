# Provenance

## PES extended-stream-id fixture (`fixtures/pes/`)

Independent-oracle fixture for W6/issue #1052 (r01-C1): `PES_extension_field_length`
is `marker_bit(1) + PES_extension_field_length(7)` (ISO/IEC 13818-1 §2.4.3.7, Table
2-21), not a plain 8-bit length, so every conformant `PES_extension_flag_2 == 1`
PES (any real `stream_id == 0xFD` "extended_stream_id" packet) was rejected.
Generated with `ffmpeg 8.1.2` (`-f lavfi -i "color=c=black:size=32x32:rate=5:
duration=0.4" -c:v vc2 -b:v 50k -f mpegts`, synthetic input only): the `vc2`
(SMPTE VC-2/Dirac) encoder is one of the few codecs ffmpeg muxes into TS with
`stream_id` `0xFD`, which requires `PES_extension_flag_2`/`stream_id_extension`
signalling. Only the video PID's TS packets are kept (PAT/PMT stripped). Expected
values (`PES_extension_field_length` = 1, `stream_id_extension_flag` = 0,
`stream_id_extension` = `0x60`) are read directly off the muxed bytes, not derived
from our own serializer — see `mpeg-pes/tests/it/fixture_pes_extension.rs`.

| File | Source | Date | Licence |
|---|---|---|---|
| `dirac-vc2-extended-stream-id.ts` | ffmpeg 8.1.2 `vc2` encoder output into `mpegts`, synthetic `color`/`lavfi` source; video PID's TS packets only. | 2026-09 | Own work — MIT OR Apache-2.0 |
