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

It is never re-blessed to make a change pass.
