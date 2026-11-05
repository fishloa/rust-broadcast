# Provenance

## W4 spec-misread descriptor fixture (`fixtures/si/`)

Independent-oracle fixture for the W4 spec-misread wave (issue #1131, C1/C2/C3/C4
in issues #1004/#1053/#1005/#1006): a PMT compiled by TSDuck `tstabcomp` 3.44
(`tstabcomp -c tsduck-w4-spec-misread-pmt.xml -o tsduck-w4-spec-misread-pmt.bin`,
synthetic hand-authored XML source, no third-party stream data), carrying a
`content_labelling_descriptor` (STC and NPT indicators, exercising the 33-bit
time-base bit layout including the bit-32 carry case), an `m4mux_timing_descriptor`
(FlexMuxTiming, 8-byte body), a `metadata_STD_descriptor` (22-bit fields with
reserved bits set), and a `J2K_video_descriptor` with `extended_capability_flag`
set. Expected field values are read directly off the compiled bytes (verified
against `tstabcomp -d`'s own decompile), not derived from our own serializer —
see `dvb-si/tests/it/tsduck_interop.rs`'s
`decodes_tsduck_compiled_w4_spec_misread_descriptors`.

| File | Source | Date | Licence |
|---|---|---|---|
| `tsduck-w4-spec-misread-pmt.xml` | Hand-authored TSDuck XML source (synthetic field values only). | 2026-09 | Own work — MIT OR Apache-2.0 |
| `tsduck-w4-spec-misread-pmt.bin` | Compiled from the above with `tstabcomp -c` (TSDuck 3.44-4676). | 2026-09 | Own work — MIT OR Apache-2.0 |
