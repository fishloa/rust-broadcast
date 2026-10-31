# media-doctor

[![crates.io](https://img.shields.io/crates/v/media-doctor.svg)](https://crates.io/crates/media-doctor)
[![docs.rs](https://img.shields.io/docsrs/media-doctor)](https://docs.rs/media-doctor)

**A lint-style diagnostics harness for MPEG-2 Transport Streams, fMP4/CMAF codec signalling, HLS playlists and DASH MPDs, with a CLI and a live UDP `watch` mode that serves Prometheus metrics.**

## What it does / what it does not

Each `Diagnostic` checks one rule against a TS byte buffer and pushes
`Finding`s (severity, location, machine-readable `rule_id`, message) into a
`Report`. A CLI runs the set over a file (`check`), validates a playlist
(`check-hls`) or manifest (`check-dash`), or ingests a live UDP feed and exposes
what it sees as Prometheus text (`watch`). It is for QC of captures and
packager output, CI gates (`--json`), and live health probes. Diagnostics are
validated against real captures (a clean H.264+AAC stream and a multi-programme
DVB capture give zero false positives) plus crafted fault fixtures, and the HLS
output of the workspace's packagers is cross-checked against Apple's own
`mediastreamvalidator` (macOS-only oracle test).

It does not repair streams (see `ts-fix`), grade DVB conformance by TR 101 290
indicator (see `dvb-conformance` / `compliance-probe`), decode elementary
streams, or fetch anything over the network: the HLS and DASH validators judge
only what is in the manifest text. Rules that need fetched segments or
cross-reload state are listed explicitly under "Deferred manifest rules".

## Standards supported

Editions are the ones cited in the module docs; where the specification is not
in the vendored `private/specs` set (ISO/IEC 14496-12/-15 and 23009-1) the edition is marked unverified.

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ITU-T H.222.0 / ISO/IEC 13818-1 | H.222.0 v9 (08/2023) (vendored; verified) | §2.4.3.2 sync byte; §2.4.3.3 continuity_counter and legal duplicates (`broadcast_common::ts_dup`); §2.4.3.5 PCR; §2.4.3.7 PTS/DTS and the forbidden `PTS_DTS_flags == 01`; §2.4.4 PAT/PMT versioning; Table 2-34 stream_type | Full for the listed checks | `SyncByteCheck`, `CcAnomalyCheck`, `PcrCheck`, `PtsCheck`, `PatPmtVersionCheck`. |
| ETSI TR 101 290 | V1.4.1 (vendored; verified) | PCR repetition/discontinuity style checks; full indicator set in `watch` via `dvb-conformance` | Partial | 2.4 `PCR_accuracy_error` is not computable (see `dvb-conformance`). |
| ANSI/SCTE 35 | 2023r1 (vendored; verified) | `splice_insert` out/in pairing, duplicate open "out"s (`Scte35Check`) | Partial: `splice_insert` | |
| ITU-T H.264 / H.265 | vendored (editions per `private/specs-md/INDEX.md`) | §7.4 / §E.2.1 SPS decode via `transmux`: stream_type vs actual codec, VUI frame rate, parameter sets before IDR/IRAP, interlace (`frame_mbs_only_flag`) | Partial: the fields named | Codec bitstreams are never decoded beyond headers. |
| ISO/IEC 14496-12 / 14496-15 | 14496-15:2017 as cited (not vendored; unverified) | `avcC`/`hvcC` profile/level/chroma/bit-depth vs in-band SPS; sample-entry dims vs SPS dims; no Annex B start codes in samples (`check_container_codec`) | Partial | |
| RFC 8216bis (HLS) | draft-pantos-hls-rfc8216bis-22 (vendored: `specs/ietf_draft_pantos_hls_rfc8216bis.txt`, 1 May 2026) | Structured parse via `broadcast_hls::MediaPlaylist` plus 8 rules | Partial | See below and "Deferred manifest rules". |
| ISO/IEC 23009-1 (DASH MPD) | 2012 edition as cited (ISO/IEC 23009-1 itself is not vendored; unverified) | Structured parse via `transmux::Mpd` plus 6 rules | Partial | No profile / interoperability-point rules yet. DVB-DASH profile rules are available and deferred: ETSI TS 103 285 V1.4.1 is vendored (`private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf`, profile identifiers `urn:dvb:dash:profile:dvb-dash:2014`/`2017`). The ISO/IEC 23009-1 §8 interoperability points are not vendored, so no rule is written for them. |

## Feature flags

| Feature | Default | What it enables |
|---|---|---|
| `std` | yes | `std::error::Error` impls and `check_dash_mpd` (the MPD parser, `quick-xml`, needs `std`) |
| `serde` | yes | JSON report output (`serde`, `serde_json`) |
| `metrics` | no | `render_metrics` / `publish_metrics`: the `metrics` facade and `metrics-exporter-prometheus` (implies `std`) |
| `net` | no | `metrics_server` (`hyper` http1 `GET /metrics`) and `udp` (`socket2` bind); implies `metrics` |
| `cli` | yes | The `media-doctor` binary (`clap`), including `watch`; implies `std` and `net`, and pulls in `container-probe` |

Without `std` the library is `no_std` + `alloc`.

```toml
[dependencies]
media-doctor = { version = "0.12", default-features = false } # library only
```

## Library quick start

```rust
use media_doctor::{check_hls_playlist, Diagnostic, PtsCheck, Report, SyncByteCheck};

// A real capture: the harness runs each diagnostic over the whole buffer.
let ts = std::fs::read("../fixtures/ts/h264_aac.ts").unwrap();
let mut report = Report::new();
SyncByteCheck.run(&ts, &mut report);
PtsCheck.run(&ts, &mut report);
assert!(report.findings().is_empty(), "a clean stream has no findings");

// HLS: EXTINF longer than TARGETDURATION is an error (RFC 8216bis §4.4.3.1).
let mut report = Report::new();
check_hls_playlist(
    "#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXTINF:15.0,\nseg0.ts\n#EXT-X-ENDLIST\n",
    &mut report,
);
for f in report.findings() {
    println!("[{}] {} @ {:?}: {}", f.severity.name(), f.rule_id, f.location, f.message);
}
assert!(report.findings().iter().any(|f| f.rule_id == "hls-extinf-exceeds-target"));
```

`run_all(ts, &[&dyn Diagnostic], &mut report)` runs several diagnostics in order.
`check_dash_mpd(text, &mut report)` validates an MPD (needs `std`);
`check_playlist` is an alias of `check_hls_playlist` kept for older callers.

## Diagnostics

| Check | Detects |
|---|---|
| `SyncByteCheck` | missing `0x47` sync bytes |
| `PatPmtVersionCheck` | PAT/PMT `version_number` changes across the stream |
| `CcAnomalyCheck` | continuity-counter discontinuities (honouring legal duplicates and `discontinuity_indicator`) |
| `PcrCheck` | PCR jitter / discontinuity on the PCR PID, honouring signalled discontinuities |
| `PtsCheck` | non-monotonic **decode** timestamps (DTS, else PTS: legal B-frame PTS reorder is not flagged) and forbidden `PTS_DTS_flags == 0b01`, on real PES PIDs only |
| `Scte35Check` | unbalanced `splice_insert` out/in pairs, duplicate open "out"s |
| `CodecSignallingCheck` | PMT `stream_type` vs actual ES codec; `esds` ASC vs ADTS (reuses `transmux` SPS decoders) |
| `FpsCadenceCheck` | VUI frame rate vs track timescale cadence |
| `ParamSetsCheck` | missing SPS/PPS/VPS before the first IDR/IRAP |
| `InterlaceCheck` | interlaced coding (`frame_mbs_only_flag == 0`) reported as a content fact |
| `check_container_codec` | `avcC`/`hvcC` vs in-band SPS; sample-entry dims vs SPS dims (ISOBMFF/CMAF) |
| `check_hls_playlist` (alias `check_playlist`) | HLS validator: `hls-missing-extm3u`, `hls-missing-targetduration`, `hls-extinf-exceeds-target`, `hls-malformed-daterange` (§4.4.5.1), `hls-preload-hint-with-endlist` (§4.4.5.3), `hls-skip-without-can-skip-until` (§4.4.3.8), `hls-part-duration-range` (§4.4.4.9), `hls-parse-error` (§4) |
| `check_dash_mpd` | DASH validator: `dash-static-mpd-missing-duration` (§5.3.1.2), `dash-dynamic-mpd-no-availability-start`, `dash-representation-id-duplicate` (§5.3.5.2 Table 7), `dash-segment-timeline-monotonic` (§5.3.9.6.2), `dash-period-no-adaptation-sets` (§5.3.2), `dash-parse-error` |

Public types: `Diagnostic` (trait), `Finding`, `Report`, `Severity`, `Location`
(`new`, `at_pid`, `at_track`), `run_all`, and for live monitoring `WatchState`
(`feed_datagram`, `snapshot`), `WatchSnapshot`, `ConformanceSample`, `PidFlag`.

## CLI

```text
$ media-doctor --help
DVB/MPEG-TS diagnostics harness

Usage: media-doctor <COMMAND>

Commands:
  check       Run diagnostic checks against a Transport Stream
  check-hls   Validate an HLS playlist (.m3u8)
  check-dash  Validate a DASH MPD (.mpd)
  watch       Continuously ingest a live UDP MPEG-TS feed, serving Prometheus metrics
  help        Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version

$ media-doctor check --help
Run diagnostic checks against a Transport Stream

Usage: media-doctor check [OPTIONS] --input <INPUT>

Options:
  -i, --input <INPUT>  Input Transport Stream file
      --json           Emit JSON report on stdout instead of human text
  -h, --help           Print help
```

Two runnable library examples (`std::fs`, committed fixtures, no network): `check_ts` runs the TS diagnostic harness over a capture and prints a per-rule tally; `check_playlists` lints the committed `fixtures/hls/*.m3u8` set with `check_playlist`.

```console
$ cargo run -p media-doctor --example check_ts --all-features
$ cargo run -p media-doctor --example check_playlists --all-features
```

Example invocations on committed fixtures (run from the repository root):

```console
$ media-doctor check --input fixtures/ts/h264_aac.ts
No issues found.

$ media-doctor check --input fixtures/ts/h264_aac.ts --json
{
  "findings": []
}

$ media-doctor check --input fixtures/ts/m6-single.ts      # a capture with real faults
Findings: 901 error(s), 0 warning(s), 0 info(s)
   1. [error] [cc-anomaly] packet=4 pid=0x0083 PID 0x0083: expected CC=1, got CC=0 (not a legal duplicate or signalled discontinuity)
...

$ media-doctor check-hls --input fixtures/hls/bad-extinf.m3u8
Findings: 1 error(s), 0 warning(s), 0 info(s)
   1. [error] [hls-extinf-exceeds-target] packet=4 pid=0x0000 EXTINF duration 15 (rounded to 15) exceeds TARGETDURATION 10 — §4.4.3.1

$ media-doctor check-dash --input fixtures/dash/manifest.mpd
No issues found.
```

### `watch`: live compliance probe (issue #665)

```console
$ media-doctor watch --udp 239.1.1.1:5000 --metrics-addr 127.0.0.1:9090
```

Ingests a raw MPEG-TS-over-UDP feed (unicast or multicast: the address's
multicast-range membership decides whether the socket joins a group) and serves
an always-current snapshot as Prometheus text on `GET /metrics`.

| Flag | Default | Meaning |
|---|---|---|
| `--udp <host:port>` | required | UDP address to listen on |
| `--metrics-addr <host:port>` | `127.0.0.1:9090` | HTTP address serving `GET /metrics` |
| `--metrics-max-conns <n>` | `32` | metrics connections served at once; a connection beyond the cap is closed at accept |
| `--metrics-io-timeout-ms <ms>` | `5000` | total time a metrics connection may take, so a dribbling client cannot hold a slot |
| `--udp-rcvbuf <bytes>` | OS default | requested `SO_RCVBUF` (a warning is printed if the kernel grants less) |
| `--udp-reuse-addr` | off | `SO_REUSEADDR` (lets several probes share a multicast port) |
| `--udp-interface <ipv4\|index>` | OS choice | interface for the multicast join: IPv4 address (IPv4 groups) or interface index (IPv6 groups) |

| Metric | Kind | Source |
|---|---|---|
| `media_doctor_packets_total` | counter | well-formed TS packets processed |
| `media_doctor_datagrams_total` | counter | datagrams fed |
| `media_doctor_resync_events_total` / `media_doctor_dropped_bytes_total` | counter | TS resync (`mpeg_ts::resync::TsResync`) |
| `media_doctor_conformance_in_sync` | gauge | TR 101 290 monitor sync state |
| `media_doctor_conformance_events_total{indicator,priority}` | counter | full TR 101 290 indicator set (`dvb-conformance`), timed on wall-clock arrival |
| `media_doctor_scte35_events_total` / `media_doctor_scte35_open_events` | counter / gauge | `splice_insert` events / currently open |
| `media_doctor_pts_dts_anomalies_total` / `media_doctor_pts_dts_anomaly{pid}` | counter / gauge | non-monotonic decode timestamps |
| `media_doctor_codec_signalling_mismatch{pid}` | gauge | PMT-declared codec vs bitstream framing |
| `media_doctor_last_packet_clock_seconds` | gauge | elapsed ingest time of the last packet |

Scope: UDP only (SRT would need `srt-runtime`'s engine and is a follow-up).
`PcrCheck`/`CcAnomalyCheck` are not re-wired for `watch`: `ConformanceMonitor`
already computes the equivalent indicators from the same per-packet data. The
ingest/metrics core (`WatchState::feed_datagram` / `snapshot`) has no socket
dependency, so it is tested by feeding a real capture in UDP-sized chunks. The
text is produced by `metrics-exporter-prometheus` (`render_metrics`), the HTTP
side is `hyper` (`metrics_server`) and the bind is `socket2` (`udp`); the body is
re-rendered at most every 250 ms and once more when the feed goes quiet.

## Deferred manifest rules

Each spec-mandated rule not implemented has a stated reason, so an absence is
never mistaken for an oversight. Categories: **source not available** (spec text
not vendored, so any rule would be fabricated; vendored sources with deferred rules are marked "deferred"), **information not in manifest**
(needs fetched segments or cross-reload state), **model gap** (the `transmux`
parser does not carry the field).

### HLS (RFC 8216bis)

| Rule | Clause | Reason deferred |
|---|---|---|
| EXT-X-MAP cross-reference validity | §4.4.4.5 | Information not in manifest: needs the init segment |
| Open-segment closure / PART-HOLD-BACK consistency | §4.4.4.9 | Information not in manifest: cross-reload state and wall-clock |
| Discontinuity accounting | §4.4.3.3 | Information not in manifest: per-reload tracking |
| Variant URI cross-reference (master to media) | §4.4.4.1, §6.2.4 | Information not in manifest: needs referenced playlists |
| EXT-X-MEDIA (alternate renditions) in a multivariant playlist | §4.4.6 | Model gap: `transmux::MasterPlaylist` does not parse EXT-X-MEDIA |

### DASH (ISO/IEC 23009-1)

| Rule | Clause | Reason deferred |
|---|---|---|
| SegmentTemplate `@media` / `@initialization` URL-identifier validity | §5.3.9.4.2 | Information not in manifest |
| `@presentationTimeOffset` / `@timescale` coherence with segment data | §5.3.9.2.3 | Information not in manifest |
| ServiceDescription / Latency validity | §5.3.10 | Model gap: `transmux::Mpd` does not parse them |
| `@bandwidth` vs codecs plausibility | §5.3.5.2 | Model gap |
| BaseURL resolution | §5.6 | Information not in manifest: network fetch |
| `@id` collisions across AdaptationSets | §5.3.2 | Rare in practice; low-priority additive check |
| Profile / interoperability-point validation | §8 | ISO 23009-1 §8 interoperability points: source not available (not vendored). DVB-DASH profile rules (ETSI TS 103 285 V1.4.1, vendored): deferred |

### Not spec requirements (so not deferred rules)

`EXT-X-VERSION` position: RFC 8216 §4.3.1.2 says VERSION "applies to the entire
Playlist file" with no position requirement (unlike `EXT-X-MEDIA-SEQUENCE` and
`EXT-X-DISCONTINUITY-SEQUENCE`, which must precede the first segment). A
validator that flags a conformant playlist is worse than none.

## Design notes

- **Findings, not panics.** Every rule has a stable `rule_id`; malformed input is
  a finding (`hls-parse-error`, `dash-parse-error`), never a crash.
- **One reading of the spec.** Legal duplicates are `broadcast_common::ts_dup`
  (shared with `dvb-conformance` and `ts-fix`, issue #956); PCR wrap arithmetic is
  `clock33` (`tests/clock33_guard.rs`); HLS syntax is `broadcast-hls`.
- **Independent oracle.** `tests/mediastreamvalidator_oracle.rs` (macOS, skips
  loudly elsewhere) feeds packager output to Apple's `mediastreamvalidator`.
- **Allocation-bounded.** `CcAnomalyCheck` keeps a fixed 188-byte array per PID,
  so a clean stream is O(PIDs) allocations (`tests/cc_anomaly_allocs.rs`).
- `no_std` + `alloc` library; the CLI, `net` and `metrics` features are std-only.
  Public enums carry `name()` and `Display`.

## Related crates and license

Depends on `broadcast-common`, `mpeg-ts`, `transmux`, `broadcast-hls`,
`dvb-conformance` (and `container-probe` for the CLI). Used by `compliance-probe`
for its structural checks. Independently versioned. MSRV 1.95.0. Licensed under
MIT OR Apache-2.0.
