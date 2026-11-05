# Real closed-caption captures (CEA-608/708) - provenance

Only permissively licensed material is committed (workspace rule: "no licence
stated" = rejected). Each upstream's licence file sits next to its fixtures.

| Path | Upstream | Licence | Notes |
|---|---|---|---|
| `libcaption/tos.scc` (8 KB) | https://github.com/szatmary/libcaption (`unit_tests/tos.scc`) | MIT (`libcaption/LICENSE.txt`, "The MIT License ... Copyright 2016-2017 Twitch Interactive") | Scenarist_SCC (CEA-608 byte pairs). Text is Tears of Steel (Blender Foundation, CC-BY 3.0) - attribution: (CC) Blender Foundation | mango.blender.org. |
| `pycaption/example.scc` (560 B) | https://github.com/pbs/pycaption/blob/main/examples/example.scc | Apache-2.0 (`pycaption/LICENSE`) | Tiny SCC. |

## Moved to the private submodule

`mux.js/*` (multi-channel 608 TS, DASH fMP4) and `shaka/captions-test.ts`, with their
`*.cc_data.txt` reference lists, are in `private/fixtures/captions/`: the upstream
repositories are Apache-2.0 but declare no origin for the media, so its licence is
unclear. See `private/fixtures/captions/PROVENANCE.md`.

## Derived reference files

`gen_cc_data_reference.py` writes the per-SEI `<pts_90k> <hex cc_data()>` lists
(kept with the media in the private submodule), an independent Python extractor (TS demux, PES
reassembly, Annex B NAL split, emulation-prevention removal, SEI payloadType 4
`itu_t_t35` with country 0xB5 / provider 0x0031 / `GA94` / user_data_type_code
0x03; H.264 7.3.2.3, ATSC A/53 Part 4 6.2.3). The Rust tests extract the same
list with the workspace crates and must reproduce it byte-for-byte.

Not committed on purpose: `sintel-captions.ts` (CC-BY content, 321 KB),
the EBU loudness set (non-commercial terms), the TSDuck streams (broadcaster
copyright not addressed; used locally only, skip-loudly).
