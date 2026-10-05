# media-doctor 0.9.0

_Released 2026-10-05._

Breaking (0.x minor) release of the container and stream diagnostics crate and its `media-doctor` binary. It is mostly a correctness release: an audit (issue #1112, MD-W1 to MD-W11) found places where the tool gave the wrong answer, including printing "No issues found." for a transport stream full of errors. Most of those fixes only change what you see in a report: **you will see findings you did not see before, and a few you used to see are gone.** The API breaks are small: `check_dash_mpd` needs the `std` feature, `Location::pid` is `u32`, `WatchState::render_prometheus` is removed, and `cli::WatchArgs` gained public fields. Anyone who gates CI on media-doctor's exit status or finding counts should re-baseline.

Read together with: [transmux-0.25.0.md](transmux-0.25.0.md) (the DASH validator parses with its quick-xml-based `Mpd`), [broadcast-hls-0.3.0.md](broadcast-hls-0.3.0.md), [dvb-si-11.0.0.md](dvb-si-11.0.0.md), [mpeg-ts-0.5.0.md](mpeg-ts-0.5.0.md), [timed-metadata-0.6.0.md](timed-metadata-0.6.0.md), [scte35-splice-3.0.0.md](scte35-splice-3.0.0.md), [container-probe-0.1.1.md](container-probe-0.1.1.md).

## Breaking changes

### `check_dash_mpd` requires `std`

The validator parses the MPD with `transmux::Mpd`, whose parser is now built on `quick-xml` and is `std`-only. `media_doctor::check_dash_mpd` (and its module) are therefore gated behind the `std` feature. A `--no-default-features` build keeps every other check; default features are unchanged.

```toml
# before: check_dash_mpd available with default-features = false
media-doctor = { version = "0.8", default-features = false }
# after: enable std to keep it
media-doctor = { version = "0.9", default-features = false, features = ["std"] }
```

### `Location::pid` is `u32`

`Location::pid` carries a TS PID for transport-stream checks but a media track id for container checks, and ISO/IEC 14496-12 `track_ID` is 32 bits. `check_container_codec` narrowed it with `as u16`, aliasing every track id above 65535 onto a lower one. `Location::new(packet: usize, pid: u32)` takes the wider type, so a literal or a match on `pid` needs updating (issue #1112). `PidFlag::pid` and the `codec_common` PIDs remain `u16`.

### `WatchState::render_prometheus` is removed

The Prometheus text is now produced by `metrics-exporter-prometheus`. Use `media_doctor::render_metrics(&WatchState)` (feature `metrics`) or read the typed `WatchState::snapshot()` (new `WatchSnapshot`, `ConformanceSample`, `PidFlag`).

```rust
// 0.8
let text = state.render_prometheus();
// 0.9   (feature "metrics")
let text = media_doctor::render_metrics(&state);
```

Exposition differences for the same input, checked against the 0.8.0 golden `tests/golden/watch/m6-single.prom`: every HELP text, TYPE and sample value is unchanged, but

- the `# clauses: ...` comment line (for example `# clauses: Continuity_count_error=TR 101 290 v1.4.1 Table 5.0a indicator 1.4`) is **no longer emitted**. This loses operator-visible information and was accepted on 2026-10-03; the clause now lives only on `ConformanceSample::clause`;
- a family with no series yet no longer writes bare `# HELP`/`# TYPE` lines (the exporter writes headers only with a sample);
- families are separated by a blank line and ordered differently.

A scraper parsing the text semantically is unaffected; one grepping for the `# clauses:` line or a bare `# TYPE` is not.

### `cli::WatchArgs` has more public fields

`WatchArgs` (not `#[non_exhaustive]`) gained `metrics_max_conns: usize` (`--metrics-max-conns`, default `cli::DEFAULT_METRICS_MAX_CONNS` = 32), `metrics_io_timeout_ms: u64` (`--metrics-io-timeout-ms`, default `cli::DEFAULT_METRICS_IO_TIMEOUT_MS` = 5000), `udp_rcvbuf: Option<usize>`, `udp_reuse_addr: bool` and `udp_interface: Option<String>`. A struct literal naming every field no longer compiles; build it through clap (`WatchArgs::parse_from`). All new flags default to the previous behaviour.

### Features

`cli` now implies `net`, which implies `metrics`; `cli` also pulls the new `container-probe` dependency. Verified manifest delta against 0.8.0:

```toml
broadcast-common = "9.4"   # was 9.3
mpeg-pes = "0.5"           # was 0.4
mpeg-ts = "0.5"            # was 0.4
dvb-si = "11.0"            # was 10
dvb-conformance = "11.0"   # was 10
scte35-splice = "3.0"      # was 2.1
timed-metadata = "0.6"     # was 0.5
transmux = "0.25"          # was 0.24
broadcast-hls = "0.3"      # was 0.2
# new, optional: container-probe 0.1 (cli); metrics 0.24 + metrics-exporter-prometheus 0.18 (metrics);
# tokio, tokio-util, hyper 1, hyper-util, http-body-util, socket2 0.6 (net)
# features: metrics = ["std", ...]; net = ["metrics", ...]; cli = [..., "net"]
```

## Behaviour changes in what the tool reports

- **`check` no longer misroutes transport streams (MD-W7).** Input routing used a "`0x47` at byte 0 and byte 188" sniff; a capture not starting on a packet boundary (`tcpdump`/`dd` cuts), a 192-byte M2TS or 204-byte RS-framed stream, or a file with a leading junk byte went to the container path, which bailed at its ISOBMFF sniff and printed "No issues found." for a TS full of errors. Routing now uses `container-probe`'s stride-and-phase lattice search (188/192/204/208), and the packet stream is extracted at the matched stride before the TS diagnostics run.
- **`PcrCheck` evaluates TR 101 290 v1.4.1 Table 5.0b 2.3a and 2.3b as defined, on consecutive PCR values.** 2.3b `PCR_discontinuity_indicator_error` (a step outside 0 to 100 ms without the discontinuity indicator) is an Error and now also covers a **backward** step; before only forward steps over 100 ms were flagged, so a PCR running backwards unsignalled was never reported. 2.3a `PCR_repetition_error` (more than 100 ms between PCR values) is reported at Info, once per PID for the worst step, because a recorded file carries no arrival timing and PCR value steps cannot support an Error. The 40 ms tier the module docs used to promise does not exist in the current standard and is not implemented. A step reported as 2.3b is never also reported as 2.3a. The old value-derived arrival clock flagged 20 of the 46 committed captures; the new tiers flag the six whose real PCR spacing violates the definition, each re-parsed independently (`tests/tools/max_pcr_step.py`) and cross-checked against TSDuck 3.44 for the worst case. An Error-severity 2.3a is evaluated only on the live `watch` path, which has real arrival times.
- **`Scte35Check` and `watch` no longer report auto-return breaks as unbalanced.** A `splice_insert` with `out_of_network_indicator = 1` and `break_duration.auto_return = 1` is self-closing (ANSI/SCTE 35 §9.8.2, §9.9.2.2); every one produced a `scte35-unbalanced` finding and kept `media_doctor_scte35_open_events` climbing for the life of the process. `watch` now holds one map entry per currently open break instead of one per `splice_event_id` ever seen, so a long-running ingest cannot grow it without bound. `Scte35Check` and `watch` now share one `SpliceTracker`, replacing two diverged ones (#1141).
- **`Scte35Check` finds the real cue PID (#1046).** It inspected only PID `0x01F0`; it now discovers the cue PID(s) from the PMT (`stream_type 0x86`), falling back to `0x01F0` only when the stream carries no PSI. Captures whose cue PID differs previously got zero SCTE-35 findings.
- **`check_playlist` is no longer a reduced-rules check (MD-W10).** It was a near-verbatim second copy of `check_hls_playlist` that had drifted; it is now an alias, pinned to identical findings. It can now emit `hls-parse-error`, `hls-preload-hint-with-endlist`, `hls-skip-without-can-skip-until` and `hls-part-duration-range`, which it never produced. Callers that relied on the old four-rule subset will see more findings.
- **`hls-part-duration-range` follows RFC 8216bis §4.4.4.9.** The INDEPENDENT/GAP/followed-by-GAP/final-part exemptions relax only the "at least 85% of the Part Target Duration" floor; "less than or equal to the Part Target Duration" applies to every part, so an overrunning INDEPENDENT or final part is now flagged. The open (in-progress) segment's parts are now checked as well as closed segments. The rule table's documented severity (Warning) disagreed with the emitted severity (Error); the table now says Error and each finding names the bound violated.
- **HLS playlist kind and parse errors (#1112).** Kind is classified from the tags a playlist carries, not by which parse happened to succeed: a Media Playlist that failed its media parse used to fall into the empty multivariant rule set and be reported clean. A parse failure now reports the parser's message with the line number, the line verbatim and the reason.
- **`check_dash_mpd`.** `Representation@id` is checked for uniqueness over the whole Period (ISO/IEC 23009-1 §5.3.5.2 Table 7), not per AdaptationSet. A `SegmentTimeline` `@r = -1` is legal ("repeat until the next `S` or the end of the Period", §5.3.9.6.2 Table 17); the next `S`'s explicit `@t` now governs instead of a guessed end time.
- **`PatPmtVersionCheck` parses with `dvb-si` and validates CRC-32.** The old 4-byte-stride loop read the PAT's trailing CRC_32 as a fourth program entry (on `m6-single.ts` it aliased PID `0x03EF`), registered `program_number 0` (the NIT PID) as a PMT PID, never validated the CRC (a corrupted section raised a spurious version change), compared `current_next_indicator = 0` sections against the current one, and keyed state on `(pid, table_id)` so two PMTs legally sharing a PID emitted a finding on every repetition. It now keys on `(pid, table_id, table_id_extension)`, skips next-generation sections, and attributes findings to the TS packet index where they were seen (not packet 0). Cross-checked against TSDuck 3.44.
- **`length-prefix-violation` honours `lengthSizeMinusOne`.** The NAL length-prefix width comes from the track's own `avcC`/`hvcC` (ISO/IEC 14496-15 §5.3.3.1.1, §8.3.3.1.2), not a fixed 4 bytes. A conformant MP4 with `lengthSizeMinusOne = 1` previously got a false error on every AVC/HEVC sample. This check uses `transmux::iter_length_prefixed_nals_with`, which is new in transmux 0.25.0, so media-doctor 0.9.0 needs it.
- **`PtsCheck` and `watch` no longer drop the first post-break access unit.** A packet carrying a TS-layer `discontinuity_indicator` is normally the PUSI start of the first PES after the break; both reset the per-PID state and then returned without feeding it. They now reset and feed the payload, so the first post-break timestamp is baselined and checked.
- **`watch` follows PMT version changes.** It rebuilds a program's tracked PIDs when `version_number` changes instead of only adding. Before, a PID that moved from AAC to H.264 kept `EsKind::AudioAdts` and reported `media_doctor_codec_signalling_mismatch` forever, a removed SCTE-35 PID stayed tracked, and a stale decode-timestamp baseline survived a codec change.
- `codec_common::collect_pmt_streams` dedups by elementary PID (latest PMT wins) instead of recording one entry per PMT repetition (about every 100 ms), which made every `.contains()` scan quadratic in stream length (#1069).

## `watch` metrics server

- **No longer one connection at a time (#1112).** 0.8.0 served `/metrics` on a single accept loop with an untimed blocking read, so a TCP client that connected and sent nothing (a port scanner, a half-open health check) blocked every later scraper, reachable in the documented `--metrics-addr 0.0.0.0:9090` deployment. The server is now a `hyper` http1 server on tokio with a header-read timeout, a total per-connection deadline (`--metrics-io-timeout-ms`, default 5000) and a concurrency cap (`--metrics-max-conns`, default 32). A connection over the cap is closed immediately at accept (no `503`; holding a task and fd per refused socket for the I/O timeout was worse), and the refusal path discards bytes the peer already sent so Linux does not RST the close.
- The response head differs cosmetically because hyper writes it: header names are lower-case, a `date` header is added, order differs (`content-type: text/plain; version=0.0.4`, `connection: close`, `content-length`, `date`).
- The exposition is rendered at most every 250 ms and flushed when the feed goes quiet, instead of on every scrape.
- UDP: new flags `--udp-rcvbuf`, `--udp-reuse-addr` (default off, as before), `--udp-interface` (IPv4 address or interface index); IPv6 multicast groups are now joined. New public modules (feature `net`): `metrics_server` (`serve`, `channel`, `MetricsServerConfig`, `MetricsPublisher`) and `udp` (`bind_udp`, `UdpConfig`, `MulticastInterface`, `recv_buffer_size`).

## Other

- Test infrastructure moved to the shared `test-bounded` crate, `wait-timeout` and a no-hand-roll guard; two `mediastreamvalidator_oracle` cases validate a real rendered SSAI Interstitial `EXT-X-DATERANGE` and a base `DATERANGE` with unknown attributes against Apple's tool (issue #1140; `ssai-runtime` is a dev-dependency only). The integration tests' PAT/PMT builders now compute a real CRC-32 over the whole section; previously one used a zeroed placeholder and another computed the CRC over the wrong range.
- Diagnostics consolidated onto single owners: `has_adts_sync` lives once in `diagnostics::codec_common`, and `watch`'s decode-timestamp wrap uses `broadcast_common::clock33` instead of a second hand-rolled `1 << 33`.

MSRV 1.95.0.

---

Published from tag `media-doctor-v0.9.0`.
