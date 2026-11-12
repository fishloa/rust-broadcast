# PROVENANCE


## Rtmp-runtime goldens

Goldens for W1-R-low-a. Generated from `main` at commit 182d03f78bd5 with:

    GOLDEN_UPDATE=1 cargo test --locked -p rtmp-runtime --all-features --test golden_wire

Compared byte-for-byte by the same test without `GOLDEN_UPDATE`. Only Task 4
(`transport-header.golden`, parameter order) may regenerate one, after a reviewed diff.

## RTMP push wire goldens

`push-av-flv.txt` records the exact RTMP message bodies `multimux`'s RTMP
push transport publishes for `fixtures/flv/av.flv`'s two tracks (real H.264 +
AAC): the `onMetaData` script body, the FLV tag headers of the published
sequence, and the AVC sequence header, AAC sequence header and first media
bodies. The bytes were captured off a real loopback socket by an independent
RTMP server implementation (`rtmp_runtime::server::ServerSession`).

Taken from `main` at commit 2ee7f4c52a06662e8b50480dd1ecf66ba660848d, before
the `onMetaData` builder and the FLV codec gate moved from `multimux` into
`transmux` (issue #2369). Regenerated only by the test that owns it:

    REGENERATE_PUSH_GOLDENS=1 cargo test -p multimux --all-features --locked --test it push_wire_goldens

Only the `onMetaData_body` line has ever been re-recorded; the test's own
regenerate switch rewrote that one line and no other byte of the file. The
tag headers, both sequence headers and the first media body are the bytes
taken at the commit above.

What changed in that line, over two reviewed steps:

1. The push stopped building its own AMF0 `Object` and frames the shared
   `transmux::on_metadata_body` instead. The old push wrote `width`,
   `height`, `videocodecid`, `audiocodecid`, `audiosamplerate` and
   `audiochannels`, and its client wrapped them in an Object (a receiving
   server turned that into a `03` script tag). The shared body carries an
   ECMA array (an `08` script tag, with the end-of-object marker) — the same
   shape `FlvMux` writes to a file — and no `duration`, which a live stream
   has none of.
2. `audiosamplerate` was added back to the shared builder (issue #2369
   follow-up): Adobe FLV v10.1 §E.5
   (`private/specs-md/adobe_flv_f4v_v10_1.md`) defines `audiosamplerate`
   (Number) and `stereo` (Boolean) and does not define `audiochannels`.
   `stereo` is now derived from the AAC track's channel count rather than
   always `true`, so this fixture's mono AAC track records `false`; the old
   push's `audiochannels` is not written by any writer.

So the recorded properties are, in order: `width`, `height`,
`videocodecid`, `audiocodecid`, `audiosamplerate`, `stereo` — and the
`audiochannels` key the pre-#2369 push wrote is gone. The record is never
re-blessed again to make a change pass.
