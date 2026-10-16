# Downstream migration notes - the 2026-10 breaking wave

Status: DRAFT (prepared before the release; versions below are the **proposed** ones from
[`release-plan-2026-10.md`](release-plan-2026-10.md)). Derived from the real API diffs between the
last tags and `main`, and from grepping the three downstream trees (read-only) for the symbols that
change: `zenith` (`backend/`), `zenith-tv` (`ZenithMedia/rust`, `tools/stream-cut`) and `axis-origin`
(`src/`). Every site named below was found by `rg` in those trees on 2026-10-16; line numbers will
drift, symbol names will not.

## The five changes that touch everyone

| # | Change | Where it comes from | Mechanical fix |
|---|---|---|---|
| 1 | `broadcast-common` 9 -> **10**: `Serialize::to_bytes()` is gone | broadcast-common 10.0.0 | `.to_bytes()` on a `Serialize` value -> `.try_to_bytes()?` (tests: `.try_to_bytes().unwrap()`). **Only** the trait method is gone: `bytes::Bytes::to_bytes()`, `axum` `Body::collect().to_bytes()` etc. are unrelated and must not be touched. |
| 2 | PIDs are `mpeg_ts::pid::Pid`, not `u16` | mpeg-ts 0.6.0, dvb-si 12, dvb-t2mi 12, dvb-conformance 12, dvb-ci 0.10, dvb-ci-runtime 0.18, dvb-stream 0.7, ts-fix 0.7, transmux 0.27, media-doctor 0.11 | `Pid::try_new(raw)?` at an input boundary, `Pid::new(raw)` for already-masked wire bits, `pid.value()` to get the `u16` back. `From<u16> for Pid` is **removed**. `{:04X}` formatting, serde and JSON output are unchanged. |
| 3 | Track/program/segment ids are newtypes | transmux 0.27 (`TrackId`, `ProgramNumber`), media-plane 0.7 + hls-runtime 0.9 (`SegmentSeq`, `PartIndex`, `Generation`) | `TrackId::new(n)` / `.value()`; no silent `From`. Serde/Display/Debug render the bare number, so wire formats and goldens do not change. |
| 4 | dvb-si tables/descriptors are `#[non_exhaustive]` and carry reserved-bit fields | dvb-si 12.0.0 | Struct literals outside the crate stop compiling: use `PatSection::new(..)`, `PatEntry::new(..)`, `SdtService::new(..)`, `CatSection::new(..)` ... then assign the public fields you need. |
| 5 | `Cargo.toml` epochs | all | see each project's table below; run `cargo tree -i broadcast-common` and `cargo tree -i mpeg-ts` afterwards: **two versions in the output means a stale pin**. |

## zenith (`/backend`)

### Dependency bumps (workspace `backend/Cargo.toml` and the member crates)

