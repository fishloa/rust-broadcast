# Real closed-caption captures (CEA-608/708) - provenance

Only permissively licensed material is committed (workspace rule: "no licence
stated" = rejected). Each upstream's licence file sits next to its fixtures.

| Path | Upstream | Licence | Notes |
|---|---|---|---|
| `mux.js/multi-channel-608-captions.ts` (331 KB) | https://github.com/videojs/mux.js/tree/main/test/segments | Apache-2.0 (`mux.js/LICENSE`, "Apache License Version 2.0, January 2004") | MPEG-2 TS, H.264 video with ATSC A/53 `GA94` cc_data SEI, CC1-CC4 (multi-channel). Media origin is not declared by the repository. |
| `mux.js/dash-608-captions-init.mp4` + `-seg.m4s` (190 KB) | same | Apache-2.0 | fMP4 (DASH) carrying CEA-608 as `GA94` SEI in the video samples. |
| `shaka/captions-test.ts` (124 KB) | https://github.com/shaka-project/shaka-player/tree/main/test/test/assets | Apache-2.0 (`shaka/LICENSE`) | MPEG-2 TS with CEA-608 `GA94` SEI. Its cc_data() headers carry `process_em_data_flag = 0` (bit 7 clear), which the Table B.9-only reading of the header rejects. No per-asset provenance document upstream. |
| `libcaption/tos.scc` (8 KB) | https://github.com/szatmary/libcaption (`unit_tests/tos.scc`) | MIT (`libcaption/LICENSE.txt`, "The MIT License ... Copyright 2016-2017 Twitch Interactive") | Scenarist_SCC (CEA-608 byte pairs). Text is Tears of Steel (Blender Foundation, CC-BY 3.0) - attribution: (CC) Blender Foundation | mango.blender.org. |
| `pycaption/example.scc` (560 B) | https://github.com/pbs/pycaption/blob/main/examples/example.scc | Apache-2.0 (`pycaption/LICENSE`) | Tiny SCC. |

## Derived reference files

`*.cc_data.txt` are the per-SEI `<pts_90k> <hex cc_data()>` lists produced by
`gen_cc_data_reference.py`, an independent Python extractor (TS demux, PES
reassembly, Annex B NAL split, emulation-prevention removal, SEI payloadType 4
`itu_t_t35` with country 0xB5 / provider 0x0031 / `GA94` / user_data_type_code
0x03; H.264 7.3.2.3, ATSC A/53 Part 4 6.2.3). The Rust tests extract the same
list with the workspace crates and must reproduce it byte-for-byte.

Not committed on purpose: `sintel-captions.ts` (CC-BY content, 321 KB),
the EBU loudness set (non-commercial terms), the TSDuck streams (broadcaster
copyright not addressed; used locally only, skip-loudly).
