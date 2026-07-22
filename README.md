# rust-broadcast

[![CI](https://github.com/fishloa/rust-broadcast/actions/workflows/ci.yml/badge.svg)](https://github.com/fishloa/rust-broadcast/actions)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

**Spec-grounded DVB protocol parsers and builders in Rust.** Feed a transport
stream in; get typed, decoded, serde-ready data out. Every wire layout is cited
to its ETSI / ISO clause, has a symmetric serializer, and is round-trip tested.

```text
TS (T2-MI PID) ─▶ dvb-t2mi ─▶ BBFrame ─▶ dvb-bbframe ─▶ inner TS ─▶ mpeg-ts ─▶ dvb-si ─▶ typed SI
   T2-MI pump        AnyPayload      Bbheader + up_iter        SiDemux      AnyTableSection + collect
```

Each crate is independently useful; together they decode a DVB-T2 modulator
feed all the way down to a service name string.

## Status & MSRV

| | |
|---|---|
| **Versions** | 6 lockstep core crates (`broadcast-common`, `dvb-si`, `dvb-t2mi`, `dvb-bbframe`, `dvb-conformance`, `dvb-tools`) at **8.5.0**; every other crate below is independently versioned — see its own badge for the current version |
| **MSRV** | **1.86** across the workspace (pinned in `rust-toolchain.toml`) |
| **Edition** | **2024** across the workspace |
| **`no_std`** | Most parser/builder library crates below build `#![no_std]` + `alloc` under `--no-default-features` (suitable for embedded targets with a heap) — see each crate's own docs for its exact story. `dvb-tools`, `dvb-stream`, `dvb-ci-runtime`, `multimux`, `multimux-cli` and other tokio/axum-based crates require `std` and are not embedded-suitable. |

To build a `no_std` crate for an embedded target, build it directly with `--no-default-features` rather than the whole workspace (feature unification across the workspace can otherwise mask a `std`-only call — see `docs/` for details), e.g.:

```console
$ cargo build -p dvb-si --no-default-features --locked
```

## The crates

### DVB Service Information & MPEG-2 transport

| Crate | Version | Docs | What it does |
|---|---|---|---|
| [`broadcast-common`](broadcast-common/) | [![crates.io](https://img.shields.io/crates/v/broadcast-common.svg)](https://crates.io/crates/broadcast-common) | [![docs.rs](https://img.shields.io/docsrs/broadcast-common)](https://docs.rs/broadcast-common) | Shared `Parse`/`Serialize` traits, the `mux` container-mux traits, and CRC-32/MPEG-2 that everything else builds on. |
| [`dvb-si`](dvb-si/) | [![crates.io](https://img.shields.io/crates/v/dvb-si.svg)](https://crates.io/crates/dvb-si) | [![docs.rs](https://img.shields.io/docsrs/dvb-si)](https://docs.rs/dvb-si) | ETSI EN 300 468 Service Information + MPEG-2 PSI: every table_id and descriptor, DSM-CC carousel, Annex A text, a version-gated `SiDemux`. TS framing lives in `mpeg-ts`. |
| [`dvb-t2mi`](dvb-t2mi/) | [![crates.io](https://img.shields.io/crates/v/dvb-t2mi.svg)](https://crates.io/crates/dvb-t2mi) | [![docs.rs](https://img.shields.io/docsrs/dvb-t2mi)](https://docs.rs/dvb-t2mi) | ETSI TS 102 773 DVB-T2 Modulator Interface (T2-MI): all 12 packet types + a feed-and-iterate pump. |
| [`dvb-bbframe`](dvb-bbframe/) | [![crates.io](https://img.shields.io/crates/v/dvb-bbframe.svg)](https://crates.io/crates/dvb-bbframe) | [![docs.rs](https://img.shields.io/docsrs/dvb-bbframe)](https://docs.rs/dvb-bbframe) | DVB-S2 / S2X / T2 BBFRAME headers (MATYPE/UPL/DFL/SYNCD) + user-packet extraction. |
| [`dvb-conformance`](dvb-conformance/) | [![crates.io](https://img.shields.io/crates/v/dvb-conformance.svg)](https://crates.io/crates/dvb-conformance) | [![docs.rs](https://img.shields.io/docsrs/dvb-conformance)](https://docs.rs/dvb-conformance) | ETSI TR 101 290 stream conformance monitor: Priority-1/2 + SI-repetition indicators on a caller-supplied clock. |
| [`mpeg-ts`](mpeg-ts/) | [![crates.io](https://img.shields.io/crates/v/mpeg-ts.svg)](https://crates.io/crates/mpeg-ts) | [![docs.rs](https://img.shields.io/docsrs/mpeg-ts)](https://docs.rs/mpeg-ts) | MPEG-2 TS framing (ITU-T H.222.0 / ISO/IEC 13818-1): TS packet, adaptation field, PCR, PSI section reassembly + packetisation, resync. `no_std`. **Independently versioned.** |
| [`mpeg-pes`](mpeg-pes/) | [![crates.io](https://img.shields.io/crates/v/mpeg-pes.svg)](https://crates.io/crates/mpeg-pes) | [![docs.rs](https://img.shields.io/docsrs/mpeg-pes)](https://docs.rs/mpeg-pes) | PES depacketization + PTS/DTS (ISO/IEC 13818-1 §2.4.3): `PesPacket`, 33-bit `Pts`/`Dts`, per-PID `PesAssembler`. `no_std`, depends only on `broadcast-common`. **Independently versioned.** |
| [`mpeg-ps`](mpeg-ps/) | [![crates.io](https://img.shields.io/crates/v/mpeg-ps.svg)](https://crates.io/crates/mpeg-ps) | [![docs.rs](https://img.shields.io/docsrs/mpeg-ps)](https://docs.rs/mpeg-ps) | MPEG-1/2 Program Stream (`.mpg`/`.vob`) framing (ISO/IEC 13818-1 §2.5): pack header (42-bit SCR), system header, program stream map (PSM), pack walker; PES via `mpeg-pes`. `no_std`. **Independently versioned.** |
| [`dvb-stream`](dvb-stream/) | [![crates.io](https://img.shields.io/crates/v/dvb-stream.svg)](https://crates.io/crates/dvb-stream) | [![docs.rs](https://img.shields.io/docsrs/dvb-stream)](https://docs.rs/dvb-stream) | Async/tokio stream adapters: `SectionStream` and `T2miEventStream` over any `AsyncRead` source (file, TCP, UDP multicast). **Independently versioned** (tokio MSRV moves faster than the workspace). |

### Container muxing, adaptive streaming & DPI signalling

| Crate | Version | Docs | What it does |
|---|---|---|---|
| [`transmux`](transmux/) | [![crates.io](https://img.shields.io/crates/v/transmux.svg)](https://crates.io/crates/transmux) | [![docs.rs](https://img.shields.io/docsrs/transmux)](https://docs.rs/transmux) | Any-to-any media container muxing hub (ISO/IEC 14496-12 / 13818-1 / 23009-1, RFC 8216/3550): demux TS/fMP4/PS/WebM/FLV/RTMP into one neutral IR and mux to CMAF/progressive-MP4/TS/HLS/DASH/LL-DASH/LL-HLS/Smooth, plus CENC decrypt, RTP/RTCP, IR transforms (splice/SSAI, trick-play), and a conformance validator. `no_std`+`alloc`. **Independently versioned.** |
| [`ts-fix`](ts-fix/) | [![crates.io](https://img.shields.io/crates/v/ts-fix.svg)](https://crates.io/crates/ts-fix) | [![docs.rs](https://img.shields.io/docsrs/ts-fix)](https://docs.rs/ts-fix) | MPEG-2 TS stream-conditioning CLI: continuity/PID-filter/PAT-PMT regen/stuffing/PCR-restamp repair. **Independently versioned.** |
| [`media-doctor`](media-doctor/) | [![crates.io](https://img.shields.io/crates/v/media-doctor.svg)](https://crates.io/crates/media-doctor) | [![docs.rs](https://img.shields.io/docsrs/media-doctor)](https://docs.rs/media-doctor) | Container/stream diagnostics: pluggable lint-style checks (sync, PAT/PMT versioning, CC anomalies, PCR, PTS/DTS monotonicity, SCTE-35 splice consistency) + HLS/fMP4/CMAF playlist and structural validation. **Independently versioned.** |
| [`mp4-emsg`](mp4-emsg/) | [![crates.io](https://img.shields.io/crates/v/mp4-emsg.svg)](https://crates.io/crates/mp4-emsg) | [![docs.rs](https://img.shields.io/docsrs/mp4-emsg)](https://docs.rs/mp4-emsg) | ISO BMFF / DASH Event Message Box (`emsg`): parse and build `emsg` boxes (v0 and v1) for in-band event signalling (DASH-IF, SCTE-35 inband, ID3). `no_std`. **Independently versioned.** |
| [`timed-metadata`](timed-metadata/) | [![crates.io](https://img.shields.io/crates/v/timed-metadata.svg)](https://crates.io/crates/timed-metadata) | [![docs.rs](https://img.shields.io/docsrs/timed-metadata)](https://docs.rs/timed-metadata) | Converts DPI/timed-metadata signalling between SCTE-35, HLS `EXT-X-DATERANGE` (RFC 8216 §4.4.5.1), and DASH `emsg` (SCTE 214-3): lossless round-trips, 33-bit PTS wrap-unroll via a `Timeline`. `no_std`. **Independently versioned.** |
| [`scte35-splice`](scte35-splice/) | [![crates.io](https://img.shields.io/crates/v/scte35-splice.svg)](https://crates.io/crates/scte35-splice) | [![docs.rs](https://img.shields.io/docsrs/scte35-splice)](https://docs.rs/scte35-splice) | ANSI/SCTE 35 splice information (DPI cueing): every command + splice descriptor, the segmentation assignment tables, round-trip builders. `no_std`. **Independently versioned.** |
| [`scte104`](scte104/) | [![crates.io](https://img.shields.io/crates/v/scte104.svg)](https://crates.io/crates/scte104) | [![docs.rs](https://img.shields.io/docsrs/scte104)](https://docs.rs/scte104) | ANSI/SCTE 104 2023 automation→compression DPI signalling: single/multiple operation messages + all ~20 operations (splice/time_signal/insert-descriptor/segmentation/…). `no_std`. **Independently versioned.** |

### RTSP / RTP / SRT streaming & the multimux HTTP origin hub

| Crate | Version | Docs | What it does |
|---|---|---|---|
| [`rtsp-runtime`](rtsp-runtime/) | [![crates.io](https://img.shields.io/crates/v/rtsp-runtime.svg)](https://crates.io/crates/rtsp-runtime) | [![docs.rs](https://img.shields.io/docsrs/rtsp-runtime)](https://docs.rs/rtsp-runtime) | Sans-IO RTSP 1.0 (RFC 2326) session engine: driveable client + server state machines, interleaved RTP/RTCP framing, Basic/Digest/Bearer auth (via `broadcast-auth`); optional `tokio` (+ TLS) socket adapter. **Independently versioned.** |
| [`rtp-packet`](rtp-packet/) | [![crates.io](https://img.shields.io/crates/v/rtp-packet.svg)](https://crates.io/crates/rtp-packet) | [![docs.rs](https://img.shields.io/docsrs/rtp-packet)](https://docs.rs/rtp-packet) | RTP fixed header + CSRC list + generic header extension (RFC 3550 §5.1/§5.3.1) — spec-complete parse/serialize. `no_std`. **Independently versioned.** |
| [`rtcp-packet`](rtcp-packet/) | [![crates.io](https://img.shields.io/crates/v/rtcp-packet.svg)](https://crates.io/crates/rtcp-packet) | [![docs.rs](https://img.shields.io/docsrs/rtcp-packet)](https://docs.rs/rtcp-packet) | RTCP control packets: SR/RR/SDES/BYE/APP + compound packet (RFC 3550 §6) — spec-complete parse/serialize. `no_std`. **Independently versioned.** |
| [`srt-runtime`](srt-runtime/) | [![crates.io](https://img.shields.io/crates/v/srt-runtime.svg)](https://crates.io/crates/srt-runtime) | [![docs.rs](https://img.shields.io/docsrs/srt-runtime)](https://docs.rs/srt-runtime) | SRT packet codecs + sans-IO HSv5 Caller-Listener/Rendezvous handshake state machines, ARQ, TSBPD delivery scheduling, Live/File congestion control, optional payload encryption and an optional async UDP socket adapter. `no_std` core. **Independently versioned.** |
| [`broadcast-auth`](broadcast-auth/) | [![crates.io](https://img.shields.io/crates/v/broadcast-auth.svg)](https://crates.io/crates/broadcast-auth) | [![docs.rs](https://img.shields.io/docsrs/broadcast-auth)](https://docs.rs/broadcast-auth) | Shared multi-scheme HTTP/RTSP auth: client `Credentials`/`Authenticator` (Basic/Digest/Bearer) + server `Verifier` (challenge+verify, incl. a reverse-proxy `forwarded` scheme). **Independently versioned.** |
| [`ll-hls-runtime`](ll-hls-runtime/) | [![crates.io](https://img.shields.io/crates/v/ll-hls-runtime.svg)](https://crates.io/crates/ll-hls-runtime) | [![docs.rs](https://img.shields.io/docsrs/ll-hls-runtime)](https://docs.rs/ll-hls-runtime) | Sans-IO Low-Latency HLS (RFC 8216bis) client + server engines in one crate (blocking reload, part prefetch, rolling-window origin), with an optional tokio+reqwest IO adapter. **Independently versioned.** |
| [`multimux`](multimux/) | [![crates.io](https://img.shields.io/crates/v/multimux.svg)](https://crates.io/crates/multimux) | [![docs.rs](https://img.shields.io/docsrs/multimux)](https://docs.rs/multimux) | Multi-input (RTSP/RTP/TS-UDP/TS-HTTP/HLS-pull), multi-output (LL-HLS/DASH/LL-DASH) just-in-time repackaging HTTP origin (tokio + axum), with shared output auth and an external scheme plugin registry. **Independently versioned.** |
| [`multimux-cli`](multimux-cli/) | [![crates.io](https://img.shields.io/crates/v/multimux-cli.svg)](https://crates.io/crates/multimux-cli) | [![docs.rs](https://img.shields.io/docsrs/multimux-cli)](https://docs.rs/multimux-cli) | The `multimux` CLI binary: config-driven multi-route hub, or a single-route quick start. **Independently versioned.** |

### Broadcast data carriage (captions, subtitles, ancillary, VBI, multicast)

| Crate | Version | Docs | What it does |
|---|---|---|---|
| [`cc-data`](cc-data/) | [![crates.io](https://img.shields.io/crates/v/cc-data.svg)](https://crates.io/crates/cc-data) | [![docs.rs](https://img.shields.io/docsrs/cc-data)](https://docs.rs/cc-data) | DVB closed-caption carriage: `cc_data()` (ETSI TS 101 154 Table B.9) → typed CEA-608/708 triplets + 608/708 split. `no_std`. **Independently versioned.** |
| [`dvb-subtitle`](dvb-subtitle/) | [![crates.io](https://img.shields.io/crates/v/dvb-subtitle.svg)](https://crates.io/crates/dvb-subtitle) | [![docs.rs](https://img.shields.io/docsrs/dvb-subtitle)](https://docs.rs/dvb-subtitle) | ETSI EN 300 743 DVB (bitmap) subtitling: page/region/CLUT/object/display-definition/disparity segments + 2/4/8-bit pixel-data sub-blocks, fed the PES data field. `no_std`, depends only on `broadcast-common`. **Independently versioned.** |
| [`st291`](st291/) | [![crates.io](https://img.shields.io/crates/v/st291.svg)](https://crates.io/crates/st291) | [![docs.rs](https://img.shields.io/docsrs/st291)](https://docs.rs/st291) | SMPTE ST 291-1 ancillary (ANC) data content: typed parse/serialize for its transports — ST 2038:2021 MPEG-2 TS/PES carriage (`anc_data_descriptor` + ANC data PES packet) and RFC 8331 / ST 2110-40 RTP carriage. `no_std`. **Independently versioned.** |
| [`dvb-vbi`](dvb-vbi/) | [![crates.io](https://img.shields.io/crates/v/dvb-vbi.svg)](https://crates.io/crates/dvb-vbi) | [![docs.rs](https://img.shields.io/docsrs/dvb-vbi)](https://docs.rs/dvb-vbi) | VBI data carriage in DVB (ETSI EN 301 775) — the PES data field: VPS, WSS, Closed Captioning, EBU/Inverted Teletext, and monochrome 4:2:2 luminance sample data units. `no_std`. **Independently versioned.** |
| [`ule`](ule/) | [![crates.io](https://img.shields.io/crates/v/ule.svg)](https://crates.io/crates/ule) | [![docs.rs](https://img.shields.io/docsrs/ule)](https://docs.rs/ule) | Unidirectional Lightweight Encapsulation (RFC 4326 + RFC 5163): SNDU framing, extension-header chains, and TS-packet de-fragmentation over DVB-S/T/C MPEG-2 TS. `no_std`. **Independently versioned.** |
| [`dvb-flute`](dvb-flute/) | [![crates.io](https://img.shields.io/crates/v/dvb-flute.svg)](https://crates.io/crates/dvb-flute) | [![docs.rs](https://img.shields.io/docsrs/dvb-flute)](https://docs.rs/dvb-flute) | ALC/LCT/FLUTE/NORM multicast object-delivery wire formats (RFC 5651/5775/6726/5740): LCT headers, header-extension chains, ALC + FEC Payload IDs, FLUTE EXT_FDT/EXT_CENC, and NORM messages. `no_std`. **Independently versioned.** |
| [`st12-1`](st12-1/) | [![crates.io](https://img.shields.io/crates/v/st12-1.svg)](https://crates.io/crates/st12-1) | [![docs.rs](https://img.shields.io/docsrs/st12-1)](https://docs.rs/st12-1) | SMPTE ST 12-1:2014 Linear Timecode (LTC) — the 80-bit logical LTC codeword: BCD time address, drop/color frame flags, binary groups, sync word. `no_std`. **Independently versioned.** |
| [`st337`](st337/) | [![crates.io](https://img.shields.io/crates/v/st337.svg)](https://crates.io/crates/st337) | [![docs.rs](https://img.shields.io/docsrs/st337)](https://docs.rs/st337) | SMPTE ST 337-2015 non-PCM audio/data burst-preamble framing over AES3 — spec-complete parse/serialize. `no_std`. **Independently versioned.** |
| [`rdd29`](rdd29/) | [![crates.io](https://img.shields.io/crates/v/rdd29.svg)](https://crates.io/crates/rdd29) | [![docs.rs](https://img.shields.io/docsrs/rdd29)](https://docs.rs/rdd29) | SMPTE RDD 29:2019 Dolby Atmos bitstream — frame/element framing + bed/object metadata. `no_std`. **Independently versioned.** |
| [`st377-1`](st377-1/) | [![crates.io](https://img.shields.io/crates/v/st377-1.svg)](https://crates.io/crates/st377-1) | [![docs.rs](https://img.shields.io/docsrs/st377-1)](https://docs.rs/st377-1) | SMPTE ST 377-1:2019 Material Exchange Format (MXF) — KLV framing, Partition/Primer Pack, local-set structural metadata, Random Index Pack. `no_std`. **Independently versioned.** |

### Conditional access

| Crate | Version | Docs | What it does |
|---|---|---|---|
| [`dvb-ci`](dvb-ci/) | [![crates.io](https://img.shields.io/crates/v/dvb-ci.svg)](https://crates.io/crates/dvb-ci) | [![docs.rs](https://img.shields.io/docsrs/dvb-ci)](https://docs.rs/dvb-ci) | DVB Common Interface (ETSI EN 50221): APDU/resource objects (ca_info, ca_pmt, ca_pmt_reply, application_info, …), ASN.1 length codec, SPDU/TPDU framing, and a `build_ca_pmt` builder from a dvb-si PMT. `no_std`. **Independently versioned.** |
| [`dvb-ci-runtime`](dvb-ci-runtime/) | [![crates.io](https://img.shields.io/crates/v/dvb-ci-runtime.svg)](https://crates.io/crates/dvb-ci-runtime) | [![docs.rs](https://img.shields.io/docsrs/dvb-ci-runtime)](https://docs.rs/dvb-ci-runtime) | Pure-Rust EN 50221 DVB Common Interface driver runtime: device I/O, TPDU/SPDU poll loop, and resource state machines over the `dvb-ci` codecs. **Independently versioned.** |
| [`dvb-simulcrypt`](dvb-simulcrypt/) | [![crates.io](https://img.shields.io/crates/v/dvb-simulcrypt.svg)](https://crates.io/crates/dvb-simulcrypt) | [![docs.rs](https://img.shields.io/docsrs/dvb-simulcrypt)](https://docs.rs/dvb-simulcrypt) | DVB SimulCrypt head-end CA message framing (ETSI TS 103 197): the generic TLV message structure plus the ECMG⇔SCS and EMMG/PDG⇔MUX registries. Signalling only — CW/ECM/EMM/datagram payloads stay opaque. `no_std`. **Independently versioned.** |

### Diagnostics & tooling

| Crate | Version | Docs | What it does |
|---|---|---|---|
| [`dvb-tools`](dvb-tools/) | [![crates.io](https://img.shields.io/crates/v/dvb-tools.svg)](https://crates.io/crates/dvb-tools) | [![docs.rs](https://img.shields.io/docsrs/dvb-tools)](https://docs.rs/dvb-tools) | Command-line analyzer over the family: `dump` / `services` / `epg` / `pids` / `t2mi`. |

### Deprecated re-export shims

Frozen at their last pre-rename version, kept only so old `Cargo.toml`
references keep resolving. New code should depend on the renamed crate
directly.

| Crate | Docs | Superseded by |
|---|---|---|
| [`dvb-common`](dvb-common/) | [![docs.rs](https://img.shields.io/docsrs/dvb-common)](https://docs.rs/dvb-common) | [`broadcast-common`](broadcast-common/) |
| [`dvb-pes`](dvb-pes/) | [![docs.rs](https://img.shields.io/docsrs/dvb-pes)](https://docs.rs/dvb-pes) | [`mpeg-pes`](mpeg-pes/) |
| [`dvb-cc`](dvb-cc/) | [![docs.rs](https://img.shields.io/docsrs/dvb-cc)](https://docs.rs/dvb-cc) | [`cc-data`](cc-data/) |
| [`dvb-scte35`](dvb-scte35/) | [![docs.rs](https://img.shields.io/docsrs/dvb-scte35)](https://docs.rs/dvb-scte35) | [`scte35-splice`](scte35-splice/) |
| [`dvb-emsg`](dvb-emsg/) | [![docs.rs](https://img.shields.io/docsrs/dvb-emsg)](https://docs.rs/dvb-emsg) | [`mp4-emsg`](mp4-emsg/) |
| [`dvb-ule`](dvb-ule/) | [![docs.rs](https://img.shields.io/docsrs/dvb-ule)](https://docs.rs/dvb-ule) | [`ule`](ule/) |

For GSE, see the existing [`dvb-gse`](https://crates.io/crates/dvb-gse) crate.

## Quickstart

Demux a `.ts` capture and print its SI sections — the
[`dvb-tools dump`](dvb-tools/) CLI:

```console
$ cargo run -p dvb-tools -- dump dvb-si/tests/fixtures/m6-single.ts
pid=0x0000 PROGRAM_ASSOCIATION v0 sn=0
pid=0x0064 PROGRAM_MAP v1 sn=0
-- packets=1264 sections=47 emitted=3 suppressed=44 crc_failures=0 malformed=0

$ cargo run -p dvb-tools -- dump dvb-si/tests/fixtures/m6-single.ts --json
{
  "pat": {
    "transport_stream_id": 1,
    "entries": [ { "program_number": 1025, "pid": 100 } ]
    // … (other fields elided for brevity)
  }
}
```

In code, the section-level pipeline is a feed-and-match loop:

```rust
use dvb_si::demux::SiDemux;
use dvb_si::descriptors::AnyDescriptor;
use dvb_si::tables::AnyTableSection;

let mut demux = SiDemux::builder().build();
for packet in ts_packets {                       // each aligned 188-byte packet
    for event in demux.feed(&packet) {           // changed sections only
        if let Ok(AnyTableSection::SdtSection(sdt)) = event.table_section() {
            for service in &sdt.services {
                for item in service.descriptors.iter().flatten() {
                    if let AnyDescriptor::Service(svc) = item {
                        println!("{}", svc.service_name.decode()); // Annex A → UTF-8
                    }
                }
            }
        }
    }
}
```

## DVB System Software Update (SSU) chain

`dvb-si` ships complete end-to-end support for the DVB-SSU receiver chain
(ETSI TS 102 006). Every layer is typed:

```text
NIT linkage_descriptor (type 0x0A)
  └─▶ PMT data_broadcast_id_descriptor (tag 0x66, id = 0x000A)
         └─▶ IdSelector::Ssu → SsuIdSelector  (TS 102 006 §7.1 Table 4)
               UNT (table_id 0x4B) on the signalled PID
                 └─▶ UntPlatform × N (compatibilityDescriptor + descriptors)
                       DSM-CC carousel: DSI (messageId 0x1006) + DII + DDB
                         └─▶ GroupInfoIndication  (TS 102 006 §8.1.1 Table 6)
                               ModuleReassembler → complete firmware module bytes
```

To decode an SSU stream:

1. Parse a `NitSection`; find a `linkage_descriptor` with `linkage_type = 0x0A`
   — it points to the network carrying the UNT.
2. Parse the `PmtSection` for the SSU service; find a `DataBroadcastIdDescriptor`
   with `data_broadcast_id = 0x000A`. Its `id_selector` will be
   `IdSelector::Ssu(SsuIdSelector { oui_entries, … })`.
3. The same PMT ES entry's PID carries UNT sections (`table_id 0x4B`). Parse
   `UntSection`; each `UntPlatform` describes a compatible device group with its
   own `CompatibilityDescriptor` and operational descriptors.
4. Feed the carousel PID into `SiDemux` + `DsmccSection` → `UnMessage::Dsi`.
   Decode `dsi.private_data` as `GroupInfoIndication::parse(dsi.private_data)` to
   find the update groups and their sizes.
5. Parse `UnMessage::Dii` to enumerate modules; feed `DownloadDataBlock` messages
   into `ModuleReassembler` to reconstruct complete firmware bytes.

## Why these crates

These are not "good enough to parse the common case" parsers. The defining
discipline is spec fidelity, verified several ways over:

- **Grounded in the ETSI deliverables.** The PDFs are vendored in the repo and
  their syntax tables transcribed into reviewable markdown under
  [`dvb-si/docs/`](dvb-si/docs/); every module doc cites its spec, section, and
  tag/table_id. No magic numbers — every hex literal outside tests is a named
  constant or enum.
- **Symmetric and round-trip tested — these crates *emit* as well as parse.**
  Every table and descriptor implements `Serialize`, not just `Parse`: build a
  `PatSection` / `PmtSection` / `CaDescriptor` and call `serialize_into` to get a
  complete section (CRC-32 included). Parse → serialize → parse is byte-identical,
  a hard project invariant enforced by tests. So there's no need to hand-roll PSI
  encoders.
- **Decoded, not just typed.** Spec-enumerated codes are typed enums with decoded
  names — `running_status` is a `RunningStatus`, `stream_type` a `StreamType`,
  `service_type` a `ServiceType`; content genre, parental-rating age, AC-3/E-AC-3
  (0x6A/0x7A typed descriptors), and more decode in the library, so consumers
  never re-implement an ETSI lookup table.
- **Five adversarial spec-audit rounds** against the transcriptions, plus
  fixture tests run against **real transponder captures** (e.g. a live French
  TNT / M6 HbbTV mux; a 10 s satellite capture decoding "Emission Spéciale
  Politique" out of an EIT).
- **Complete coverage.** Every allocated `table_id` in EN 300 468 V1.19.1
  Table 2 and every `descriptor_tag` in Table 12; all 12 T2-MI packet types.

## Documentation

- Per-crate front pages: [dvb-si](dvb-si/README.md) · [dvb-t2mi](dvb-t2mi/README.md) · [dvb-bbframe](dvb-bbframe/README.md) · [broadcast-common](broadcast-common/README.md) · [dvb-tools](dvb-tools/README.md) · [dvb-conformance](dvb-conformance/README.md)
- [Adding a parser crate](docs/extending.md) — how a new sibling crate (e.g. `scte35-splice`) plugs its own wire types into the existing dispatch via the runtime registries and open `*Def` traits, with zero breaking change.
- [`dvb-si` 4.0 migration guide](dvb-si/MIGRATION-4.0.md) — 3.x → 4.0 breaking changes: section parser names (`NitSection`, `SitSection`, …), `AnyTableSection`, CamelCase `TableId`, and complete multi-section table collection.
- [`dvb-si` 3.1 migration guide](dvb-si/MIGRATION-3.1.md) — 1.x / 2.x → 3.1 breaking changes (typed `DescriptorLoop`, Serialize-only serde, typed SIT, optional `yoke`) with before/after code.
- [`dvb-si` 2.0 migration guide](dvb-si/MIGRATION-2.0.md) — 1.x → 2.0 breaking changes with before/after code.
- API docs: [docs.rs/dvb-si](https://docs.rs/dvb-si) (each crate's docs.rs front page carries a runnable quickstart).

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option. Contributions are accepted under the same dual license.
