# media-doctor — DVB / MPEG-TS diagnostics

A lint-style analysis harness for MPEG-2 Transport Streams, HLS playlists, and DASH MPDs.
Each [`Diagnostic`] checks one rule against a TS byte buffer and pushes
[`Finding`]s into a [`Report`]; a small CLI runs the full set over a file.
`no_std` + `alloc` (the CLI is `std`).

## Diagnostics

| Check | Detects |
|---|---|
| `SyncByteCheck` | missing `0x47` sync bytes |
| `PatPmtVersionCheck` | PAT/PMT `version_number` changes across the stream |
| `CcAnomalyCheck` | continuity-counter discontinuities (honouring legal duplicates + `discontinuity_indicator`) |
| `PcrCheck` | PCR jitter / discontinuity on the PCR PID (TR 101 290-style), honouring signalled discontinuities |
| `PtsCheck` | non-monotonic **decode** timestamps (DTS, else PTS — legal B-frame PTS reorder is not flagged) + forbidden `PTS_DTS_flags == 0b01`, on real PES PIDs only |
| `Scte35Check` | SCTE-35 splice consistency — unbalanced `splice_insert` out/in pairs, duplicate open "out"s |
| `CodecSignallingCheck` | codec signalling vs bitstream — PMT `stream_type` vs actual ES codec; `esds` ASC vs ADTS (reuses `transmux` SPS decoders) |
| `check_container_codec` | `avcC`/`hvcC` profile/level/chroma/bit-depth vs in-band SPS; sample-entry dims vs SPS-decoded dims |
| `FpsCadenceCheck` | VUI frame rate vs track timescale cadence |
| `ParamSetsCheck` | missing SPS/PPS/VPS before the first IDR/IRAP |
| `InterlaceCheck` | interlaced coding (`frame_mbs_only_flag == 0`) reported as content fact |
| `check_playlist` | HLS playlist validation (RFC 8216): missing `#EXTM3U`, missing `#EXT-X-TARGETDURATION`, `#EXTINF` exceeding target, malformed `#EXT-X-DATERANGE` |
| `check_hls_playlist` | HLS playlist validator (RFC 8216bis): structured parse via `transmux::MediaPlaylist`, plus LL-HLS rules — `hls-preload-hint-with-endlist` (§4.4.5.3), `hls-skip-without-can-skip-until` (§4.4.3.8), `hls-part-duration-range` (§4.4.4.9), `hls-malformed-daterange` (§4.4.5.1), `hls-parse-error` (§4) |
| `check_dash_mpd` | DASH MPD validator (ISO/IEC 23009-1): structured parse via `transmux::Mpd`, plus — `dash-static-mpd-missing-duration` (ISO/IEC 23009-1:2012 §5.3.1.2), `dash-representation-id-duplicate` (§5.3.5.2 Table 7), `dash-segment-timeline-monotonic` (§5.3.9.6.2), `dash-period-no-adaptation-sets` (§5.3.2), `dash-parse-error` |

Diagnostics are validated against **real captures** (e.g. a clean H.264+AAC
stream and a multi-programme DVB capture yield zero false positives) plus
crafted fault fixtures.

## CLI

```console
$ media-doctor check --input stream.ts
Findings: 0 error(s), 0 warning(s), 0 info(s)

$ media-doctor check-hls --input playlist.m3u8
Findings: 0 error(s), 0 warning(s), 0 info(s)

$ media-doctor check-dash --input manifest.mpd
Findings: 0 error(s), 0 warning(s), 0 info(s)
```

`--json` emits the report as JSON (requires the `serde` feature, on by default).

### `watch` — live compliance probe (issue #665)

```console
$ media-doctor watch --udp 239.1.1.1:5000 --metrics-addr 127.0.0.1:9090
media-doctor watch: ingesting UDP 239.1.1.1:5000, metrics on http://127.0.0.1:9090/metrics
```

