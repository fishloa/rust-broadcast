# `rtmp-obs-publish.bin` provenance

Byte-identical copy of `rtmp-runtime/tests/fixtures/obs-publish.bin` — copied
here (rather than referenced by relative path) so `multimux`'s own loopback
ingest test (`multimux/src/source/rtmp.rs`, issue #738 Task 11b) is hermetic
(doesn't reach across a crate boundary at `CARGO_MANIFEST_DIR`-relative test
time). See `rtmp-runtime/tests/fixtures/PROVENANCE.md` for the full capture
provenance (real `ffmpeg -f flv` publish of `app=live`, `stream_key=testkey`,
H.264 320x240 + AAC-LC 44.1kHz mono, captured against the real sans-IO
`rtmp_runtime::server::ServerSession`).
