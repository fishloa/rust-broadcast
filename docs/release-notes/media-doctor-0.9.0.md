# media-doctor 0.9.0

_Released 2026-10-05._

### Added
- `mediastreamvalidator_oracle.rs` (issue #1140): two new cases validate a
  real rendered SSAI Interstitial `EXT-X-DATERANGE`
  (`ssai_runtime::playlist::InterstitialDateRange::to_tag_line`) and a real
  base `EXT-X-DATERANGE` carrying unknown attributes
  (`timed_metadata::daterange::DateRange::extra_attrs`) against Apple's
  independent HLS conformance tool — both validate clean. New dev-dependency:
  `ssai-runtime` (path, default-features = false).
- Features `metrics` (exposition) and `net` (hyper metrics server + socket2 UDP bind; implied by `cli`).
- `WatchSnapshot`, `ConformanceSample`, `PidFlag`, `WatchState::snapshot()`.
- `metrics_server` (`serve`, `channel`, `MetricsServerConfig`, `MetricsPublisher`) and `udp` (`bind_udp`, `UdpConfig`, `MulticastInterface`, `recv_buffer_size`) modules.
- `watch` flags `--udp-rcvbuf`, `--udp-reuse-addr` (default off, as before), `--udp-interface`; IPv6 multicast groups are now joined.
- Dev: `wait-timeout` in the test harness (`tests/support/bounded.rs`); `tests/no_handroll_guard.rs`; the `watch` metrics-server tests no longer reserve-then-rebind ports or sleep.

### Changed (breaking)
- **BREAKING: `check_dash_mpd` now requires the `std` feature.** It parses the
  MPD with `transmux::Mpd`, whose parser is now built on `quick-xml` (hand-rolled
  XML replaced; XML support is `std`-only in `transmux`), so the validator and
  its re-export are gated behind `std`; a `--no-default-features` build keeps
  every other check.
- **Requires the next `transmux` release.** The `length-prefix-violation`
  fix below uses `transmux::iter_length_prefixed_nals_with`, which exists only
  in `transmux`'s in-tree `[Unreleased]` — crates.io's `0.24.1` does not carry
  it. The dependency is declared the way this workspace declares every
  in-tree pre-release dependency (a path + caret version, here
  `path = "../transmux", version = "0.24"`), so `check-published-dep-consistency.py`
  is satisfied, but **`media-doctor` cannot be published until `transmux` is
  released with that API** — release prep must order the `transmux` tag first.
  No version is changed here; releases are the orchestrator's job.
- `cli::WatchArgs` gains the public fields `metrics_max_conns: usize`
  (`--metrics-max-conns`) and `metrics_io_timeout_ms: u64`
  (`--metrics-io-timeout-ms`), and the module gains the public constants
  `cli::DEFAULT_METRICS_MAX_CONNS` (32) and
  `cli::DEFAULT_METRICS_IO_TIMEOUT_MS` (5 000). `WatchArgs` is not
  `#[non_exhaustive]`, so a struct literal that names every field no longer
  compiles; construct it via clap (`WatchArgs::parse_from`) instead
  (issue #1112).
- `Location::pid` is now `u32`, not `u16`. The field carries a TS PID for
  transport-stream checks but a media **track id** for container checks, and
  ISO/IEC 14496-12 `track_ID` is a 32-bit field — `check_container_codec`
  narrowed it with `as u16`, silently aliasing every track id above 65535
  onto a lower one. Any code constructing or matching on `Location::pid`
  needs the wider type; `Location::new`'s second parameter is now `u32`
  (issue #1112).
- **BREAKING: `WatchState::render_prometheus` is removed.** The Prometheus text is now produced by `metrics-exporter-prometheus`: use `media_doctor::render_metrics(&WatchState)` (feature `metrics`) or read the typed `WatchState::snapshot()`. Differences in the exposition text for the same input (checked semantically against the golden from `main`, `tests/golden/watch/m6-single.prom`): the `# clauses: ...` comment line is no longer emitted - **loss of operator-visible information, accepted by the orchestrator on 2026-10-03 under the owner's go-ahead** (the clause is only on `ConformanceSample::clause`); a family with no series yet no longer writes bare `# HELP`/`# TYPE` lines (the exporter writes headers only together with a sample); families are separated by a blank line and ordered differently. Example, before: `# clauses: Continuity_count_error=TR 101 290 v1.4.1 Table 5.0a indicator 1.4` after the `media_doctor_conformance_events_total` series, and bare `# HELP media_doctor_pts_dts_anomaly ...` / `# TYPE ... gauge` with no samples; after: both absent. Every HELP text, TYPE and sample value is unchanged.
- The `watch` HTTP response head differs only cosmetically (hyper writes it): header names are lower-case, a `date` header is added, header order differs. Before: `Content-Type: text/plain; version=0.0.4` / `Content-Length: N` / `Connection: close`. After: `content-type: text/plain; version=0.0.4` / `connection: close` / `content-length: N` / `date: Sat, 03 Oct 2026 17:54:15 GMT`.
- `watch` no longer spawns a thread per metrics connection. Over-cap connections are now closed immediately at accept (formerly a hand-written `503`; a 503 per over-cap socket held a task and fd for `io_timeout`); the concurrency cap, the total per-connection deadline and `--metrics-max-conns` / `--metrics-io-timeout-ms` keep their meaning and defaults. A header-read timeout (same value) is now also enforced.
- `watch` renders the exposition at most every 250 ms and flushes when the feed goes quiet, instead of rendering on every scrape.
- The `cli` feature now implies `net` (and so `metrics`).

### Changed

- `Scte35Check` and `media-doctor watch` now share one `SpliceTracker` (section reassembly, `splice_insert` parse, cancel skip, open/closed state machine incl. `auto_return`) instead of two diverged trackers; findings and metrics unchanged (#1141).
- The crate's duplicated logic is consolidated onto single owners (audit
  MD-W10). `check_playlist` (the free function in `playlist.rs`) was a
  near-verbatim second copy of `check_hls_playlist` and had already drifted
  from it (the rule messages differed); it is now a thin alias of
  `check_hls_playlist`, so a rule can only be fixed once, and a test pins the
  two to identical findings. `has_adts_sync` existed as two identical private
  copies (`codec_signalling` and `watch`) and now lives once in
  `diagnostics::codec_common`. `watch`'s decode-timestamp wrap used a second
  hand-rolled `1 << 33` modulus beside the one in `pts_check`; both now use
  `broadcast_common::clock33`.
  **Behaviour change**: `check_playlist` is no longer a reduced-rules check —
  it now runs the same typed Media-Playlist validation as
  `check_hls_playlist`, so it can emit `hls-parse-error`,
  `hls-preload-hint-with-endlist`, `hls-skip-without-can-skip-until` and
  `hls-part-duration-range` findings it previously never produced. Callers
  that counted on the old four-rule subset will see more findings, not fewer
  (issue #1112).
- `media-doctor watch`'s metrics endpoint no longer serves one connection at a
  time on a single accept loop with an untimed blocking read. A TCP client
  that connected and sent nothing (a port scanner, a half-open health check)
  blocked `/metrics` for every later scraper until it gave up — reachable in
  the documented `--metrics-addr 0.0.0.0:9090` deployment, not just locally.
  Each connection is now handled on its own thread, with a
  *total* accept-to-response deadline (`--metrics-io-timeout-ms`, default
  5 000 ms) so a hung or dribbling peer cannot pin a thread for the process's
  lifetime, and at most `--metrics-max-conns` (default 32) connections are
  served at once — a connection beyond the cap is answered `503` and closed
  (issue #1112).
- `media-doctor watch` now rebuilds a program's tracked PIDs when its PMT's
  `version_number` changes, instead of only ever adding to them
  (`entry().or_insert…`). A PMT version bump is how a multiplex signals that
  a program's stream set changed (ETSI EN 300 468 §5.1), so previously a PID
  that moved from AAC to H.264 kept `EsKind::AudioAdts` and reported
  `media_doctor_codec_signalling_mismatch` forever, a removed SCTE-35 PID
  kept being tracked, and a stale decode-timestamp baseline survived a codec
  change (issue #1112).
- The `media-doctor check` CLI no longer misroutes a transport stream whose
  first bytes fail a "`0x47` at byte 0 and byte 188" sniff — a capture that
  does not start on a packet boundary (`tcpdump`/`dd` cuts), a 192-byte M2TS
  or 204-byte RS-framed stream, or a file with a leading junk byte. All of
  those were sent to the container path, which bailed at its ISOBMFF sniff
  and printed **"No issues found."** for a TS full of errors — the worst
  possible answer from a diagnostic tool. Input routing is now
  `container-probe`'s stride×phase lattice search (188/192/204/208 bands),
  and the packet stream is extracted at the matched stride before the TS
  diagnostics run. New dependency: `container-probe` (path + version, behind
  the existing `cli` feature, so the library build is unchanged)
  (issue #1112).
- `check_container_codec`'s `length-prefix-violation` check no longer assumes
  a 4-byte NAL length prefix. The prefix width comes from the track's own
  `avcC`/`hvcC` `lengthSizeMinusOne` (ISO/IEC 14496-15 §5.3.3.1.1 /
  §8.3.3.1.2), where 1-, 2- and 4-byte prefixes are all conformant — a
  conformant MP4 with `lengthSizeMinusOne = 1` previously got a false error
  on every AVC/HEVC sample, and the message no longer claims "4-byte"
  regardless of the record (issue #1112).
- `PtsCheck` and `media-doctor watch` no longer discard the payload of a
  packet carrying a TS-layer `discontinuity_indicator`. Both reset the
  per-PID PES state on such a packet and then `return`ed/`continue`d without
  feeding it — but that packet is normally the `payload_unit_start_indicator`
  start of the first PES after the break, so the first post-break access unit
  was silently dropped and the first post-break timestamp was never
  baselined or checked (the source comment claimed the opposite). Both now
  reset and fall through to feed the packet's payload (issue #1112).
- `PcrCheck` now evaluates TR 101 290 v1.4.1 Table 5.0b indicators 2.3a and
  2.3b exactly as the table defines them, on the difference between two
  consecutive PCR **values**, with no derived arrival clock at all:

  - **2.3b `PCR_discontinuity_indicator_error`** — "The difference between
    two consecutive PCR values (PCR_(i+1) - PCR_i) is outside the range of
    0…100 ms without the discontinuity_indicator set" — is an **Error**, and
    now covers a **backward** step as well as a forward one over 100 ms. The
    old code flagged only forward steps over 100 ms, so a PCR that ran
    backwards without signalling it was never reported.
  - **2.3a `PCR_repetition_error`** — "Time interval between two consecutive
    PCR values more than 100 ms" (Table 5.0b note 2: the 40 ms limitation was
    removed in 2005) — is reported at **Info**, once per PID for the worst
    step seen. A recorded file carries no arrival timing, so the PCR value
    step is the only evidence available and cannot support an Error; the
    40 ms tier the module docs used to promise does not exist in the current
    standard and is not implemented. The live `watch` path, which is fed real
    arrival times, is where an Error-severity 2.3a is evaluated.

  A step assigned to 2.3b is **never also reported as 2.3a**: a +600 s
  program splice is one discontinuity, and reporting it as both inflated the
  count and misdescribed it. Measured before this change, the old
  value-derived arrival clock flagged 20 of the 46 committed captures; the
  value-based tiers flag only the six whose real PCR spacing genuinely
  violates the definition, each confirmed by an independent re-parse
  (`media-doctor/tests/tools/max_pcr_step.py`), and the same verdict is
  cross-checked against TSDuck 3.44 for the worst case
  (`fixtures/scte35-ssai/ts/video_with_scte35_splice_insert.ts`: 260 PCRs,
  every step exactly 100 ms, no repetition error in either tool)
  (issue #1112).
- `Scte35Check` and `media-doctor watch` no longer report an auto-return
  break as unbalanced. A `splice_insert` with `out_of_network_indicator = 1`
  and `break_duration.auto_return = 1` is a *self-closing* break — the
  splicer returns after `duration` with no separate "in" cue (ANSI/SCTE 35
  §9.8.2, §9.9.2.2) — which is standard SSAI signalling, but every one of
  them produced a `scte35-unbalanced` finding and kept
  `media_doctor_scte35_open_events` climbing for the life of the process.
  The committed real canonical industry vector (`fixtures/ts/scte35-real.ts`,
  event_id `0x4800008f`) is exactly such a break; the fixture test that
  asserted the opposite held a wrong premise and now asserts the vector
  parses and is correctly *not* flagged. `watch` also no longer retains
  closed events: its per-PID map now holds one entry per *currently open*
  break instead of one per `splice_event_id` ever seen, so a long-running
  ingest cannot grow it without bound (issue #1112).
- `check_hls_playlist`'s `hls-part-duration-range` rule no longer misapplies
  RFC 8216bis §4.4.4.9. The INDEPENDENT/GAP/followed-by-GAP/final-part
  exemptions relax only the "at least 85% of the Part Target Duration"
  floor — "MUST be less than or equal to the Part Target Duration" applies
  to *every* part, so an INDEPENDENT or final part overrunning PART-TARGET
  (the common real fault, a keyframe-aligned part overrunning its target) is
  now flagged instead of silently accepted. The open (in-progress) segment's
  parts — the live edge, and the parts a client is about to fetch — are now
  checked too; previously only *closed* segments were. The rule table's
  documented severity (Warning) also disagreed with the emitted severity
  (Error); the table now says Error, and each finding names the bound it
  violated (issue #1112).
- `PatPmtVersionCheck` no longer hand-rolls its PAT/PMT walk. The old
  4-byte-stride loop over the section body read the trailing CRC_32 as a
  fourth program entry (so a PAT's own CRC bytes were registered as a PMT
  PID — on the committed `m6-single.ts` fixture that aliased PID `0x03EF`,
  and a PMT there would report a `pmt-version` finding for a table the PAT
  never declared), registered `program_number 0` (the NIT PID) as a PMT PID,
  never validated the section CRC (a corrupted section raised a spurious
  version change), compared `current_next_indicator = 0` next-generation
  sections against the current one (flip-flopping findings on a mux that
  pre-announces a version), and keyed version state on `(pid, table_id)`
  with no `table_id_extension` — so two PMTs legally sharing one PID
  (ISO/IEC 13818-1 §2.4.4.8) emitted a finding on every repetition. It now
  parses with `dvb-si`'s typed `PatSection`/`PmtSection`, validates the
  CRC-32 via `mpeg_ts::section::Section::validate_crc`, skips
  next-generation sections, keys on `(pid, table_id, table_id_extension)`,
  and attributes each finding to the TS packet index it was seen at rather
  than packet 0. Cross-checked against TSDuck 3.44's independent PAT/PMT
  analysis of `m6-single.ts` (issue #1112).
- `media-doctor/tests/integration.rs`'s PAT/PMT fixture builders computed the
  CRC-32 over the wrong byte range (skipping the 3-byte section header, which
  ISO/IEC 13818-1 Annex B includes) through a second, private CRC
  implementation; `pat_pmt_version_no_changes` used a zeroed placeholder CRC,
  so it never exercised a real section. Both now use
  `broadcast_common::crc32_mpeg2::compute` over the whole section
  (issue #1112).

### Fixed
- `watch` metrics server: the over-cap `503` is now reliably delivered on Linux.
  The refusal path discards any bytes the peer already sent before closing (an
  unread receive buffer makes Linux close with RST, which can destroy the 503
  in flight). `tests/watch_metrics_server.rs` no longer relies on a fixed
  `sleep` for the accept loop to take its slots: it proves the cap is full
  (a probe is refused and every held connection is still open). The old tests
  failed deterministically on Linux because the startup readiness probe still
  held a slot, so a held connection was refused and the over-cap one was
  accepted instead.
- `Scte35Check` no longer only inspects the conventional PID `0x01F0` for
  SCTE-35 `splice_info_section`s — it now discovers the real cue PID(s) from
  the PMT (`stream_type 0x86`), as `watch.rs` already did, falling back to
  `0x01F0` only when the stream carries no PSI at all. Any real capture whose
  cue PID isn't `0x01F0` previously got zero SCTE-35 findings. Verified
  against a real TSDuck-built fixture whose cue is on PID `0x0150` (issue
  #1046).
- `codec_common::collect_pmt_streams` now dedups by elementary PID (latest
  PMT generation wins) instead of recording one entry per PMT repetition —
  a PMT repeats roughly every 100 ms, so the previous list, and every
  `.contains()` scan over it in `codec_signalling`/`param_sets`/`interlace`,
  grew (and were scanned) once per repetition rather than once per declared
  PID, quadratic in stream length (issue #1069).
- `check_dash_mpd` now checks `Representation@id` uniqueness over the whole
  **Period** rather than each AdaptationSet separately, as ISO/IEC 23009-1
  §5.3.5.2 Table 7 requires ("shall be unique within a Period"). The
  realistic violation — the same `@id` reused by two AdaptationSets of one
  Period — was invisible to the per-AdaptationSet check. The same function's
  `SegmentTimeline` walk also treated a negative `@r` as a single repeat with
  a "not valid per spec" comment; `@r = -1` is legal and means "repeat until
  the next `S` element or the end of the Period" (§5.3.9.6.2 Table 17), so
  the following `S` element's explicit `@t` now governs instead of a guessed
  end time (issue #1112).
- `check_hls_playlist`/`check_playlist` no longer discard the structured
  parse error and no longer confuse Media and Multivariant Playlists. The
  playlist kind is now classified from the tags it carries
  (`#EXT-X-STREAM-INF`/`#EXT-X-I-FRAME-STREAM-INF` for Multivariant,
  `#EXTINF`/`#EXT-X-TARGETDURATION` for Media, RFC 8216bis
  §4.4.3.1/§4.4.4.1/§4.4.6.1) instead of by which parse happened to succeed —
  a *Media* Playlist that failed its media parse (no segments, no
  TARGETDURATION) used to fall into the multivariant branch, whose rule set
  was empty, and was reported clean. A parse failure now reports the
  parser's own message, which carries the offending line number, the line
  verbatim and the reason, replacing a bare "failed to parse" with no
  location (issue #1112).

---

Published from tag `media-doctor-v0.9.0`.
