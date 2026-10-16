---
status: DRAFT
---
# The 2026-10 wave: typed ids, preserved bits, no more panicking serializers

_Planned release 2026-10; dates are stamped at sign-off (`prepare-release.py --restamp`)._

This is the largest single wave the workspace has shipped: **54 crates, all major-class** (the lockstep five go 11.1.0 -> 12.0.0, `broadcast-common` 9.4.0 -> 10.0.0, `mpeg-ts` 0.5 -> 0.6, `transmux` 0.26 -> 0.27, `multimux` 0.12 -> 0.13, ...). It is large because the workspace's epoch-purity rule says a major bump of `broadcast-common` moves every crate that depends on it, and 49 do. About 31 crates have genuine breaking changes of their own; the other 23 ship because their dependency epoch moved. The plan, with the per-crate justification, tag order and process traps, is in [`../release-plan-2026-10.md`](../release-plan-2026-10.md); gate evidence and the must-fix list are in [`../release-audit-2026-10.md`](../release-audit-2026-10.md); downstream migration (zenith, zenith-tv, axis-origin) is in [`../migration-notes-downstream.md`](../migration-notes-downstream.md).

## Why you should care in one paragraph

Three classes of silent corruption are gone. (1) A serializer can no longer **mask an over-range value into a different valid-looking one** (about 220 sites in `dvb-si`, plus every crate in the `#1129`/`#1202` sweep): it returns `FieldOverflow`. (2) A parser no longer **drops reserved, marker and stuffing bits**: tables, descriptors, PES/TS/adaptation headers, VBI data units and SCTE 104/35 structures keep them in typed fields, so what a real sender wrote re-serializes byte-identically. (3) `Serialize::to_bytes()`, which **panicked** on a value a lenient decoder had assembled from damaged wire data, is removed in favour of `try_to_bytes()`. On top of that the API stops handing out bare integers for things that are not interchangeable: a PID is a `Pid`, a track a `TrackId`, a segment a `SegmentSeq`.

## Themes

### Typed identifiers (#1159, #1188)
`mpeg_ts::pid::Pid` replaces `u16` in `mpeg-ts`, `dvb-si`, `dvb-t2mi`, `dvb-conformance`, `dvb-ci`, `dvb-ci-runtime`, `dvb-stream`, `ts-fix`, `transmux` and `media-doctor` (13-bit, `Pid::try_new` at boundaries, `UpperHex`, serde-transparent, so JSON and goldens do not change). `transmux::{TrackId, ProgramNumber}`, `media_plane::ids::{SegmentSeq, PartIndex}` and `hls_runtime::server::Generation` do the same for tracks, programs, segments and init generations. A swapped segment/part argument is a compile error. `Pid::from(u16)` is removed because it silently masked.

### Preserve, don't normalise (reserved bits, stuffing, flags)
`dvb-si` captures every `reserved`/`reserved_future_use`/`reserved_zero_future_use` bit of every table section (`SectionBits`) and descriptor; `mpeg-ts` the whole adaptation field extension and stuffing bytes; `mpeg-pes` header stuffing and ES_rate markers; `mpeg-ps` the system-header tail; `dvb-vbi` reserved and stuffing; `mp4-emsg` the FullBox flags; `cc-data` the `process_em_data_flag`; `dvb-ci` the BER `length_field` form (a CAM codes `82 00 09` for 9; captured on hardware). Each is gated by a **real-capture** byte-identity test (TSDuck corpus, Fraunhofer MPEG-H, shaka-player captions, a real FFmpeg FATE 7.1 E-AC-3 stream, DASH-IF livesim2, MP4Box/gpac `wvtt`).

### Checked serialization (#1129, #1197, #1202)
`broadcast_common::len::{fit_u8, fit_u16, fit_bits, fit_mask}` everywhere a value is narrowed; lexical guards (`serialize_mask_guard`) keep it that way.

### Standards catch-up
ETSI EN 300 468 **V1.20.1** (2038 MJD wrap rule; SAT spelling; `AudioRenderingIndication`), ETSI TS 103 752-1 DVB TA compact SCTE 35 (the old layout was misdecoded; fixed), the **Chinese AVS family** (AVS2/AVS3 video+audio descriptors, T/AI 109.x, GY/T 420-2025 ISOBMFF boxes, GY/T 270 closed caption, GY/T 422 multi-audio, GY/T 282/377/379 loudness), MPEG-H 3D audio descriptors, AV3A RTP payload, SCTE 104 audio/alternate-break operations typed from Tables 9-33..9-36.

