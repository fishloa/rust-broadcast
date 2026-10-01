# `rtmp-obs-publish.bin` provenance

Byte-identical copy of `rtmp-runtime/tests/fixtures/obs-publish.bin` — copied
here (rather than referenced by relative path) so `multimux`'s own loopback
ingest test (`multimux/src/source/rtmp.rs`, issue #738 Task 11b) is hermetic
(doesn't reach across a crate boundary at `CARGO_MANIFEST_DIR`-relative test
time). See `rtmp-runtime/tests/fixtures/PROVENANCE.md` for the full capture
provenance (real `ffmpeg -f flv` publish of `app=live`, `stream_key=testkey`,
H.264 320x240 + AAC-LC 44.1kHz mono, captured against the real sans-IO
`rtmp_runtime::server::ServerSession`).

# `localhost-cert.der` / `localhost-key.der` provenance

Byte-identical copies of `rtsp-runtime/tests/fixtures/localhost-{cert,key}.der`
— copied here (same hermeticity rationale as above) for `multimux`'s own
`rtsps://` (RTSP-over-TLS) loopback test (`multimux/src/source/rtsp.rs`, issue
#804). A self-signed `CN=localhost` cert/key pair `rtsp-runtime`'s own
`tests/io_loopback.rs::tls_full_session_over_loopback` already uses to run a
real `tokio-rustls` TLS handshake against `127.0.0.1` loopback with server
name `"localhost"`; not secret (self-signed, loopback-only test fixture).

# `piff-sample-encrypted.mp4` provenance

A real PIFF-CBC sample-encrypted, fragmented MP4 (audit run 7, W15) —
generated from this workspace's own `fixtures/ts/h264_aac.ts` with Bento4:

```bash
ffmpeg -y -i fixtures/ts/h264_aac.ts -c copy /tmp/base.mp4
mp4fragment /tmp/base.mp4 /tmp/frag.mp4
mp4encrypt --method PIFF-CBC \
  --key 1:0123456789abcdef0123456789abcdef:0123456789abcdef \
  /tmp/frag.mp4 multimux/tests/fixtures/piff-sample-encrypted.mp4
```

Track 1 (video) is PIFF-CBC encrypted: the fragment's `traf` carries the PIFF
"UUID Sample Encryption Box" (`A2394F52-5A9B-4F14-A244-6C427C648DF4`), which
`fragment_looks_encrypted` must recognise. The key/IV are a throwaway
test vector, not secret. Own work — MIT OR Apache-2.0.
