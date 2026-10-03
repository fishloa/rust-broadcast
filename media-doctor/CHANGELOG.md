# Changelog

## [Unreleased]

### Added
- `mediastreamvalidator_oracle.rs` (issue #1140): two new cases validate a
  real rendered SSAI Interstitial `EXT-X-DATERANGE`
  (`ssai_runtime::playlist::InterstitialDateRange::to_tag_line`) and a real
  base `EXT-X-DATERANGE` carrying unknown attributes
  (`timed_metadata::daterange::DateRange::extra_attrs`) against Apple's
  independent HLS conformance tool — both validate clean. New dev-dependency:
  `ssai-runtime` (path, default-features = false).

### Changed (breaking)
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

## [0.8.0] - 2026-08-11

### Fixed
- **`cc-anomaly` (`CcAnomalyCheck`) under-enforced ITU-T H.222.0 /
  ISO/IEC 13818-1 §2.4.3.3's "legal duplicate" rule in two ways**, both now
  delegated to the new shared `broadcast_common::ts_dup`
  (`check_duplicate`) — the same primitive `dvb-conformance` was fixed to
  use for issue #956:
  - **Byte-identity was payload-only.** The check compared only the
    elementary-stream payload slice between a same-CC pair, so a packet
    whose adaptation-field content changed (e.g. `splice_countdown` or
    OPCR) while the payload stayed identical was wrongly accepted as a
    legal duplicate. Per §2.4.3.3 ("each byte of the original packet shall
    be duplicated, with the exception that in the program clock reference
    fields... a valid value shall be encoded"), only the PCR field is
    exempt from byte-for-byte identity.
  - **"Two, and only two" was never enforced.** An unbounded run of
    byte-identical (PCR excepted) repeats on the same CC was silently
    accepted forever; the spec permits exactly one repeat, and a third
    consecutive one is itself flagged.
  - **Behaviour change**: on the committed `m6-duplicate.ts` fixture, the
    strict byte-identity rule and third-repeat rule do not change the
    finding count (879, unchanged from `m6-single.ts` — this fixture's
    duplicate packets are all genuinely legal under the strict rule and
    contain no third-repeat runs, confirmed by an independent oracle added
    to `tests/integration.rs` that recomputes legal/illegal-repeat counts
    directly via `broadcast_common::ts_dup`), so the two fixture tests'
    bounds tightened to exact counts but did not move. A stream that DOES
    carry an AF-body-only difference or a third consecutive repeat will
    now report additional `cc-anomaly` findings that this check previously
    missed — confirmed with synthetic packets exercising both gaps.
  - Also corrects a stale doc-comment on the fixture test
    (`cc_anomaly_m6_duplicate_legal_dups_not_flagged`), which claimed
    `m6-duplicate.ts` has "4 true legal duplicates"; the correct count
    under the spec's byte-identity rule is 5 (matching `ts-fix`'s existing,
    correct count on the same fixture).
- `pts_check`'s wrap-aware backward-jump delta now delegates to
  `broadcast_common::clock33::wrapping_forward_distance` — the shared owner
  of this math (a duplication-audit consolidation with `timed-metadata`,
  `transmux`, `compliance-probe`, each of which previously hand-rolled the
  same modular-distance formula). Identical computation, no behaviour
  change; no public API change (internal only).
- **Version bumped 0.7.0 -> 0.8.0 for epoch purity.** In-tree `media-doctor`
  0.7.0 requires `transmux ^0.23`, but the *published* 0.7.0 requires
  `^0.22`; the caret epoch had moved without a version bump. A published
  bucket spanning two epochs breaks consumers of both lines (#858), so the
  version moves instead. Caught by
  `tools/check-published-dep-consistency.py` when `compliance-probe` became
  the first in-tree consumer.
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
All notable changes to `media-doctor` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.7.0] - 2026-08-02

### Changed
- `hls-part-duration-range` severity raised from Warning to Error
  (RFC 8216bis §4.4.4.9 MUST: the 85% partial-segment duration floor is a
  MUST, not a SHOULD). Validator exceptions for INDEPENDENT=YES, GAP=YES,
  immediately-before-GAP, and final-part-of-segment are unchanged.

### Added
- `mediastreamvalidator_oracle.rs` test harness (issue #870): renders every
  HLS playlist shape the origin can produce and validates each with Apple's
  own `mediastreamvalidator` — a genuinely independent oracle, unlike
  validating our renderer against our own `check_hls_playlist` (both encode
  the same reading of the spec, so a shared misreading passes both). macOS-
  only; skips loudly (prints why) when the binary isn't on `PATH`, so it is
  a no-op on Linux/CI. Calibrated against the RFC's own §9 example
  playlists first (`fixtures/hls/spec/`) and proven to bite against a
  deliberately malformed playlist. Caught and fixed a real gap in
  `transmux`'s CMAF-HLS `HlsPackager`: no `#EXT-X-MAP` tag was emitted at
  all (see `transmux`'s own CHANGELOG). Wired into the `golden-gate` CI job
  (non-blocking, same posture as the existing `ffprobe` check) and
  documented as a local command in `CLAUDE.md`.

### Changed
- `check_hls_playlist`'s structured parse layer now uses `broadcast-hls`
  directly instead of reaching through `transmux` for it (issue #878) — this
  crate no longer needs `transmux` solely to parse an M3U8 playlist (it
  still depends on `transmux` for container/codec diagnostics). No public
  API or behaviour change: `check_hls_playlist`/`check_playlist` still take
  `&str` and never exposed the playlist types themselves.

## [0.6.1] - 2026-07-30

### Fixed
- Floor `mpeg-ts` to `0.3.1`. The `^0.3` bucket also contains 0.3.0, which is
  built against `broadcast-common` 8, so a consumer could resolve two
  `broadcast-common` majors into one graph and hit trait-resolution errors
  pointing at this crate's internals (#858).
- Floor `timed-metadata` to `0.4.1`. The `^0.4` bucket also contains 0.4.0,
  which is built against `broadcast-common` 8, creating the same split-bucket
  risk (#858).

## [0.6.0] - 2026-07-30

### Changed (Breaking)
- `cli::Cli` now carries `#[non_exhaustive]` (issue #806's non_exhaustive
  drift-guard audit). Not expected to affect real consumers (this is the
  binary's own top-level subcommand enum), but is technically a public API
  change since `cli` is a public module.

### Added
- `tests/non_exhaustive_coverage.rs` drift guard (issue #806).
- HLS manifest validator `check_hls_playlist()` (issue #756) — structured
  validation via `transmux::MediaPlaylist::parse`, plus LL-HLS rules:
  `hls-preload-hint-with-endlist` (RFC 8216bis §4.4.5.3),
  `hls-skip-without-can-skip-until` (§4.4.3.8),
  `hls-part-duration-range` (§4.4.4.9), `hls-malformed-daterange` (§4.4.5.1),
  `hls-parse-error` (§4).
  - 12 manifest rules are spec-mandated but **deferred** with stated reasons
    (see [`README.md §Deferred Manifest Rules`](README.md#deferred-manifest-rules)).
    The list was previously recorded only in the PR body and is now permanently
    documented there. `hls-version-tag-after-segments` is explicitly recorded
    as **not a spec requirement** — RFC 8216 imposes no position constraint on
    `#EXT-X-VERSION` (unlike MEDIA-SEQUENCE and DISCONTINUITY-SEQUENCE, which
    explicitly require position-before-first-segment).
- DASH MPD validator `check_dash_mpd()` (issue #756) — structured validation
  via `transmux::Mpd::parse`: `dash-static-mpd-missing-duration` (ISO/IEC
  23009-1:2012 §5.3.1.2 Table 3, CM), `dash-representation-id-duplicate`
  (§5.3.5.2 Table 7), `dash-segment-timeline-monotonic` (§5.3.9.6.2),
  `dash-period-no-adaptation-sets` (§5.3.2), `dash-parse-error`.
  - Deleted `dash-bandwidth-mismatch` before release: it was a heuristic
    not grounded in any ISO/IEC 23009-1 clause and produced false positives
    on standard ABR ladders (reviewer-confirmed against
    `dash.akamaized.net/akamai/bbb_30fps/bbb_30fps.mpd`).
- CLI subcommands `check-hls` and `check-dash` (issue #756).

## [0.5.0] - 2026-07-29

### Changed (BREAKING)
- **Requires `broadcast-common` 9** (issue #819). No functional or API change of
  this crate's own.

  Staying on `broadcast-common` 8 was not neutral: this crate's types implement
  `Parse`/`Serialize` from whichever major it links, so a consumer that used it
  alongside a 9-based crate (`transmux` 0.20, `dvb-si` 9, …) got **both majors
  in one graph**, and the trait methods resolved against the wrong one —
  surfacing as `no method named to_bytes found` / `no function named parse
  found` on types that plainly have them, with the compiler pointing at
  `broadcast-common-8.x/src/traits.rs`.

  The 9.0.0 wave originally shipped only the crates needed to publish
  `transmux`/`media-plane`/`multimux`, on the reasoning that everything else
  stayed coherent on its own 8 line. That reasoning was wrong: these crates
  exist to be composed, and the breakage only appears in a consumer that mixes
  them.

### Changed (originally drafted as 0.4.3, never separately published — crates.io jumps 0.4.2 → 0.5.0)
- Widen the `transmux` dependency to `0.20` (was `0.18`), picking up
  transmux's media-plane IR changes: `Sample.data` is now `bytes::Bytes`
  and the IR types moved into a `transmux::ir` module. media-doctor's own
  public API is unchanged (no transmux type crosses its boundary), so this
  remains a patch release.

## [0.4.2] - 2026-07-21
### Changed
- Widen the `transmux` dependency to `0.18` (was `0.17`) and the internal
  `mpeg-ts` dependency to `0.3` (was `0.2`; issue #663) — dependency-floor
  bumps only, no functional change to `media-doctor`. Internal test helpers
  updated to `mpeg_ts::mux::SectionPacketiser`/`packetise` (the `mpeg-ts` 0.3
  British-spelling rename).

## [0.4.1] - 2026-07-14
### Changed
- Widen the `transmux` dependency to `0.16` (was `0.15`): transmux 0.16.0 adds
  the CENC/CBCS encrypt path (issue #564) and makes one breaking struct-literal
  change (`dash::ContentProtectionSystem` gained a `pssh` field). media-doctor's
  own code is unchanged; this is a dependency-floor bump only.

## [0.4.0] - 2026-07-12
### Added
- `media-doctor watch` — a live, continuous compliance probe (issue #665,
  `docs/IDEAS.md` item #4): ingests a raw MPEG-TS feed over **UDP**
  (`--udp <host:port>`, unicast or multicast — auto-joins the IPv4 multicast
  group when the address is in range) and serves an accumulated snapshot as
  Prometheus text exposition format on `GET /metrics` (`--metrics-addr`,
  default `127.0.0.1:9090`).
  - **Scope note**: this release is UDP-only. The full product-vision idea
    also covers SRT; SRT ingest needs `srt-runtime`'s sans-IO handshake/ARQ
    engine and is left as a follow-up issue, not implemented here.
  - New dependency on `dvb-conformance`: every TS packet is fed to
    `ConformanceMonitor`, exposing the full ETSI TR 101 290 indicator set
    (`media_doctor_conformance_events_total{indicator=...,priority=...}`,
    `media_doctor_conformance_in_sync`), timed against wall-clock arrival
    time rather than stream-embedded PCR.
  - PMT-declared SCTE-35/H.264/HEVC/AAC-ADTS PIDs are discovered dynamically
    (via `dvb-si`'s `SiDemux`, not a fixed PID), feeding incremental
    SCTE-35 `splice_insert` open/closed tracking
    (`media_doctor_scte35_events_total`, `media_doctor_scte35_open_events`),
    decode-timestamp (DTS, else PTS) backward-jump detection
    (`media_doctor_pts_dts_anomalies_total`,
    `media_doctor_pts_dts_anomaly{pid=...}`), and declared-codec-vs-bitstream
    framing mismatch (`media_doctor_codec_signalling_mismatch{pid=...}`) —
    the same checks as `Scte35Check`/`PtsCheck`/`CodecSignallingCheck`,
    restructured to hold state across packets instead of scanning a whole
    buffer. `PcrCheck`/`CcAnomalyCheck` are intentionally not re-wired
    separately: `ConformanceMonitor` already computes the equivalent
    PCR-repetition/discontinuity and continuity-count indicators from the
    same per-packet data.
  - The ingest/metrics core (`media_doctor::WatchState`) is plain
    `no_std`+`alloc` logic with no socket dependency — `feed_datagram` takes
    a raw byte slice and `render_prometheus` renders the current snapshot,
    both unit-tested directly against a real capture
    (`fixtures/ts/m6-single.ts`) chunked into UDP-payload-sized pieces, with
    no socket opened. The `cli`-gated binary is a thin `UdpSocket`/
    `TcpListener` shell (two `std::thread`s sharing `Arc<Mutex<WatchState>>`
    — no async runtime) around this core.
  - Datagrams need not be 188-byte-aligned: `mpeg_ts::resync::TsResync`
    recovers sync-byte-aligned TS packets from whatever bytes are actually
    present, buffering partial packets across calls.
  - Prometheus exposition is hand-rolled (`# HELP`/`# TYPE` + label lines) —
    no `prometheus` crate dependency, matching this workspace's
    dependency-light CLI ethos; the format is simple enough that a small
    formatter is less code than a crate integration.

## [0.3.0] - 2026-07-04
### Added
- Codec-level signalling-vs-bitstream cross-validation checks (issue #567), reusing
  `transmux`'s SPS/NAL/ADTS decoders — no duplicated parsing:
  - `CodecSignallingCheck`: flags a PMT-declared H.264/HEVC/AAC-ADTS PID
    (`stream_type` `0x1B`/`0x24`/`0x0F`) whose elementary stream never once looks
    like that codec's framing (no Annex B NAL / no ADTS sync anywhere) —
    `codec-signalling-mismatch`.
  - `FpsCadenceCheck`: flags an AVC/HEVC track whose VUI-declared frame rate
    disagrees (>10%) with the measured PES-timestamp sample cadence —
    `fps-cadence-mismatch`.
  - `ParamSetsCheck`: flags an IDR (AVC) / IRAP (HEVC) access unit that appears
    on the wire before its PID's SPS+PPS have been observed —
    `missing-parameter-sets`.
  - `InterlaceCheck`: surfaces AVC `frame_mbs_only_flag == 0` (interlaced coding
    tools) — `avc-interlaced-content` (Info; TS/PMT signalling carries no
    progressive/interlace container claim to compare against).
  - `check_container_codec` — a new ISOBMFF/CMAF codec-level check (fragmented via
    `Fmp4Demux`, progressive via `ProgressiveDemux`) for MP4/CMAF input: `avcC`/
    `hvcC` profile/level/chroma vs the record's own embedded SPS
    (`avcc-sps-mismatch`/`hvcc-sps-mismatch`), sample-entry `width`/`height` vs the
    SPS-decoded coded dimensions (`container-sps-dimension-mismatch`), AVC
    `frame_mbs_only_flag == 0` (`avc-interlaced-content`, mirroring `InterlaceCheck`),
    and an Annex B start code left in place of a sample's 4-byte NAL length prefix
    on an AVC/HEVC track (`length-prefix-violation`).
  - The CLI `check` command now sniffs its input (`0x47` TS sync at packet 0/1)
    and runs the TS diagnostic set (v1 + the new codec checks above) for a TS
    file, or `check_container_codec` for an ISOBMFF/CMAF file — one command,
    no new flag.
- Dependency on `transmux` (path, `default-features = false`) for the SPS/NAL/ADTS
  decoders and the `Media`/`Fmp4Demux`/`ProgressiveDemux`/`TsDemux` IR reused by
  the checks above.

## [0.2.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

## [0.1.0] — 2026-07-01
### Added
- `check_playlist` — text-input HLS playlist validator (RFC 8216): flags a missing
  `#EXTM3U` header, a media playlist without `#EXT-X-TARGETDURATION`, an `#EXTINF`
  duration exceeding the target, and a malformed `#EXT-X-DATERANGE` line (validated
  via `timed-metadata`). Adds `timed-metadata` dependency.
- `Scte35Check` diagnostic: container-level SCTE-35 splice consistency —
  reassembles `splice_info_section`s (table_id 0xFC) and flags unbalanced
  `splice_insert` out/in pairs (out with no matching in by stream end) and
  duplicate open "out"s per `splice_event_id`. Adds `scte35-splice` dependency.
- `PtsCheck` diagnostic: per-PID PES PTS/DTS monotonicity (33-bit wrap-unrolled,
  so a legal wrap is not flagged) + forbidden `PTS_DTS_flags == 0b01` detection
  (ITU-T H.222.0 §2.4.3.7). Honours signalled TS-layer discontinuities.
- Dependency on `mpeg-pes` for PES reassembly + PTS/DTS extraction.

### Fixed
- `PtsCheck` no longer false-positives on real streams. It now (a) only examines
  real PES PIDs (payload starts `00 00 01` + a PES stream_id), so PSI/SI PIDs
  like EIT (0x0012) are no longer misread as PES headers, and (b) validates the
  **decode timestamp** (DTS when present, else PTS) rather than PTS — legal
  B-frame PTS reordering is no longer flagged as `pts-backward`. Verified against
  real captures (`h264_aac.ts`, `france-tnt-pcr.ts`) which now yield zero findings.
- The CLI now runs the full diagnostic set (`SyncByteCheck`, `PatPmtVersionCheck`,
  `CcAnomalyCheck`, `PcrCheck`, `PtsCheck`, `Scte35Check`) — previously only
  `SyncByteCheck` ran.

_Unreleased — `media-doctor` has not yet been published to crates.io._
