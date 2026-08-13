# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A Rust workspace of DVB (Digital Video Broadcasting) protocol parsers + builders, published to crates.io:

- **broadcast-common** — shared `Parse<'a>` / `Serialize` traits, the `mux` container-mux traits (`Unpackage`/`Package`/`Encrypt`/`Decrypt`), CRC-32/MPEG-2, the `bcd`/`time`/`hex` codecs, and `cenc::CencScheme` (the container-independent ISO/IEC 23001-7 scheme identity that `transmux`'s crypto boxes and `broadcast-hls`'s `#EXT-X-KEY` both name — defined once here, below both). Everything else depends on it; versioned independently of the DVB lockstep.
- **dvb-si** — the big one: ETSI EN 300 468 Service Information + MPEG-2 PSI. All 29 allocated table_ids, descriptors, DSM-CC data carousel, Annex A text decoding. TS packet / section reassembly lives in `mpeg-ts` (used internally via the `ts` feature).
- **mpeg-ts** — generic MPEG-2 TS framing (ITU-T H.222.0 / ISO/IEC 13818-1): TS packet, adaptation field, PCR, PSI section reassembly + packetisation, resync. `no_std`. Independently versioned.
- **container-probe** — media container-format detection over a byte prefix (issue #960): MPEG-2 TS (a stride×phase lattice search across 188/192/204/208, so M2TS and mid-packet captures are found — the fixed-offset check in `transmux::cli::detect_container` this replaces missed both), ISOBMFF, Matroska/WebM, MPEG-PS, FLV, MXF, WAV, Ogg, ASF, and the ADTS/MP3/AnnexB elementary streams. **Every prober always runs and scores its evidence** (`CERTAIN`/`STRONG`/`STRUCTURAL`/`LATTICE_*`/`HEURISTIC`); the highest wins and a near-tie reports `Ambiguous` rather than silently picking, so the verdict never depends on declaration order. Scoring is structural, not magic-byte: a lane needs 50% sync *coverage* (a CENC MP4 aligned three `0x47`s by chance and was confidently misidentified without it) and an elementary stream needs a *frame-length chain* (one TS fixture carries 18 239 MP3 syncwords — 34× the real MP3 file). `Insufficient` ("read more") and `Unknown` ("stop") are distinct and load-bearing — `need_at_least` always exceeds the bytes *examined* (`min(len, budget)`, not `len`) and grows geometrically when no structural need can be named, so the documented caller loop converges in O(log n) reads rather than crawling; `DEFAULT_BUDGET` is public because a caller must feed the buffer length back as the budget for that loop to terminate. Needs no new spec transcription — every constant is reused from a sibling crate that round-trips real fixtures, pinned by a dev-dep drift guard. `no_std`+`alloc`, depends only on `broadcast-common`; independently versioned.
- **dvb-t2mi** — TS 102 773 T2-MI packet/payload parsing.
- **dvb-bbframe** — DVB-S2/S2X/T2 BBFrame headers, user packet extraction.
- **scte35-splice** — ANSI/SCTE 35 splice information (DPI cueing); independently versioned (left the DVB lockstep at v1.0.0).
- **dvb-conformance** — ETSI TR 101 290 stream conformance monitor.
- **compliance-probe** — live probe over a `media-plane` `Trunk`/`ByteTap`: drives `dvb-conformance`'s TR 101 290 indicators, a PCR-drift/jitter estimate (explicitly distinct from, and no replacement for, TR 101 290 indicator 2.4, which `dvb-conformance` does not emit), and SCTE-35 `splice_insert` cue-sanity checks, exported through the `metrics` facade for a host process to render as Prometheus. `std` only. Unpublished.
- **dvb-tools** — CLI analyzer (`dump`/`services`/`epg`/`pids`/`t2mi`).
- **dvb-stream** — async/tokio stream adapters; independently versioned.
- **mpeg-pes** — PES depacketization + PTS/DTS (ISO/IEC 13818-1 §2.4.3); `no_std`, depends only on broadcast-common; independently versioned.
- **dvb-subtitle** — ETSI EN 300 743 DVB (bitmap) subtitling segments (page/region/CLUT/object/DDS/disparity + 2/4/8-bit pixel-data sub-blocks), fed the subtitle PES data field; `no_std`, depends only on broadcast-common; independently versioned.
- **dvb-vbi** — VBI data carriage in DVB (ETSI EN 301 775 §4): the PES data field with VPS, WSS, Closed Captioning, EBU/Inverted Teletext, and monochrome 4:2:2 luminance sample data units; `no_std`, depends only on broadcast-common; independently versioned.
- **mpeg-ps** — MPEG-1/2 Program Stream framing (ISO/IEC 13818-1 §2.5): pack header (42-bit SCR), system header, program stream map; PES via mpeg-pes; `no_std`; independently versioned.
- **scte104** — ANSI/SCTE 104 2023 automation→compression DPI signalling: single/multiple operation messages + the full operation set; `no_std`, depends only on broadcast-common; independently versioned.
- **ssai-runtime** — sans-IO SCTE-35 SSAI session core: per-session ad-break state (`SessionStore`/`BreakState`), a pluggable `AdDecisionProvider` extension point (no HTTP client, no VAST/VMAP), splice-point conditioning against real candidate boundaries, and per-session HLS Interstitial (`EXT-X-DATERANGE CLASS="com.apple.hls.interstitial"`) playlist rendering over `broadcast-hls`. `no_std`+`alloc`. Unpublished.
- **playout-runtime** — sans-IO linear channel playout: a schedule model (programme/ad/slate), transition planning across a join (PTS-rebase offset + discontinuity detection), and SCTE-35 `splice_insert()` emission points built with `scte35-splice`. No HTTP, no tokio, no transcoding. `no_std`+`alloc`. Unpublished.
- **cc-data** — DVB closed-caption carriage cc_data() (ETSI TS 101 154 Table B.9): typed CEA-608/708 triplets + 608/708 split; `no_std`, depends only on broadcast-common; independently versioned.
- **caption-convert** — caption/subtitle format conversion: CEA-608/708 and EBU Teletext to WebVTT/SRT, and WebVTT <-> SRT, wrapping the extractors `timed-metadata` already implements (issues #568/#666) rather than reimplementing cue-boundary handling. DVB bitmap subtitles and TTML/IMSC source conversion are documented gaps, never silently degraded (see the crate's conversion matrix). `no_std`+`alloc`. Unpublished.
- **dvb-ci** — DVB Common Interface (EN 50221): APDU/resource objects, session/transport PDUs, and a CA_PMT builder from a dvb-si PMT; `no_std`+`alloc`, depends on dvb-si; independently versioned.
- **dvb-ci-runtime** — EN 50221 driver runtime: device I/O, TPDU/SPDU poll loop, resource state machines, CAM/card hot-plug notifications, and a managed CAS layer (`CaDescrambler`); built on `dvb-ci`; independently versioned.
- **dvb-csa** — DVB Common Scrambling Algorithm (CSA2): pure-Rust block + stream cipher, oracle-validated against `libdvbcsa` known-answer vectors, with an optional bitsliced 64-payload batch fast path. Scramble/descramble TS payloads with an 8-byte control word. `no_std`; independently versioned.
- **dvb-simulcrypt** — DVB SimulCrypt head-end CA message framing (ETSI TS 103 197): generic TLV message plus ECMG⇔SCS and EMMG/PDG⇔MUX message_type/parameter_type registries; signalling only — CW/ECM/EMM/datagram payloads opaque; `no_std`, depends only on broadcast-common; independently versioned.
- **st291** — SMPTE ST 291-1 ancillary (ANC) data content: parse/build ANC data packets (DID/SDID, user data), currently carried over its `ts` transport (SMPTE ST 2038:2021 MPEG-2 TS/PES: `anc_data_descriptor` + ANC data PES packet), plus an RTP transport (ST 2110-40 / RFC 8331) behind the `rtp` feature (#648); `no_std`, independently versioned. (Renamed from `smpte2038`/`dvb-smpte2038`, both yanked from crates.io rather than kept as shims.)
- **st12-1** — SMPTE ST 12-1:2014 Linear Timecode (LTC): the 80-bit logical codeword with BCD time address, drop/color frame flags, eight 4-bit binary groups, and sync word; `no_std`, depends only on broadcast-common; independently versioned.
- **st337** — SMPTE ST 337:2015 non-PCM audio/data burst-preamble framing over AES3 (`Pa`/`Pb`/`Pc`/`Pd` sync + info/length words, opaque compressed-audio payload); `no_std`, depends only on broadcast-common; independently versioned.
- **st377-1** — SMPTE ST 377-1:2019 Material Exchange Format (MXF): KLV framing, Partition Pack, Primer Pack, local-set structural metadata (Preface/Identification/ContentStorage/EssenceContainerData), Random Index Pack — the first file-based-interchange crate in the workspace; `no_std`, depends only on broadcast-common; independently versioned.
- **rdd29** — SMPTE RDD 29:2019 Dolby Atmos bitstream: frame/element framing plus bed/object rendering metadata (`BedDefinition1`, `ObjectDefinition1`, `AudioDataDlc`); `no_std`, depends only on broadcast-common; independently versioned.
- **ule** — Unidirectional Lightweight Encapsulation (RFC 4326): SNDU framing + bridged/non-bridged PDU parsing over DVB-S/T/C MPEG-2 TS; `no_std`, independently versioned.
- **rmt-flute** — ALC/LCT/FLUTE/NORM multicast object-delivery wire formats (RFC 5651/5775/6726/5740): LCT headers with flag-driven CCI/TSI/TOI sizing, header-extension chains, ALC + FEC Payload IDs, FLUTE EXT_FDT/EXT_CENC, and NORM messages; `no_std`, depends only on broadcast-common; independently versioned.
- **atsc3** — ATSC 3.0 (A/331) LLS binary envelope + Service List Table (SLT) parser — the ATSC analogue of `dvb-si`'s service signalling. `no_std`+`alloc`. Unpublished; not yet mature enough to publish (do not tag/release until coverage is significant).
- **atsc3-route** — ATSC A/331 Annex A ROUTE binary framing: `EXT_ROUTE_PRESENTATION_TIME`/`EXT_TOL` LCT header extensions, source/repair FEC Payload ID layouts, and the Codepoint (CP) delivery-object semantics table. Built on `rmt-flute`'s LCT/ALC/FLUTE (the entry above); no XML/S-TSID/SLS. `no_std`+`alloc`. Unpublished.
- **rtp-packet** — RFC 3550 §5.1 RTP fixed header + CSRC list + §5.3.1 generic header extension, with optional RFC 8285 one-byte/two-byte multiplexed extension decoding; `no_std`+`alloc`, depends only on broadcast-common; independently versioned.
- **rtcp-packet** — RTCP control packets: SR/RR/SDES/BYE/APP + compound packet (RFC 3550 §6); spec-complete parse/serialize; `no_std`+`alloc`, depends only on broadcast-common; independently versioned.
- **rist-runtime** — RIST Simple Profile (VSF TR-06-1:2020) RTCP message types: Generic NACK (RFC 4585), Range NACK, RTT Echo, and compound packet builders. `no_std`+`alloc`. Unpublished.
- **st2022** — SMPTE ST 2022-6 HBRMT (SDI-over-IP) RTP payload header parser/serializer. `no_std`. Unpublished.
- **mp4-emsg** — ISO BMFF / DASH Event Message Box (`emsg`, ISO/IEC 23009-1): version 0/1 parse + serialize for inband DASH/CMAF timed events (SCTE 35 splice, ID3, ad/tracking); `no_std`, independently versioned.
- **timed-metadata** — Convert DPI/timed-metadata signalling between SCTE-35, HLS `EXT-X-DATERANGE` (RFC 8216 §4.4.5.1), and DASH `emsg` (ANSI/SCTE 214-3); lossless round-trips, 33-bit PTS wrap-unroll via `Timeline`; `no_std`; independently versioned.
- **ttml-subtitle** — W3C TTML2 / IMSC 1.1 timed-text subtitle parser + profile validator; parse XML into typed Rust structures, validate against IMSC 1.1 Text/Image profiles separately; uses `roxmltree` for parsing, manual XML serializer; `no_std`+`alloc`; independently versioned.
- **dvb-mabr** — DVB multicast ABR (ETSI TS 103 769) session configuration XML parser/serializer. `no_std`+`alloc`. Unpublished.
- **broadcast-hls** — HLS (M3U8) playlist syntax (RFC 8216 / RFC 8216bis): `MediaPlaylist`/`MasterPlaylist` parse + serialize, Low-Latency HLS (`LowLatencyConfig`/`PartSpec`/`OpenSegment`/`MapTag`/`RenditionReport`/`SkipInfo`), I-frame trick-play (`IFrameVariant`), discontinuity signalling (`mark_init_discontinuities`), and CENC/CBCS `#EXT-X-KEY` signalling (`cenc_ext_x_key`). Extracted from `transmux/src/hls.rs` (issue #878) so a consumer that only needs playlist syntax (`media-doctor`, an HLS-pull client, a fuzz target) doesn't have to pull in the whole container-muxing hub; `transmux`'s HLS/LL-HLS **segmenters** (`ts_hls`, `ll_hls` — they produce container bytes) stayed put and depend on this crate. `no_std`+`alloc`, depends only on `broadcast-common`; independently versioned.
- **transmux** — any-to-any media **container** muxing hub (ISO/IEC 14496-12 / 13818-1 / 23009-1, RFC 8216/3550, MS-SSTR): demux any input (TS/fMP4/PS/WebM/FLV/RTMP) into one neutral IR (`Media`/`Track`) and mux to any output (CMAF/progressive-MP4/TS/CMAF-HLS/TS-HLS/DASH/LL-DASH/LL-HLS/Smooth); repackage, CENC decrypt, RTP/RTCP, IR transforms (PTS/DTS rebase, splice/SSAI, trick-play), fMP4/CMAF conformance validator, and a `cli`-gated `transmux` packager binary. HLS playlist syntax lives in `broadcast-hls` (issue #878); parses codec config headers only — never en/decodes; samples opaque. `no_std`+`alloc`; independently versioned.
- **broadcast-loudness** — EBU R 128 / ITU-R BS.1770-5 loudness measurement: K-weighting filter, gated integrated loudness (LUFS), momentary/short-term, loudness range (EBU Tech 3342) and true peak (dBTP). Accepts any sample rate greater than zero (44100, 48000, 96000, 192000, …) — `LoudnessMeter::new` derives the K-weighting biquad coefficients for the given rate by a bilinear transform of the analog prototype filters (matching the BS.1770-5 Annex 1 tabulated coefficients at 48 kHz to within floating-point epsilon), and returns `Error::InvalidSampleRate` only for `sample_rate == 0`; non-finite input samples are rejected for a similar reason (one NaN would poison the IIR state permanently). Verified against the EBU Tech 3341 compliance test signals. No wire format, so the workspace round-trip invariant does not apply — the compliance vectors are its equivalent hard gate. `no_std`+`alloc`, depends only on broadcast-common; independently versioned.
- **broadcast-auth** — shared multi-scheme HTTP/RTSP auth: client `Credentials`/`Authenticator` (Basic/Digest via `http-auth`, Bearer RFC 6750) + server `Verifier` (challenge+verify, including a reverse-proxy `forwarded` scheme) over a `RequestContext` (method/uri/body/headers/peer_addr); consumed by `rtsp-runtime`, `hls-runtime`, and `multimux`. Independently versioned.
- **rtsp-runtime** — sans-IO **RTSP 1.0** (RFC 2326) session engine: driveable client + server state machines (Appendix A), CSeq correlation, `Transport` negotiation, interleaved RTP/RTCP framing, Basic/Digest/Bearer auth (via `broadcast-auth`), over the `rtsp-types` + `sdp-types` codecs; optional `tokio` (+ `tls` for `rtsps://`) socket adapter. Independently versioned.
- **hls-runtime** — sans-IO **Low-Latency HLS** (RFC 8216bis) client + server engine, mirroring rtsp-runtime's client+server split (renamed from `ll-hls-client`): a caller-driven playback client (blocking-reload scheduler, part-prefetch, `Fmp4Demux`-based output, optional `tokio`+`reqwest` IO adapter) and a sans-IO origin engine (`server`, feature `std`: `HlsOrigin` resolving init/segment/part bytes and rendering playlists over a `media_plane::Trunk`'s own rings, plus the blocking-reload/part-availability decision logic) that `multimux` adapts over tokio+axum. The rolling-window `MediaStore` this engine replaced is **deleted** — the `Trunk` is the single copy of the data, never a second cache of it. Independently versioned.
- **srt-runtime** — SRT (draft-sharabayko-srt-01) packet codecs + sans-IO HSv5 Caller-Listener and Rendezvous handshake, ARQ reliability engine, TSBPD delivery scheduler, LiveCC/FileCC congestion control, and optional payload encryption and tokio adapter; `no_std` core, depends on broadcast-common; independently versioned.
- **rtmp-runtime** — sans-IO RTMP 1.0 ingest/publish session engine: handshake, chunk-stream (de)assembly, AMF0 command routing, and server-side state machine (`connect` → `createStream` → `publish`); optional tokio socket adapter; depends on broadcast-common; independently versioned.
- **webrtc-runtime** — sans-IO WHIP (RFC 9725) + WHEP (draft-ietf-wish-whep) HTTP signalling engine: SDP offer/answer, Trickle ICE, ICE restart, session lifecycle. No IO adapter — the caller drives HTTP. The default build is `no_std`-capable and never touches the optional `media` feature (ICE + DTLS-SRTP transport via `rtc-ice`/`rtc-dtls`/`rtc-srtp`), which is what `multimux`'s `whip`/`whep` features build on. Unpublished.
- **media-plane** — the ingress/egress spine a live origin is built on, four layers: `Dialer`/`Listener` → `[ByteStage]*` → `IngestSession` → `[IrTransform]*` → `TrunkWriter` → `Trunk`. A `Trunk` is the per-program hub — bounded sample/segment/event/part rings with cursor subscribers (`Lagged` reported in-band, never silently dropped), a `ProgramId`-keyed track set, and `listen()` for bounded wake-ups. Three egress shapes (`ServedEgress` with `EgressResponse::Await` for blocking reload, `PushEgress`, `SegmentEgress`), tiered retention with DVR pinning and `ArchiveOverrun` policy. Writer cost is O(N) in *cursor* count, so a cursor is per distinct consumer, never per peer. Byte layer is `no_std` + `alloc`; `Trunk` and above need `std`. Independently versioned.
- **multimux** — multi-input (RTSP/RTP/TS-UDP/TS-HTTP/SRT/HLS-pull/DASH-pull/Smooth-pull/RTMP/file) × multi-output (LL-HLS/DASH/LL-DASH) just-in-time repackaging HTTP origin (library: tokio + axum): shared output auth (Basic/Digest/Bearer/Forwarded) across every route, an external scheme plugin registry (`Custom` input/output/output-auth + `SchemeRegistry`) so a third-party crate can add a scheme without editing this crate, Prometheus metrics + health/readiness, supervised reconnect. Built on `rtsp-runtime`/`hls-runtime`/`broadcast-auth`/`transmux`. Independently versioned.
- **multimux-cli** — the `multimux` CLI binary: `--config <FILE>` (JSON routes) or the single-route quick start (`--rtsp`/`--name`, `--outputs`/`--dash`). Independently versioned.
- **ts-fix** — MPEG-2 TS stream-conditioning CLI (PCR/continuity/timestamp repair); independently versioned.
- **media-doctor** — container/stream diagnostics (fMP4/CMAF/TS structural checks); independently versioned.
- **dvb-si-py** (`bindings/python/`) — PyO3/maturin Python bindings over dvb-si/dvb-t2mi: `parse_section(bytes)->dict` + `Demux`/`T2miDemux` classes (read-only, parse→serde_json→Python). NOT a workspace member (own MSRV); consumes published crates by version; abi3 wheels to PyPI via its own workflow.
- **transmux-py** (`bindings/transmux-py/`) — PyO3/maturin Python bindings over transmux: `demux_ts(bytes)->dict` exposing the `Media`/`Track`/`Sample` IR (codec identity/RFC 6381 string, timescale, opaque coded sample bytes) for ML/analysis front-ends (docs/IDEAS.md item #7). Hand-converts Rust structs to `PyDict`s field-by-field (transmux's pipeline IR carries no `serde::Serialize`, unlike dvb-si-py's json round-trip). NOT a workspace member (own MSRV); consumes published transmux by version; abi3 wheels to PyPI via its own workflow.
- **`cpix/`** and **`st2110/`** are **docs-only directories, not crates** — each holds a `docs/` tree of spec transcriptions only (no `Cargo.toml`, no `src/`), not a workspace member, nothing to publish. They exist so a future crate has real, cited spec groundwork to start from; don't confuse either for an implemented crate.

MSRV is **1.95.0** (workspace `rust-version`, inherited by every member via `rust-version.workspace = true`); the committed `Cargo.lock` pins MSRV-compatible deps — always build/test with `--locked`.

## Commands

```bash
# Full check, exactly what CI runs (CI sets RUSTFLAGS="-D warnings"):
cargo build --workspace --all-features --locked
cargo test  --workspace --all-features --locked
cargo build --workspace --no-default-features --locked
cargo clippy --workspace --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked

# macOS CANNOT verify Linux-gated code. `dvb-ci-runtime/src/linux.rs` is behind
# `#[cfg(all(feature = "linux", target_os = "linux"))]`, so a clean local
# clippy on macOS says nothing about it — two lints there passed every local
# gate and failed CI twice (blocking clippy AND the canary, same root cause).
# Cross-check that crate before pushing:
rustup target add x86_64-unknown-linux-gnu
cargo clippy -p dvb-ci-runtime --all-features --all-targets --locked \
  --target x86_64-unknown-linux-gnu -- -D warnings
# Whole-workspace cross-clippy does NOT work from macOS (`ring`'s build script
# needs a C cross-toolchain). `dvb-ci-runtime` is currently the only crate with
# `target_os` gating, so it is the only one that needs this.

# The clippy canary — a PINNED stable newer than the MSRV, non-blocking in CI.
# Pinned so it is reproducible: `cargo +stable` runs whatever you happen to have
# installed, which is NOT what CI runs and will silently disagree with it (that
# mismatch let the canary sit red for months — issue #770). Use it by name:
rustup toolchain install 1.97.1
cargo +1.97.1 clippy --workspace --all-features --all-targets --locked -- -D warnings
# Keep this in step with `CANARY_RUST` in .github/workflows/ci.yml. Bumping the
# pin is its own PR: raise it, fix the new lints, land them together.

# Scoped runs:
cargo test -p dvb-si --all-features                # one crate
cargo test -p dvb-si --test round_trip             # one integration test file
cargo test -p dvb-si descriptors::pdc              # tests matching a path

# Analyzer CLI (the dvb-tools binary crate):
cargo run -p dvb-tools -- dump dvb-si/tests/fixtures/m6-single.ts
cargo run -p dvb-tools -- dump dvb-si/tests/fixtures/m6-single.ts --json
cargo run -p dvb-tools -- t2mi <file.ts> [--pid 0xNNN|raw] [--inner] [--plp N]
cargo run -p dvb-tools -- services|epg|pids <file.ts>

# HLS conformance oracle (issue #870): Apple's OWN `mediastreamvalidator` —
# independent of both our renderer and our own `check_hls_playlist` checker,
# so it catches a misreading of the HLS spec shared by both of those (unlike
# validating our renderer with our own checker, which just proves the two
# agree with each other). macOS-only (Apple's Additional Tools for Xcode,
# `/usr/local/bin/mediastreamvalidator`) — every test skips itself loudly
# (prints why, via `--nocapture`) when the binary isn't on PATH, so this is a
# no-op pass on Linux/CI; run it locally on macOS for the genuine check:
cargo test -p media-doctor --test mediastreamvalidator_oracle --all-features --locked -- --nocapture
```

Formatting is rustfmt-clean and CI-gated (`cargo fmt --all --check`). The deliberately column-aligned enums (`TableId`, `DescriptorTag`) carry `#[rustfmt::skip]` — keep the attribute (and the alignment) when editing them, and use the same pattern for any new aligned table. Cargo.toml manifests keep their manual column alignment (rustfmt doesn't touch them).

Docs are warning-clean and CI-gated (`RUSTDOCFLAGS="-D warnings"`). Bit-range notation in doc comments must be backticked — `` `[7:4]` `` — or rustdoc parses it as an intra-doc link.

## Workflow: GitHub issues drive the work

Work in this repo is tracked as GitHub issues and lands via PRs to `main`. Use the `gh` CLI.

1. **Pick up work from an issue.** `gh issue list` to see open work; `gh issue view <n>` for the spec/acceptance criteria. If you're asked to do something non-trivial that has no issue, create one first (`gh issue create`) so the work is tracked.
2. **Branch per issue** off `main`, named for the work (e.g. `complete-descriptors`, `fix-tot-crc`).
3. **Commit style** follows the existing history: `feat(carousel): …`, `fix(text): …`, `docs(dvb-si): …`, or a plain scoped summary. Imperative, specific, references the spec section when relevant.
4. **Open a PR** with `gh pr create`, body referencing the issue (`Closes #n`). CI must pass before merge:
   - test matrix on stable **and** 1.95.0 (MSRV) — all-features and no-default-features builds
   - `cargo fmt --all --check`
   - clippy `-D warnings` on all targets
   - doc build with `RUSTDOCFLAGS="-D warnings"`
5. **Releases are tag-driven and CI-only.** Bump all **five** lockstep crate versions together (`dvb-si`, `dvb-t2mi`, `dvb-bbframe`, `dvb-conformance`, `dvb-tools`), merge, then push a `v<version>` tag — `release.yml` gates (tests, clippy, tag==version check) and publishes to crates.io in dependency order (`dvb-bbframe` first). **`broadcast-common` is NOT in the lockstep** — it versions on its own API changes and ships from its own `broadcast-common-v*` tag (`release-broadcast-common.yml`). It was ejected because lockstep forced it to major whenever a sibling broke (9.0.0 was purely additive and should have been 8.2.0), and with a fan-in of 35 each inherited major migrated thirty uninvolved crates — the origin of #819/#824/#858. If a wave moves it too, publish `broadcast-common-v<x>` FIRST and let it go live before the `v<y>` tag. `dvb-stream`, `scte35-splice`, `mp4-emsg` and the other independent crates release on their own cadence or not at all. **Never `cargo publish` from a workstation.**
6. **Every release produces documentation** per [`docs/RELEASE-DOCS.md`](docs/RELEASE-DOCS.md) — the authoritative standard for the docs.rs / crates.io / GitHub surfaces. Run its **per-release checklist** each tag (CHANGELOG → release note → README coverage → crate-root `//!` → Cargo.toml + docs.rs metadata sweep → GitHub Release → post-publish verify docs.rs built green). This is enforced like the gate suite.
6b. **Every NEW crate clears** [`docs/CRATE-ACCEPTANCE.md`](docs/CRATE-ACCEPTANCE.md) — the consolidated new-crate hard bar (round-trip invariant, no raw-byte API, real-fixture biting tests, verified-source/no-fabrication discipline, 6-gate suite, #204 labels, fuzz, >=2 examples, full RELEASE-DOCS). A **living standard**: tighten it whenever a new failure mode appears.
7. **Every release is audited** per [`docs/RELEASE-AUDIT.md`](docs/RELEASE-AUDIT.md) — the full battery of tests/checks run before a tag: the 6-gate CI suite (run yourself, not on CI/subagent say-so), the version + inter-crate dep-ref consistency audit (the v7.7.0 partial-publish trap; verify live versions against crates.io, not CHANGELOG headings), and the adversarial **extensibility/code-quality audit** (round-trip symmetry + no `self.raw` passthrough, no raw-byte public API, decode-completeness, spec-fidelity/no-magic-numbers, the #204 `name()`+`impl_spec_display!` label convention + per-crate `label_coverage` drift-guard, `#[non_exhaustive]`, panic-class safety, `declare_*`-macro dispatch). Companion to the doc standard above; [`AUDIT-LEDGER.md`](docs/AUDIT-LEDGER.md) records which PDF→md fidelity audits are already done.
   - **Epoch-pure compat buckets** — changing a workspace-sibling's caret-epoch within the same version bucket is a major-class bump (0.x→0.y+1, >=1→+1 major). Machine-checked by `tools/check-published-dep-consistency.py` checks 1–2. See the rule in `RELEASE-AUDIT.md` §2.

## Workflow: the delegated-engineering loop

Token-heavy authoring is delegated to DeepSeek (via the `delegate` skill → headless `opencode`); Claude stays the orchestrator, auditor, and release engineer. **Claude never marks a story done on the delegate's say-so** — only on its own fresh gate evidence.

**Claude owns (does NOT delegate):** story ordering (by dependency then value-for-effort), version semantics (patch = fixes only, minor = additive API, major = breaking; lockstep across the five lockstep crates, with `broadcast-common` versioned independently). **Changing which caret-epoch of a workspace sibling a crate builds against is a major-class change in that crate** — minor for 0.x, major for >=1.0 — because a caret bucket spanning two epochs breaks consumers of BOTH the new and the old line (#858). Every published bucket stays epoch-pure, release bundling (batch related additive stories into one minor; ship breaking/urgent work standalone), and the correctness of every CHANGELOG, `docs/release-notes/vX.Y.Z.md`, README coverage table, module spec-citation, and example/doctest.

**Per-story loop:**

1. **Scope** — read the issue's acceptance criteria and the cited `docs/` transcription. Resolve any design ambiguity *before* delegating (the delegate sees only the brief, none of this context).
2. **Baseline** — branch `story/<n>-<slug>` off `main`; commit any in-flight working state first, so the delegate's `git diff` is cleanly attributable.
   - **Prep the gate's inputs BEFORE delegating** (spec/parser work): transcribe the spec syntax tables into the *target crate's own* `docs/`, AND commit a **real fixture** (extract from an existing capture — e.g. scan committed `.ts` for the signature — pull a TSDuck stream, or use spec test vectors). Inline hand-made bytes only test the happy path; real data carries the reserved bits / mixed stream_ids / real layouts that expose bugs. No spec md + no real fixture = do not delegate yet.
3. **Delegate** — write a self-contained brief (exact files, decided behaviour/signatures, the project conventions that apply, the exact gate commands, and "fix until all pass before finishing"; boundaries: touch only <scope>, do not commit). Run in the background. The brief's exit gate must be **ungameable** and include a **real-fixture run** (parse + byte-exact round-trip the committed fixture) — a plain round-trip test is gameable by raw-passthrough serialize, and a green inline suite is not "done". Pass these gates in round ONE, never bolt them on after burning a round.
4. **Audit** — judge by `git diff` + running the **full gate suite yourself** (see Commands), never by the delegate's stdout (often empty on success) or its claims. Then check line-by-line against every AC and the hard invariants (symmetric serialize + round-trip test, no magic numbers outside `#[cfg(test)]`, spec citation in the module doc, `--no-default-features` builds, feature-gating). If a delegated test doesn't *bite*, reject or rewrite it — Claude owns verification.
5. **Drive fixes** — feed concrete findings back via `opencode run --continue` (same session keeps context). After 2 failed fix cycles on the same point, take over and finish it directly.
6. **Repeat 4–5** until every gate is green *and* every AC is met, on Claude's own run.
7. **Ship** — update CHANGELOG/release-note/README/examples; branch→PR (`Closes #n`)→CI green→merge; then the lockstep version bump + `v<version>` tag (per the tag-driven release rule above). Verify all five lockstep crates went live (plus `broadcast-common` and any other independent crates in the same release).

**Continuous improvement:** treat this loop as living. When a brief pattern, gate ordering, or audit check repeatedly saves (or costs) time, refine this section and say so in the turn. Recurring delegate failure modes belong in the brief template, not rediscovered each story.

## Architecture

### The Parse/Serialize contract (broadcast-common/src/traits.rs)

Every wire structure in every crate implements the same symmetric pair:

- `Parse<'a>` — `parse(&'a [u8]) -> Result<Self>`, borrowing from the input (zero-copy: parsed structs hold `&'a [u8]` slices and carry `<'a>` lifetimes).
- `Serialize` — `serialized_len()` + `serialize_into(&mut [u8])`.

Every parser has a symmetric serializer and a **round-trip test** (parse → serialize → byte-identical, and serialize → parse → equal). This symmetry is a hard project invariant.

### dvb-si layout

- `tables/` — one file per table (pat, pmt, sdt, eit, nit, …). Tables expose typed header fields; descriptor loops are borrowed `&[u8]` slices the caller walks with the descriptor parsers.
- `descriptors/` — one file per descriptor tag. Each module exports a `TAG` const, length consts, a `XxxDescriptor<'a>` struct, and the Parse/Serialize impls. `descriptors/any.rs` defines `AnyDescriptor` + `parse_loop` (the lazy descriptor-loop walker); `descriptors/registry.rs` adds `DescriptorRegistry` for private tags.
- `carousel/` — DSM-CC DSI/DII/DDB messages + `ModuleReassembler`, layered on `tables/dsmcc.rs` section framing.
- `text/` — EN 300 468 Annex A string decoding. `DvbText<'a>` wraps raw wire bytes and decodes on demand (`.decode()`/`Display`/serde); `LangCode` is the 3-byte language/country newtype. Serde serializes both as decoded strings; `DvbText`-bearing structs are serialize-only.
- `demux.rs` (feature `ts`) — `SiDemux`: PID-filtered, version-gated, PAT-following section pump. Feed 188-byte TS packets, get a `SectionEvent` per *changed* section; `event.table_section()` gives an `AnyTableSection`. `section.rs`/`ts.rs` provide the underlying TS packet handling and `SectionReassembler`.
- Features: `chrono` (MJD+BCD → `DateTime<Utc>`), `ts`, `serde` — all on by default; everything must also build `--no-default-features`.

### Trait-driven dispatch (the `*Def` trait + `declare_*!` macro pattern)

Each crate's unified dispatch enum — `dvb_si` `AnyTableSection`/`AnyDescriptor`,
`dvb_t2mi` `AnyPayload` — is generated from a single declarative list (the
`declare_tables!` / `declare_descriptors!` / `declare_payloads!` macro). One line
per type produces the enum variant, the `From<T>` impl, the dispatcher arm, and a
**drift test** that pins each table_id/tag/packet_type literal to the type's
`TableDef`/`DescriptorDef`/`PayloadDef` trait const (`TABLE_ID_RANGES`/`TAG`/
`PACKET_TYPE` + a SCREAMING_SNAKE `NAME`). The list is the single source of truth,
so the dispatcher can never silently drift from the implemented set. To add a
type: implement the module + the `*Def` trait, then add one line to the macro
invocation — the integration completeness test walks the generated set
automatically.

The runnable analyzer CLI (the `dvb-tools` binary crate — `dump` / `services` /
`epg` / `pids` / `t2mi` subcommands) wires the pump → dispatch → decode story
together. All CLIs follow the workspace **CLI standard** ([`docs/CLI-STANDARD.md`](docs/CLI-STANDARD.md)):
`clap` derive, named flags (no bare positional magic numbers), auto
`--help`/`--version`. `ci-probe` (the `dvb-ci-runtime` CAM tool) follows the same
standard.

### Spec grounding (the project's defining discipline)

- Vendored spec PDFs (ETSI/ISO/ITU-T/SCTE/ATSC/DVB) + non-redistributable test media live in the **private git submodule `private/`** (repo `rust-broadcast-private`) — PDFs at `private/specs/*.pdf`, fixtures at `private/fixtures/…`. Run `git submodule update --init private` to fetch (maintainers only; public/CI clones skip it and dependent tests skip cleanly). The public `specs/` dir holds only freely-redistributable text specs (RFCs, etc.) + manifests. Syntax tables are machine-extracted into reviewable markdown in `dvb-si/docs/` by `tools/dvb-si-audit/` (deterministic pdfplumber pipeline — see its README to regenerate).
- **Every layout is cited**: module doc comments name the spec, section, and tag/table_id (e.g. `//! Network Name Descriptor — ETSI EN 300 468 §6.2.28 (tag 0x40)`). When implementing or changing a layout, read the corresponding `dvb-si/docs/` transcription first and cite it.
- **No magic numbers** outside `#[cfg(test)]`: every hex literal is a named constant or enum.
- Every field in a section's syntax appears in the parsed struct (spec fidelity).
- Fixture tests (`dvb-si/tests/`) validate against real broadcast captures; round-trip and serde round-trip tests are required for new types.

### Error conventions

Structured `thiserror` errors with context: `BufferTooShort { need, have, what }`, `InvalidDescriptor { tag, reason }`, etc. Parsers validate the tag byte and length before slicing; serializers check `OutputBufferTooSmall` first. Reserved-bit policy varies by crate and is documented at the crate root (e.g. dvb-t2mi rejects non-zero RFU bits except individual addressing).

### Spec/field-enum label convention (every public enum — #204)

Every public spec/field enum across all crates exposes a uniform label pair:

- **`pub fn name(&self) -> &'static str`** — inherent method, hand-written
  `match` arms (labels live in source, next to the variant docs; the spec token
  for known variants, `"reserved"` for the reserved/unknown arm).
- **`Display`** — generated by `broadcast_common::impl_spec_display!`, a label-free
  macro that delegates to `name()`. `impl_spec_display!(Ty)` makes `Display ==
  name()`; `impl_spec_display!(Ty, Reserved, …)` renders each named byte-bearing
  catch-all as `"{name}(0x{:02X})"` so `Display` stays lossless.

Labels are NEVER put in the macro — only in `name()`. A per-crate
`tests/label_coverage.rs` drift-guard scans `src/` and fails CI if any public
`pub enum` (minus a documented SKIP list: errors, `Any*`/tag dispatch enums,
section-kind discriminants, data-carrying ADTs) lacks a `Display`. So a **new
spec/field enum must get `name()` + `impl_spec_display!(...)`**, or be added to
that crate's SKIP list if it is genuinely not a label.

### Adding a descriptor/table (the recurring task)

Follow an existing implemented module (e.g. `descriptors/network_name.rs`) exactly: spec-cited module doc → `TAG`/length consts → borrowed struct with `#[cfg_attr(feature = "serde", …)]` (+ `serde(borrow)` on slices) → `Parse` with tag + length validation → symmetric `Serialize` → unit tests in-module + round-trip coverage. All 115 `src/descriptors/` and 31 `src/tables/` files now carry real types — no doc-only stub modules remain (the `complete-descriptors` push finished); this section documents the pattern for the next new descriptor/table, not a backlog.
