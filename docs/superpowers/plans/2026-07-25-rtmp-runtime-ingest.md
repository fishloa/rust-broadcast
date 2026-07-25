# rtmp-runtime ingest — implementation plan (#738)

> REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Spec: `docs/superpowers/specs/2026-07-25-rtmp-runtime-ingest-design.md`.

**Goal:** a sans-IO RTMP 1.0 ingest (publish) server session engine + tokio listener, wired into multimux as a first-class `InputSpec::Rtmp` source. transmux keeps FLV→Media.

**Architecture:** bytes-in → (bytes-out, events) `ServerSession`; symmetric Parse/Serialize + byte-identical round-trip on all wire types; feeds `transmux::FlvDemux` via FLV bytes.

## Global constraints
- New crate `rtmp-runtime` (workspace member). `edition=2024`, `rust-version=1.86`, `license="MIT OR Apache-2.0"`, `#![forbid(unsafe_code)]`. Not no_std (uses Vec) but core needs no tokio.
- Every wire struct: `broadcast_common::Parse`/`Serialize` + a byte-identical round-trip test (HARD invariant). No raw-byte public API. No magic numbers outside `#[cfg(test)]` (named consts; spec §-cited).
- Every public spec/field enum: `name()->&'static str` + `broadcast_common::impl_spec_display!` (#204) + `tests/label_coverage.rs`.
- Spec grounding: cite `rtmp-runtime/docs/rtmp.md` §-numbers in module docs; no impl from memory.
- 6-gate per task: build all-features / build no-default-features / test / clippy -D warnings / fmt --check / doc -D warnings. MSRV 1.86.
- Ack seq = plain modular u32 (documented); timestamps RFC1982 serial compare.
- Ingest-only: no egress/play/AMF3/SharedObject/HMAC-handshake.

## Tasks

### T1 — crate scaffold
Create `rtmp-runtime/{Cargo.toml, src/lib.rs, src/error.rs, README.md, CHANGELOG.md}`; add to workspace `members`. Cargo.toml mirrors `rtsp-runtime` (features `default=[]`, `serde`, `tokio`, deps: broadcast-common, thiserror, log; tokio optional). `lib.rs`: crate `//!` (cite RTMP 1.0 + rtmp.md), `#![forbid(unsafe_code)]`, module decls (stubs). `RtmpError` enum (thiserror; BufferTooShort/Malformed/UnexpectedState variants) with a SKIP entry plan for label_coverage. Gate: build all-features + no-default-features + fmt + doc. Commit MSRV-pinned Cargo.lock update.

### T2 — handshake wire + FSM
`src/handshake.rs`: `S0`/`C0` (version byte, const `RTMP_VERSION=3`), `C1`/`S1`/`C2`/`S2` (1536-byte: time u32, zero/echo u32, 1528 random/echo) — Parse/Serialize + byte-identical round-trip tests. A `Handshake` sub-FSM: feed C0+C1 → produce S0+S1+S2; feed C2 → Done. Simple handshake only (document). Real-bytes test from a captured C0/C1 (add a small committed fixture snippet or synthesize from the spec constants). Gate.

### T3 — chunk headers
`src/chunk.rs`: `BasicHeader` (fmt 2 bits + csid, 1/2/3-byte encodings), `MessageHeader` (fmt 0/1/2/3 field sets: timestamp/length/type_id/stream_id), extended-timestamp rule. Parse/Serialize + round-trip for every fmt variant + the 2/3-byte csid forms + extended timestamp. Named consts for the fmt discriminants + csid thresholds. Gate.

### T4 — chunk (de)assembly
`src/chunk.rs` (cont.): `ChunkAssembler` — incremental: feed bytes, emit complete `Message`s, tracking per-csid header state (fmt 1/2/3 inherit prior length/type/stream), honoring an updatable `chunk_size` (from SetChunkSize), buffering partial payloads across calls. `ChunkWriter` — `Message` → chunk bytes at the current out chunk size. Tests: multi-chunk message reassembly, a mid-stream SetChunkSize change, fmt-3 continuation, round-trip a Message through writer→assembler. Gate.

### T5 — message + control
`src/message.rs`: `Message { header: MessageHeader, payload: Vec<u8> }`; `msg_type` consts (Audio=8/Video=9/DataAmf0=18/CommandAmf0=20/...); typed protocol-control (`SetChunkSize`/`Abort`/`Acknowledgement`/`WindowAckSize`/`SetPeerBandwidth`) + `UserControl` events (StreamBegin/…) with Parse/Serialize + round-trip. #204 label pair on the control/user-control enums. Gate.

### T6 — AMF0
`src/amf0.rs`: `Amf0Value` (Number/Boolean/String/Object(Vec<(String,Amf0Value)>)/Null/Undefined/EcmaArray/StrictArray/…​ the command subset) Parse/Serialize + round-trip; `Command { name, transaction_id, arguments: Vec<Amf0Value> }` parse/`to_body`. #204 label pair on the type-marker enum if public. Real values round-tripped. Gate.

### T7 — server session (the core)
`src/server.rs`: `ServerSession` + `ServerConfig { chunk_size, window_ack_size, expected_stream_key }` + `#[non_exhaustive] ServerEvent { Connected{app}, Publish{app,stream_key,stream_id}, Media{flv:Vec<u8>}, Eof }`. `handle_data(&mut self,&[u8]) -> Result<(Vec<u8>, Vec<ServerEvent>)>`: drives handshake → chunk assembly → AMF routing (connect/createStream/publish/deleteStream + tolerate releaseStream/FCPublish) → control replies (WindowAckSize/SetPeerBandwidth/SetChunkSize/`_result`/StreamBegin/onStatus) → Audio/Video/Data → FLV tag bytes (first emission prefixes the FLV file header). Ack accounting (modular u32). `expected_stream_key` mismatch → onStatus Failed, no Publish/Media. Unit tests per transition + the failed-key path. Gate.

### T8 — real-fixture bite
Generate `rtmp-runtime/tests/fixtures/obs-publish.bin` by capturing `ffmpeg -re -i fixtures/ts/h264_aac.ts -c copy -f flv rtmp://127.0.0.1:1935/live/testkey` against a tiny recording TCP listener (commit + PROVENANCE). Integration test: feed the capture to `handle_data`, assert it reaches Publishing + emits Connected{app:"live"}/Publish{stream_key:"testkey"}/≥1 Media, and the concatenated `Media.flv` fed to `transmux::FlvDemux` (dev-dependency) yields a `Media` with the expected track kinds. Byte-identical round-trip over the fixture's parsed frames. Mutation-check the assembler/AMF bite. Gate.

### T9 — tokio listener adapter
`src/io.rs` `#[cfg(feature="tokio")]`: `AsyncRtmpServer` — bind a listen addr, accept connections, drive one `ServerSession` each, expose an async stream/callback of `ServerEvent`. Test with a loopback publisher (feed the captured bytes over a tokio duplex, assert events). Gate (all-features).

### T10 — acceptance furniture
`tests/label_coverage.rs` (scan public enums, SKIP list for RtmpError/ServerEvent-ADT). A `fuzz/` target over `ServerSession::handle_data` (+ chunk assembler). ≥2 `examples/` (e.g. `parse_handshake.rs`, `ingest_to_flv.rs` reading the fixture via std::fs). Confirm every public enum has the #204 label pair. Gate incl `--examples`.

### T11 — multimux integration
`multimux/src/config.rs`: `InputSpec::Rtmp { listen, app: Option<String>, stream_key: Option<String> }`. `multimux/src/source/rtmp.rs`: `RtmpSource` listener connector pumping `ServerEvent::Media` → `FlvDemux` → `MediaStore`. `multimux/src/origin/mod.rs`: the `InputSpec::Rtmp` match arm under supervise/shutdown. multimux dep on rtmp-runtime. Test: a loopback RTMP publish (the fixture bytes) through the multimux source produces store segments. Gate (`-p multimux -p rtmp-runtime`).

### T12 — docs + release furniture
Crate `//!` complete, README (What's-implemented + spec citations + example), CHANGELOG `[0.1.0]`, `docs/release-notes/rtmp-runtime-0.1.0.md`, docs.rs metadata in Cargo.toml, `.github/workflows/release-rtmp-runtime.yml` (copy rtsp lane; tag `rtmp-runtime-v*`; publish after transmux). Version 0.1.0. Full gate. (Tag/publish awaits owner sign-off.)