### Real bugs found by real captures (selection)
* `ts-fix` / `compliance-probe`: PCR wrap arithmetic (`wrapping_sub % (2^33*300)` is garbage across a wrap) -> `clock33::pcr_forward_distance`.
* `scte35-splice`: `splice_command_length = 0xFFF` (SCTE 35 9.6.1) rejected; DVB TA compact messages misdecoded.
* `transmux`: E-AC-3 dependent substreams (7.1) produced no track / shifted the timeline; `trun` with no per-sample fields drove a 10 GB allocation from four overwritten bytes.
* `cc-data` / `timed-metadata` / `caption-convert`: real SCC/ATSC captures lost every caption (RCL erased memory; `process_em_data_flag = 0` rejected).
* `dvb-stream`: a `0x47` in an RTP header latched the TS decoder on misaligned bytes.
* `dvb-ci-runtime`: CXD2099 rejects `O_RDWR`; post-reset `EINVAL` window; long-form lengths misframed the link.

### Secrets and identity (#1142)
Passwords, tokens, content keys, stream keys and DTLS fingerprints are zeroized on drop where owned and compared in constant time (`subtle`); `Debug` never prints them; HTTP credentials are bound to the origin they were configured for (`multimux` pull, `webrtc-runtime` WHIP/WHEP); ICE credentials and session ids come from the OS CSPRNG; `multimux` refuses an unauthenticated ingest listener on a non-loopback address unless `allow_open_ingest` is set.

### WHIP/WHEP are real now (#1149)
`multimux` serves per-session resources (`/whip/session/<id>`, `/whep/session/<id>`), honours `DELETE`, Trickle-ICE `PATCH` and ICE restart (`If-Match: *`), on top of `webrtc-runtime::MediaTransport::restart_ice`.

### Robustness (audit T16, #1144)
One damaged unit no longer aborts a whole demux (`PsDemux`, `Fmp4Demux`, `WebmDemux`, `FlvDemux`, `RtpDepacketiser`; reported in `Media::degraded`); an RTSP frame that fails to depacketise no longer ends the ingest session (`HandleOutcome`); an RTP/TS UDP stream with RTP framing is unwrapped.

### De-duplication into `broadcast-common`
`clock33` PCR helpers and the 5-byte marker-bit timestamp, `parity` (odd-parity CEA-608 / Teletext), `len::fit_mask`: a dozen crates deleted their private copies.

## Breaking changes you will hit first

| You use | You will see | Fix |
|---|---|---|
| anything with `.to_bytes()` from `Serialize` | no method | `.try_to_bytes()?` |
| a PID | `expected Pid, found u16` | `Pid::try_new(x)?` / `.value()` |
| a track id | `expected TrackId, found u32` | `TrackId::new(n)` / `.value()` |
| a `dvb-si` table/descriptor struct literal | non-exhaustive / missing fields | `::new(..)` + field assignment |
| `Bbheader::serialize()` | `Result` | `?` |
| `ClientSession::handle_data` (rtsp-runtime) | `HandleOutcome` | `.into_result()?` |
| WHIP/WHEP `Location` | constant path gone | follow the header |
| `multimux::Config` literal | missing `allow_open_ingest` | add it / `..Default::default()` |
| EIT/TDT/TOT before 1948-08-05 | no longer decode | handle outside the codec |

## Release mechanics (summary)

50 tags (49 independent + the `v12.0.0` lockstep), pushed **one at a time** in the topological order of the plan, each waiting for the `crates-io` approval gate and the index: `broadcast-common-v10.0.0` -> the leaf tier (incl. `mp4-emsg-v0.5.0`) -> `mpeg-ts-v0.6.0` -> `v12.0.0` -> `transmux-v0.27.0` -> ... -> `multimux-v0.13.0` -> `multimux-cli-v0.11.0`. `atsc3` and the three `test-*` crates are never tagged.

## All crates in the wave