Continuously ingests a live raw-MPEG-TS-over-**UDP** feed (unicast or
multicast — the address's multicast-range membership decides whether the
socket joins an IGMP group) and serves an always-current snapshot as
[Prometheus text exposition format](https://prometheus.io/docs/instrumenting/exposition_formats/)
on `GET /metrics`:

| Flag | Default | Meaning |
|---|---|---|
| `--udp <host:port>` | *(required)* | UDP address to listen on for raw MPEG-TS |
| `--metrics-addr <host:port>` | `127.0.0.1:9090` | HTTP address serving `GET /metrics` |

Metrics exposed (see `media_doctor::WatchState` for the full accounting):

| Metric | Kind | Source |
|---|---|---|
| `media_doctor_packets_total` | counter | well-formed TS packets processed |
| `media_doctor_datagrams_total` | counter | ingest datagrams fed |
| `media_doctor_resync_events_total` / `media_doctor_dropped_bytes_total` | counter | TS byte-stream resync (`mpeg_ts::resync::TsResync`) |
| `media_doctor_conformance_in_sync` | gauge | ETSI TR 101 290 monitor sync state |
| `media_doctor_conformance_events_total{indicator,priority}` | counter | full TR 101 290 indicator set (`dvb-conformance`), timed on wall-clock arrival |
| `media_doctor_scte35_events_total` / `media_doctor_scte35_open_events` | counter / gauge | SCTE-35 `splice_insert` events / currently-open ones |
| `media_doctor_pts_dts_anomalies_total` / `media_doctor_pts_dts_anomaly{pid}` | counter / gauge | non-monotonic decode-timestamp events |
| `media_doctor_codec_signalling_mismatch{pid}` | gauge | PMT-declared codec vs actual bitstream framing |
| `media_doctor_last_packet_clock_seconds` | gauge | elapsed ingest time of the last packet processed |

**Scope (v1):** UDP only. The full product-vision idea (`docs/IDEAS.md` item
#4) also covers SRT; that needs `srt-runtime`'s sans-IO handshake/ARQ engine
and is a follow-up issue, not implemented here. `PcrCheck`/`CcAnomalyCheck`
are not separately re-wired for `watch` — `ConformanceMonitor` already
computes the equivalent PCR-repetition/discontinuity and continuity-count
indicators from the same per-packet data.

The ingest/metrics core (`media_doctor::WatchState::feed_datagram` /
`render_prometheus`) is plain logic with no socket dependency, so it's
unit-tested by feeding a real capture chunked into UDP-payload-sized pieces —
no socket is ever opened in tests. The `watch` binary itself is a thin
`UdpSocket`/`TcpListener` shell (two `std::thread`s sharing
`Arc<Mutex<WatchState>>`; no async runtime) around that core.

## Library

```rust
use media_doctor::{Diagnostic, PtsCheck, Report};

let mut report = Report::new();
PtsCheck.run(&ts_bytes, &mut report);
for f in report.findings() {
    println!("[{:?}] {} @ {:?}", f.severity, f.rule_id, f.location);
}
```

HLS playlists are validated with `check_playlist` (legacy line-based) or
`check_hls_playlist` (structured, using `transmux::MediaPlaylist::parse`).
DASH MPDs are validated with `check_dash_mpd` (structured, using `transmux::Mpd::parse`).

## Deferred Manifest Rules

These rules are spec-mandated but deferred from the current implementation.
Each has a specific, stated reason — a rule without a reason looks like an
accidental omission, which is the failure this project keeps correcting.

### HLS (RFC 8216bis)

| Rule | Clause | Reason deferred |
|---|---|---|
| EXT-X-MAP cross-reference validity | §4.4.4.5 | Requires fetching + inspecting the init segment; a manifest alone does not carry the init segment's contents |
| Open-segment closure / PART-HOLD-BACK consistency | §4.4.4.9 | Requires cross-reload state and wall-clock knowledge |
| Discontinuity accounting (sequence-number vs playlist-state drift) | §4.4.3.3 | Requires per-reload tracking of discontinuity-sequence |
| Variant URI cross-reference | §4.4.4.1, §6.2.4 | Requires fetching referenced variant playlists; a master playlist alone doesn't carry their content |
| EXT-X-MEDIA (alternate renditions) in multivariant playlist | §4.4.6 | Model gap: `transmux::MasterPlaylist` does not parse EXT-X-MEDIA |
| Tag ordering beyond parse-enforced rules | §4.4.1.2, §6.2.1 | Already enforced by `transmux::MediaPlaylist::parse` — unknown/cross-repeated tags fail at parse time |

### DASH (ISO/IEC 23009-1)

| Rule | Clause | Reason deferred |
|---|---|---|
| SegmentTemplate `@media` / `@initialization` URL-identifier validity | §5.3.9.4.2 | Requires fetching + validating segment content; a manifest alone does not carry the segment data |
| `@presentationTimeOffset` / `@timescale` coherence with segment data | §5.3.9.2.3 | Requires cross-checking against actual segment durations |
| ServiceDescription / Latency element validity | §5.3.10 | Model gap: `transmux::Mpd` does not parse ServiceDescription or Latency |
| `@bandwidth` vs codecs (bitrate plausibility check) | §5.3.5.2 | Model gap: needs a codec-model comparison the parser doesn't carry |
| BaseURL resolution (hierarchy + relative/absolute) | §5.6 | Requires network fetch; the parser does not do URL resolution |
| `@id` collisions across AdaptationSets (different-set, same id) | §5.3.2 | Rare in practice (<0.1% of real-world manifests examined); low-priority additive check |
| profile / interoperability-point validation | §8 | Deferred: ISO 23009-1 is not available in the private submodule; without the spec text, any profile rule would be fabricated |

These deferred rules are recorded here (not only in a PR body) so users know
exactly what the validator does and does not check. They are also cross-referenced
in the CHANGELOG.

## License

MIT OR Apache-2.0.