| Crate | Pinned now | Move to |
|---|---|---|
| `broadcast-common` | 9.4.0 | 10.0.0 |
| `dvb-si` (features `yoke`, `chrono`) | 11.0.1 | 12.0.0 |
| `dvb-bbframe` / `dvb-conformance` | 11.0.1 | 12.0.0 |
| `mpeg-ts` / `mpeg-pes` | 0.5.0 | 0.6.0 |
| `dvb-ci` | 0.9.0 | 0.10.0 |
| `dvb-ci-runtime` (feature `linux`) | 0.17.1 | 0.18.0 |
| `dvb-csa` | 0.3.1 | 0.4.0 (no API change; only the `mpeg-ts` epoch) |
| `transmux` (`default-features = false`, `std`) | 0.25.1 | 0.27.0 (the 0.26.0 epoch was `rtcp-packet`-only and does not touch zenith's use) |
| `media-plane` (`std`) | 0.5.0 | 0.7.0 |
| `hls-runtime` (`std`, `tokio`) | 0.7.0 | 0.9.0 |
| `tools/regression/si-dump` | dvb-si 9.1.1 / mpeg-ts 0.1.0 | separate, stale epoch; bump together or delete |

### Code sites found

1. **Table literals in test support code** (`backend/src/test_util/ts_fixtures.rs:103,111,229,239,295,304,363`, `tests/hls_open_gop_sps_pps_test.rs:210`, `tests/assembler_bus_state_test.rs:131`): `PatSection { .. }`, `PatEntry { .. }`, `SdtService { .. }`, `SdtSection { .. }`, `CatSection { .. }`, `CaDescriptor { ca_pid, .. }` are literals and no longer compile (non_exhaustive + new `bits`/`reserved*` fields; `ca_pid`/`pid` are `Pid`).
   ```rust
   // before
   let pat = PatSection { transport_stream_id: 1, version_number: 0, current_next_indicator: true, section_number: 0, last_section_number: 0, entries: pairs.map(|&(pn, pid)| PatEntry { program_number: pn, pid }).collect() };
   // after
   let pat = PatSection::new(1, 0, true, 0, 0, pairs.map(|&(pn, pid)| PatEntry::new(pn, Pid::try_new(pid).unwrap())).collect());
   ```
   Serialized bytes are unchanged for default-constructed sections except the **AC-3 descriptor `reserved_flags` default (now `0000`, was `1111`)**; a golden that embeds a hand-built AC-3 descriptor will move.
2. **`pkt.header.pid`** (`tests/replay_integration_test.rs:117,139,140,199,219,220`, `tools/regression/si-dump/src/main.rs:38`) is a `Pid`: `pkt.header.pid == well_known::PAT.value()` -> `pkt.header.pid == well_known::PAT`; `pmt_pids.contains_key(&pkt.header.pid)` needs the map keyed by `Pid` (or `.value()`). `crates/zenith-ts/src/packet.rs:102` builds a `TsHeader { .. }` literal: `pid: Pid::new(raw)`.
3. **`T2miPump::new(t2mi_pid)`** (`crates/zenith-subsystems/src/scan/si_collect.rs:157`): `Pid::try_new(t2mi_pid)?` (the probe in `zenith-stream/src/probes/t2mi.rs` documents the same hand-off).
4. **skyfire HLS pipeline** (`crates/zenith-stream/src/skyfire_hls.rs`): `known_track_ids: Vec<u32>` (270) and `video_track_id: Option<u32>` (288) must hold `TrackId` (events now carry `TrackId`), `seg.push(track_id, sample)` (459) takes a `TrackId`, and `self.segment_writer.next_sequence_number()` (535) returns a `SegmentSeq` (`.value()` for the integer, or keep it typed into `SegmentEntry::new`). `TrackSpec::new(track_id, ..)` takes a `TrackId`.
5. **PID fields of `ts_fixtures`** (`pcr_pid: u16`, `(caid, emm_pid)` pairs): these are zenith's own signatures; only the call into dvb-si changes (`Pid::try_new`).
6. **dvb-ci-runtime**: zenith uses `Driver`, `LinuxCaDevice`, `LinuxCiDataDevice`, `CiDataDevice`, `RecordingCaDevice`, `trace`, `MmiEvent`, `Notification` (`src/bin/ci_descramble.rs`, `crates/zenith-cas/src/ci_device.rs`). None of the `Pid`-typed `ManagedService`/`ManagedCa` fields is touched, so the only compile-visible change is the `dvb-ci` `length_form` field on any APDU object zenith builds by hand (none found) and the error type: a failing device operation is now a `DeviceOpError` wrapped in `io::Error` (`err.raw_os_error()` no longer answers directly; use `DeviceOpError::errno_of(&err)`). The hardware-facing improvements arrive for free: split `O_WRONLY`/`O_RDONLY` data plane on a CXD2099 `ci0` (EINVAL on `O_RDWR`), the 12 s post-`CA_RESET` EINVAL retry, long-form `length_field` tolerance.
7. **`to_bytes()`**: three hits, **none** is `Serialize::to_bytes` (two are `axum` body-collect results in `tests/oscam_reader_stats_api_test.rs:260` and `tests/auth_endpoints_test.rs:157`, one is `CStr::to_bytes` in `zenith-stream/src/cmaf/mod.rs:59`): no change is expected. (zenith-tv and zenith already use `try_to_bytes` for codec config boxes.)
8. **`AnyDescriptor::Ca(d)`** (`crates/oscam-dvbapi/src/descriptor.rs`, `zenith-cas/src/os_cam_descrambler.rs`): `d.ca_pid` is a `Pid`; map into zenith's owned type with `.value()`.
9. **`broadcast_common::Timestamp`** (`skyfire_hls.rs`): unchanged. `Timestamp::saturating_add_nanos` is new and `checked_add_nanos` (which always saturated) is deprecated.
10. **Behaviour worth re-testing** with real streams: dvb-si now preserves reserved bits (re-serialized sections of a real sender are byte-identical; the CRC is recomputed), EIT/TDT/TOT times follow the 2038 MJD wrap rule (dates before 1948-08-05 do not decode), and serializers return `FieldOverflow` for an over-range hand-built value instead of silently masking it.

## zenith-tv (`ZenithMedia/rust`, `tools/stream-cut`)

Pins: `transmux = "0.25.6"` (stream-cut: `0.25.5`), `broadcast-common = "9"`. Move to `transmux = "0.27"`, `broadcast-common = "10"`. Add `mpeg-ts = "0.6"` only if you name `Pid` directly (it is re-exported through transmux's IR types).

Code sites (`ZenithMedia/rust/src/demux.rs`, `src/lib.rs`, `tools/stream-cut/src/main.rs`):

1. **`track_id` is `TrackId`** wherever it comes off the IR: `DemuxEvent::Sample { track_id, .. }`, `DemuxEvent::TrackRemoved { track_id, .. }` (demux.rs:157,161), `spec.track_id` (demux.rs:216), `t.spec.track_id` (stream-cut main.rs:87). The FFI structs and maps are `u32` (`TrackInfo.track_id: u32`, `joins`/`codecs`/`configs` keyed by `u32`, `rejoin(track_id: u32)`, `ZmEvent.track_id`): convert once at the boundary.
   ```rust
   // before
   DemuxEvent::TrackRemoved { track_id, .. } => { self.codecs.remove(&track_id); Some(Event::TrackRemoved(track_id)) }
   track_id: spec.track_id,
   // after
   DemuxEvent::TrackRemoved { track_id, .. } => { let id = track_id.value(); self.codecs.remove(&id); Some(Event::TrackRemoved(id)) }
   track_id: spec.track_id.value(),
   ```
   `serde_json::json!({ "track": t.spec.track_id })` in stream-cut keeps compiling and prints the same bare number (`TrackId` serializes transparently).
2. **`pid: spec.source_pid.unwrap_or(0)`** (demux.rs:223): `source_pid` is `Option<Pid>` now: `spec.source_pid.map_or(0, |p| p.value())` (FFI `pid: u16` unchanged).
3. **`config.try_to_bytes()`** (demux.rs:249-269): already the new API, nothing to do. `use broadcast_common::Serialize;` stays valid; `Package`/`Unpackage` (stream-cut) are unchanged.
4. **`CodecConfig` matches**: if any `match` names `CodecConfig::Subtitle { format }` add `, ..` (it now carries `sample_entry`). `Media` is `#[non_exhaustive]` and has `degraded`: a damaged TS/PS/fMP4/WebM/FLV input now **succeeds** with `Media::degraded` populated where it used to fail the whole demux; if `stream-cut` relied on the error as a validity gate, check `media.degraded.is_empty()`.
5. **Behaviour**: `StreamingTsDemux` now recognises an AC-3 core + dependent E-AC-3 frame as one E-AC-3 access unit (7.1 streams produce a track; `dec3` `num_dep_sub` is the dependent count), drops an orphan dependent frame instead of emitting a one-frame-long sample (timeline no longer shifts), and the three `m6-*` fixtures' E-AC-3 track gains one real frame (7 -> 8 samples). Re-run any golden sample-count test that touches E-AC-3.
6. Hostile-input bounds: `trun` with no per-sample fields can no longer drive a 10 GB allocation (#1130); ISOBMFF boxes past 4 GiB use `largesize`.

## axis-origin (`src/`)

Pins today: `multimux = "0.11"`, `transmux = "0.25"`, optional `broadcast-common = "9.4"`, `media-plane = "0.5"`. That is **two epochs behind the tree for multimux (0.12 is live) and one for transmux/media-plane**, so this migration is cumulative: move to `multimux = "0.13"`, `transmux = "0.27"`, `media-plane = "0.7"`, `broadcast-common = "10"` in one step. (The 2026-10-13 wave only forced epochs here: see [`multimux-0.12.0.md`](release-notes/multimux-0.12.0.md), [`transmux-0.26.0.md`](release-notes/transmux-0.26.0.md), [`media-plane-0.6.0.md`](release-notes/media-plane-0.6.0.md).)

Code sites:

1. **`TrackSpec::new(track_id, clock_rate, config)`** (`src/convert.rs:177,184`, argument `track_id: u32`) and **`SessionEvent::Sample { track_id: self.track_id, .. }`** (`src/vdo_source.rs:191,274,806,819`, field `track_id: u32`, constant `TRACK_ID`): both take `transmux::TrackId`.
   ```rust
   // before
   const TRACK_ID: u32 = 1;           TrackSpec::new(track_id, clock_rate, config)
   SessionEvent::Sample { track_id: self.track_id, .. }
   // after
   const TRACK_ID: TrackId = TrackId::new(1);     // `new` is const
   TrackSpec::new(track_id, clock_rate, config)
   SessionEvent::Sample { track_id: self.track_id, .. }     // media_plane::SessionEvent::Sample.track_id is a TrackId
   ```
   `media_plane::ingress::{HandshakePolicy, IngestDriver, IngestSession, ProgramId, SessionEvent}`, `media_plane::trunk::{TrunkConfig, RetentionClass}` and `media_plane::DEFAULT_MAX_PROGRAMS` keep their names; only the id fields above change. `multimux::source::{DriverProgress, advance_route}`, `multimux::{Backoff, RouteHandle, supervise_driver}` and `multimux::origin::{AppState, router}` keep their signatures (`route_channel` now returns `Option<TrackId>`; axis-origin does not call it).
2. **`broadcast_common::{Demand, Stage, Timestamp}`** (`vdo_source.rs`, `multimux-edge.rs`): unchanged. Nothing in `src/` calls `Serialize::to_bytes` (0 hits), so the 10.0.0 removal costs nothing but the version bump.
3. **Newtypes `Pid` / `TrackId`**: `axis-origin` does not use PIDs. If a future TS path is added: `Pid::try_new`. Segment/part numbers: it does not call `Trunk::part_bytes` etc., so `SegmentSeq`/`PartIndex` stay internal to multimux.
4. **`multimux::Config::allow_open_ingest`** (multimux 0.13, default `false`): axis-origin builds its **own** `Config` struct (`src/config.rs`), not `multimux::Config`; if it ever constructs `multimux::Config { .. }` as a literal it must add `allow_open_ingest` (or `..Default::default()`). The origin it serves is HLS/DASH egress only, so the open-ingest refusal does not apply. A deployment that adds an RTMP/WHIP/SRT-listener ingest on a non-loopback address must set `"allow_open_ingest": true` explicitly or give the route a `stream_key`.
5. **WHIP/WHEP `Location`** (multimux 0.13): the `201` now returns `/whip/session/<id>` / `/whep/session/<id>` instead of the constant `/whip/session`. axis-origin has no WHIP/WHEP client or route (0 hits in `src/`, `html/`, `tests/`), so no change; any browser client that hard-coded `/whip/session` must instead follow the `Location` header, and can now `DELETE` it to end the session and `PATCH` it for Trickle ICE / ICE restart (`If-Match: *`).
6. **Pull credentials** are bound to the configured source origin (a redirect to another host drops `Authorization`). Only relevant if axis-origin pulls from an authenticated HLS/DASH/Smooth source.
7. **DASH/LL-DASH MPD**: `availabilityStartTime` is the program's presentation origin (the `Trunk` mint time) and the MPD returns `503` before the first publish instead of carrying a made-up AST; players that polled the MPD before the first frame will see a `503` for a moment. `SegmentTemplate@startNumber` stays fixed at 1 (0.12 behaviour).
8. **`rtsp-runtime` 0.7 -> 0.8** is a transitive epoch (`ClientSession::handle_data` returns `HandleOutcome`); axis-origin does not use it directly.

## Suggested order of work (any of the three)

1. Bump every pin in one commit (the table above), `cargo update -p <crate>` for each, `cargo tree -i broadcast-common -i mpeg-ts -i transmux` must show exactly one version of each.
2. `cargo build --all-targets`; fix the compile errors in this order: `Serialize::to_bytes`, `Pid`, `TrackId`/`SegmentSeq`, table literals.
3. Run the tests that parse real captures; the behavioural changes (AC-3 reserved flags default, E-AC-3 dependent substreams, `Media::degraded`, 2038 MJD, `FieldOverflow`) show up there, not in the compiler.
4. Only then take the optional improvements (`Media::degraded` reporting, `try_new` at the input boundary).