| Crate (note) | From | To |
|---|---|---|
| [`atsc3-route`](atsc3-route-0.3.0.md) | 0.2.0 | 0.3.0 |
| [`broadcast-auth`](broadcast-auth-0.5.0.md) | 0.4.0 | 0.5.0 |
| [`broadcast-common`](broadcast-common-10.0.0.md) | 9.4.0 | 10.0.0 |
| [`broadcast-hls`](broadcast-hls-0.4.0.md) | 0.3.1 | 0.4.0 |
| [`broadcast-loudness`](broadcast-loudness-0.5.0.md) | 0.4.0 | 0.5.0 |
| [`caption-convert`](caption-convert-0.3.0.md) | 0.2.0 | 0.3.0 |
| [`cc-data`](cc-data-0.7.0.md) | 0.6.0 | 0.7.0 |
| [`compliance-probe`](compliance-probe-0.4.0.md) | 0.3.0 | 0.4.0 |
| [`container-probe`](container-probe-0.2.0.md) | 0.1.1 | 0.2.0 |
| [`dvb-bbframe`](dvb-bbframe-12.0.0.md) | 11.1.0 | 12.0.0 |
| [`dvb-ci`](dvb-ci-0.10.0.md) | 0.9.1 | 0.10.0 |
| [`dvb-ci-runtime`](dvb-ci-runtime-0.18.0.md) | 0.17.2 | 0.18.0 |
| [`dvb-conformance`](dvb-conformance-12.0.0.md) | 11.1.0 | 12.0.0 |
| [`dvb-csa`](dvb-csa-0.4.0.md) | 0.3.1 | 0.4.0 |
| [`dvb-mabr`](dvb-mabr-0.3.0.md) | 0.2.0 | 0.3.0 |
| [`dvb-si`](dvb-si-12.0.0.md) | 11.1.0 | 12.0.0 |
| [`dvb-simulcrypt`](dvb-simulcrypt-0.7.0.md) | 0.6.0 | 0.7.0 |
| [`dvb-stream`](dvb-stream-0.7.0.md) | 0.6.0 | 0.7.0 |
| [`dvb-subtitle`](dvb-subtitle-0.6.0.md) | 0.5.0 | 0.6.0 |
| [`dvb-t2mi`](dvb-t2mi-12.0.0.md) | 11.1.0 | 12.0.0 |
| [`dvb-tools`](dvb-tools-12.0.0.md) | 11.1.0 | 12.0.0 |
| [`dvb-vbi`](dvb-vbi-0.5.0.md) | 0.4.2 | 0.5.0 |
| [`hls-runtime`](hls-runtime-0.9.0.md) | 0.8.0 | 0.9.0 |
| [`media-doctor`](media-doctor-0.11.0.md) | 0.10.0 | 0.11.0 |
| [`media-plane`](media-plane-0.7.0.md) | 0.6.0 | 0.7.0 |
| [`mp4-emsg`](mp4-emsg-0.5.0.md) | 0.4.1 | 0.5.0 |
| [`mpeg-pes`](mpeg-pes-0.6.0.md) | 0.5.0 | 0.6.0 |
| [`mpeg-ps`](mpeg-ps-0.6.0.md) | 0.5.0 | 0.6.0 |
| [`mpeg-ts`](mpeg-ts-0.6.0.md) | 0.5.0 | 0.6.0 |
| [`multimux`](multimux-0.13.0.md) | 0.12.0 | 0.13.0 |
| [`multimux-cli`](multimux-cli-0.11.0.md) | 0.10.0 | 0.11.0 |
| [`playout-runtime`](playout-runtime-0.3.0.md) | 0.2.1 | 0.3.0 |
| [`rdd29`](rdd29-0.5.0.md) | 0.4.0 | 0.5.0 |
| [`rist-runtime`](rist-runtime-0.4.0.md) | 0.3.0 | 0.4.0 |
| [`rmt-flute`](rmt-flute-0.7.0.md) | 0.6.0 | 0.7.0 |
| [`rtcp-packet`](rtcp-packet-0.6.0.md) | 0.5.0 | 0.6.0 |
| [`rtmp-runtime`](rtmp-runtime-0.8.0.md) | 0.7.2 | 0.8.0 |
| [`rtp-packet`](rtp-packet-0.5.0.md) | 0.4.0 | 0.5.0 |
| [`rtsp-runtime`](rtsp-runtime-0.8.0.md) | 0.7.1 | 0.8.0 |
| [`scte104`](scte104-0.6.0.md) | 0.5.0 | 0.6.0 |
| [`scte35-splice`](scte35-splice-4.0.0.md) | 3.0.1 | 4.0.0 |
| [`srt-runtime`](srt-runtime-0.6.0.md) | 0.5.0 | 0.6.0 |
| [`ssai-runtime`](ssai-runtime-0.3.0.md) | 0.2.1 | 0.3.0 |
| [`st12-1`](st12-1-0.4.0.md) | 0.3.1 | 0.4.0 |
| [`st2022`](st2022-0.2.0.md) | 0.1.0 | 0.2.0 |
| [`st291`](st291-0.5.0.md) | 0.4.1 | 0.5.0 |
| [`st337`](st337-0.4.0.md) | 0.3.1 | 0.4.0 |
| [`st377-1`](st377-1-0.5.0.md) | 0.4.0 | 0.5.0 |
| [`timed-metadata`](timed-metadata-0.7.0.md) | 0.6.1 | 0.7.0 |
| [`transmux`](transmux-0.27.0.md) | 0.26.0 | 0.27.0 |
| [`ts-fix`](ts-fix-0.7.0.md) | 0.6.0 | 0.7.0 |
| [`ttml-subtitle`](ttml-subtitle-0.4.0.md) | 0.3.0 | 0.4.0 |
| [`ule`](ule-0.5.0.md) | 0.4.1 | 0.5.0 |
| [`webrtc-runtime`](webrtc-runtime-0.4.0.md) | 0.3.0 | 0.4.0 |

Lockstep GitHub Release body: [`v12.0.0.md`](v12.0.0.md).
