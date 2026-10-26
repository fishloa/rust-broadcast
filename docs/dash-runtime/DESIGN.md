# dash-runtime: sans-IO DASH-pull client (design)

Status: DESIGN ONLY (no crate, no code in this PR). Plan-only; every behavioural statement cites a clause of ISO/IEC 23009-1:2022 (Ed.5,
"23009-1"), DASH-IF IOP v4.3 ("IOP"), ETSI TS 103 285 ("DVB-DASH") or a repo source. Spec notes: `docs/specs/dash/**`; verbatim private text:
`private/specs-verified/dash/**` (formulas below were cross-checked against the public notes; the private files are not checked out in CI, so the
work-breakdown step W-c re-reads them before coding). Sections 0-11 are written and committed one at a time.

## 0. Decisions at a glance

| # | Question | Decision | Why (one line) |
|---|---|---|---|
| D1 | Placement | NEW crate `dash-runtime` (client engine + own full MPD model/parser + addressing + timing), consumed by `broadcast-client` (`source/dash.rs`) now and `multimux` `dash_pull` later; multimux is not split | owner decision (#1457 answered no) |
| D2 | Shared retry logic | EXTRACT first (W-a) into `broadcast-client/src/source/fetch/`: small state machines (ConnAlloc, Backoff+Retry-After, FailureCounter, Deadlines, Window, bounded Body/range check), NOT a generic engine; HLS refactored onto them with zero behaviour change | rounds 3 and 8 found their bugs in exactly this logic; copying it triples the audit surface |
| D3 | NotMedia check | MOVE `has_ts_structure`/`has_media_fragment`/`is_progressive_movie` from hls-runtime private fns into `container-probe` (additive), both runtimes call it | one structural definition of "is this body media" |
| D4 | Wall clock | Session gains `set_wall_clock(now_us, unix_us)` (an anchor PAIR, host optional) and `Config::dash`; clock ladder: `UTCTiming` (client-measured, mono-anchored) > host wall clock > MPD response `Date`; none => fatal `ClockUnavailable` for dynamic MPDs only | static MPDs and every non-DASH source need no clock; UTCTiming alone anchors the monotonic clock |
| D5 | MPD parsing | NEW module `dash_runtime::mpd` (full model, quick-xml); `transmux::dash_parse` is NOT changed (published, structs are not `#[non_exhaustive]`, epoch-purity rule); reuse its pure helpers (`parse_iso8601_duration`, `SegmentTemplate::resolve`, `SegmentTimeline::resolve`, `SegmentIndexBox::parse_body`) | additive-only semver for transmux, no second template/timeline implementation |
| D6 | Addressing in 0.1.0 | SegmentTemplate (`$Number$`/`$Time$`/`$Bandwidth$`/`$RepresentationID$`, widths, `$$`), SegmentTimeline (incl. negative `r`, `@n`), SegmentList, SegmentBase+`indexRange` sidx (on-demand), BaseURL alternates; `$SubNumber$`/`@k` segment sequences and Ed.6 `Pattern` => typed unsupported | covers livesim2, ffmpeg, Shaka, MP4Box, on-demand profile |
| D7 | Live/VOD | One `DashClient` plays all selected AdaptationSets on ONE timeline (unlike HLS's second adapter for audio) | DASH segments of all sets share the MPD clock; avoids PTS re-alignment |
| D8 | MPD patch / xlink / inband emsg events | Out of 0.1.0. Patch: ignore `PatchLocation`, always refetch the whole MPD (patch is optional, 5.4.1). xlink: only `resolve-to-zero` removal, other `xlink:href` Periods/sets skipped with a non-fatal typed notice. emsg/EventStream: not surfaced (HLS parity) | cost/benefit; no fixture exercises them |
| D9 | LL-DASH | Out for 0.1.0 as a LATENCY feature (segment-granular delivery), but LL MPDs must play: `availabilityTimeOffset`/`Complete` parsed; requests are scheduled at the COMPLETE-segment time unless `DashConfig::use_availability_time_offset` | chunk-level delivery needs an incremental demux path the HLS adapter also lacks |
| D10 | Containers | fMP4/CMAF only; MPEG-2 TS segments (`mp2t-main`), WebM, subtitles (`stpp`/`wvtt`), trick modes, CENC/CBCS decryption: typed `TrackDropped`/`UnsupportedEncryption`/`UnsupportedContainer` | DVB-DASH and IOP are ISOBMFF-only for the live profiles |
| D11 | Variants model | Map to the existing `Event::Variants(MasterSelection)` + `Event::AudioRendition` (`VariantInfo.url` = the Representation's initialization URL); neutral type deferred (owner question Q2) | hosts already speak it; keeps `select_variant`/`select_audio_rendition` |

Not in scope at all (owner): SRT, mTLS, Smooth-pull, RTMP-pull.

## 1. Crate layout and public API

### 1.1 Layout (mirrors `hls-runtime/src/client`)

```
dash-runtime/            std; no tokio, no reqwest, no threads, no clock read, no RNG
  src/lib.rs
  src/mpd/      model.rs  parse.rs  inherit.rs  descriptors.rs      consume-only MPD (23009-1 5.3, 5.8.5)
  src/address/  template.rs timeline.rs list.rs base_index.rs plan.rs   Representation -> SegmentRef plan (5.3.9.*)
  src/timing/   clock.rs availability.rs live_edge.rs period.rs     pure formulas over an explicit utc_us input
  src/client/   engine.rs action.rs output.rs error.rs select.rs config.rs   DashClient (the HlsClient analogue)
  tests/ fuzz/ examples/ docs/ (spec transcriptions it relies on)
```

Dependencies: `broadcast-common`, `transmux` (`Fmp4Demux`, `Sample`, `TrackSpec`, `SegmentIndexBox`, template/timeline/duration helpers),
`container-probe` (D3), `url` (RFC 3986 join, 5.6.4), `quick-xml` (the one XML dependency; hence `std`, as for `ttml-subtitle`/`dvb-mabr`/`transmux` XML paths),
`jiff` (xs:dateTime / RFC 3339 / RFC 2822 `Date` parsing, never `now()`), `thiserror`. No `tokio` feature in 0.1.0: `broadcast-client` and `multimux` own their loops
(an optional adapter can follow like `hls-runtime::tokio_client`; nobody asked for it).

### 1.2 Conventions inherited from HlsClient (so the adapter can reuse its machinery)

| HlsClient | DashClient | Note |
|---|---|---|
| `new(playlist_url)` | `new(mpd_url, DashConfig)` | no I/O |
| `poll() -> Option<Action>` | `poll(now_us)` | **deviation**: availability gating needs the clock, so every mutating call takes the session's monotonic `now_us` |
| `Action::WaitMs` + `next_wait()` | `Action::WaitUntil(us)` + `next_wake()` | absolute, same domain as `Action::WakeAt` |
| `on_playlist(bytes)` | `on_mpd(now_us, bytes, ResponseMeta)` + `on_mpd_unchanged(now_us)` (304) | `ResponseMeta{sent_us, received_us, date: Option<&str>, etag, last_modified, final_url}`; the client parses `Date` with jiff |
| `on_resource(id, bytes)` | `on_init` / `on_index` / `on_segment(now_us, id, bytes)` | `Err(NotMedia{id})` from the shared structural check |
| `on_error(id)` | `on_failure(now_us, id, Failure) -> FailureClass` | **deviation**: DASH needs the status to tell "too early" from "gone" (A.7, section 5.5); `Failure::{Status(u16), Transport, Timeout, TooLarge}` |
| `retry_resource(id)`, `skip_resource(id)`, `retry_init()` | `retry(now_us, id)`, `skip(now_us, id)` | same contract: `skip` marks delivered and emits `Output::Discontinuity` |
| `set_playlist_base(url)` | `set_mpd_base(url)` | redirect target becomes the BaseURL base (A.11 priority 2) |
| `endlist_seen()`, `target_duration_secs()` | `is_static()`/`presentation_ended()`, `max_segment_duration_secs()` | feed the BC-5 playout gate |
| `select_variant(i)` | `select_representation(i)`, `select_audio(Option<usize>)`, `set_policy(RepresentationPolicy)` | `RepresentationPolicy` has the same three fields as `hls_runtime::VariantPolicy`; the session converts `Config::variant_policy` |

`Action` (non_exhaustive): `FetchMpd{url, validators}`, `FetchUtcTiming{id, scheme, url}` (`scheme` = `HttpHead|HttpXsdate|HttpIso|HttpNtp`; `direct` is resolved inline, `ntp`/`sntp` are
skipped because the session has no UDP action: 5.8.5.7 Table 35), `FetchInit{id,url,byte_range}`, `FetchIndex{id,url,byte_range}` (sidx for SegmentBase), `FetchSegment{id,url,byte_range}`,
`WaitUntil(us)`. `ResourceId` = `Init{rep}` | `Index{rep}` | `Segment{rep, number}`; `RepId` is a client-assigned u32 per distinct (Period@id, AdaptationSet key, Representation@id), stable
across MPD refreshes, so a refresh never re-requests a segment already in flight (5.4.1 identity rules).

`Output` (non_exhaustive): `Manifest(ManifestSelection)` (the `Master` analogue: periods, sets, chosen, audio candidates), `Tracks(Vec<TrackSpec>)`, `Init(Vec<u8>)` (dropped by the adapter, as HLS),
`Samples{track_id, samples}`, `DisplayChanged`, `Discontinuity`, `PeriodChanged{id, start_us, continuous}`, `EndOfStream`, `ClockSynced{source, uncertainty_us}`, `Notice(Notice)`
(non-fatal typed: `PreselectionIgnored`, `XlinkSkipped`, `PatchIgnored`, `SegmentSequenceUnsupported`, `EventStreamIgnored`, `LowLatencyIgnored`), `TrackDropped{..}`, `TrackSkipped{..}`,
`FragmentDamaged{..}` (the last three exactly as HLS). Samples carry the container media timeline (tfdt based), not a Period-rebased one (HLS parity; section 5.6).

Config lives in `DashConfig` (pure data, `non_exhaustive`, `validate()` like `Config::validate`): see section 6.4 for the full list; the session maps its own `Config` onto it.

## 2. Shared retry/backoff/failure logic: extract first (W-a)

**Decision: extract, as small source-agnostic state machines, before any DASH code.** `broadcast-client/src/source/hls.rs` (1 391 lines) mixes protocol-specific control flow
(what a playlist failure re-requests, key/init counters) with logic that is not HLS at all. Copying it would make DASH the third place where round-3 and round-8 class bugs
(per-segment attempts reset by a playlist refetch; one 604-requests-in-602-s loop; a stale deadline expiring a reused connection id) can recur. A generic "FetchEngine<Core>"
was rejected: the three adapters differ precisely in the control flow that decides what a failure re-requests, and abstracting that is where the bugs live. So the extracted pieces are
passive data structures with no knowledge of what is being fetched; each adapter keeps its own `fail_*` routing.

New module `broadcast-client/src/source/fetch/` (crate-private; `K` is any `Copy + Eq + Ord + Hash` resource key: HLS `ResourceId`, DASH `ResourceId`):

| Piece (moved from hls.rs) | Interface | Behaviour it pins |
|---|---|---|
| `conn.rs`: `ConnAlloc`, `ConnLease` | unchanged (`lease()`, `id()`, drop releases) | wrap-safe unique `ConnId` across adapters (audit F1/#1484) |
| `retry.rs`: `Backoff{steps_ms, cap_ms}` | `delay_us(attempt)`; ctor clamps every entry and the cap to >= 1 ms | 2/4/8 s then 10 s; zero never bursts (Round 3) |
| `retry.rs`: `retry_after_us(status, headers, wall)` | delta-seconds clamped to 60 s, only for 429/503; HTTP-date form honoured ONLY when `wall: Option<&WallClock>` is `Some` (W-b; W-a passes `None` = today) | Round 8 F3 |
| `retry.rs`: `RetryGate` | `hold_until(at)` (max), `lifted(now) -> bool`, `clear()` | gating of the core queue behind a backoff |
| `failures.rs`: `FailureCounter<K>` | `record(k, status) -> {count, status}`, `clear(k)`, `retain(pred)`, `verdict(count, max) -> Retry\|GiveUp` (`max==0`: first failure final; `u32::MAX`: never) | Round 8 F2: counter cleared ONLY on delivery or window exit, never by a playlist/MPD delivery |
| `failures.rs`: `Attempts` | single counter with `bump() -> u64`, `reset()` | the playlist/key/init counter |
| `deadlines.rs`: `Deadlines<K>` | `arm(k, deadline)`, `expire(now, is_live: FnMut(&K, u64) -> bool) -> Vec<K>`, `compact(live_len, rebuild)` | lazy deletion, O(log n) per expiry, stale deadline never expires a reused id |
| `window.rs`: `Window<K>` | `is_open()`, `issue(k)`, `hold(k, body)`, `drain_ready(deliver)` in issue order, `retire(k)`, `fail(k)/unfail(k)`; plus `Lanes<A>` (`Other`/`Retry`/`Fresh` by sequence number; `next(window_open)`) | in-flight cap counts issued-not-delivered; a retry inside the window is never blocked; held bytes <= cap x max_segment_bytes |
| `body.rs`: `BodyAcc{limit}`, `range_honoured`, `slice_body`, `content_range_is` | `push(&[u8]) -> Result<(), Oversize>`; 206 + exact `Content-Range`, or 200 sliced after the whole body (cap applies to the whole file) | audit #1484 byte-range rules |

`hls.rs` then owns only: `Purpose`/`Req`, the `fail_*` routing, `go_fatal`/`report`, `drain_outputs`, key/init counters. **Exit gate (W-a):** (1) `git diff main -- broadcast-client/tests` is
EMPTY (every existing test, incl. `audit_r8_segments.rs`, `audit_r7_init.rs`, `hls_renditions.rs`, `playout.rs`, is the oracle and must pass untouched); (2) one new unit-test file per piece with
the named cases above; (3) mutation list, each mutant killed: backoff index off by one, cap not applied, `Retry-After` clamp removed, counter cleared on playlist delivery, `max==0` treated as 1,
deadline compare `>` vs `>=`, lease released on `Req` clone, window cap `<` vs `<=`, held-body order, oversize check on the wrong length; (4) the 6-gate suite plus Apple-target `cargo check`.
Because behaviour must not change, W-a is a pure refactor PR (committed working state first, per the repo rule), size M.

**NotMedia check (D3).** `hls-runtime/src/client/engine.rs` keeps `has_ts_structure` (>= half of the 188-byte lanes start with `0x47`, ISO/IEC 13818-1 2.4.3.2),
`has_media_fragment` (a complete `moof` followed by a complete `mdat`, ISO/IEC 14496-12 8.8.4) and `is_progressive_movie` as private functions. They move into `container-probe`
(`no_std`+`alloc`, additive minor release, version 0.2 -> 0.3 under 0.x rules) as `container_probe::segment::{ts_structure, media_fragment, progressive_movie}`; hls-runtime calls them (internal change, patch
release, same tests) and dash-runtime uses `media_fragment` for `.m4s` and `progressive_movie` only to report `non_fragmented_mp4` exactly as HLS F9a. Gate: the existing hls-runtime
NotMedia tests pass unchanged and `container-probe` gets the same cases plus a fuzz target.

## 3. Wall clock

### 3.1 Problem and principle

Live DASH is defined on UTC: segment availability is `availabilityStartTime + PeriodStart + MPDstart + MPDduration (- ATO)` (23009-1 5.3.9.5.3; A.3.4; IOP 4.3.4.2), so a client
without UTC cannot decide when a `$Number$` segment exists or which one is the live edge. The session has ONE clock domain, the caller's monotonic `now_us` (CLAUDE.md of the client:
"no wall clock"). Principle: **the session never reads a clock; it maps `now_us` to UTC through an anchor pair `(mono_us, utc_us)` that someone else measured.** The anchor can come from
the MPD itself (`UTCTiming`, measured through the session's own requests) so the host does not have to provide anything.

### 3.2 The `UtcClock` (in `dash-runtime::timing::clock`, pure, property-tested)

State: `Option<Anchor{mono_us, utc_us, source: ClockSource, uncertainty_us}>`. `utc_at(now_us) = utc_us + (now_us - mono_us)` in `i128`, saturating; no `now()` anywhere.
`ClockSource` (ordered by trust, non_exhaustive, `name()` + `impl_spec_display!`): `UtcTiming`, `Host`, `MpdDate`. Replacement rule: a higher-trust source replaces a lower one at once;
the same source replaces its anchor when `new.uncertainty <= old.uncertainty` or the old anchor is older than `DashConfig::utc_resync_us`; a lower source never replaces a higher one,
except when the higher one failed `utc_wait_us` after the MPD arrived (the ladder in 3.4).

### 3.3 UTCTiming (23009-1 5.8.5.7 Table 35; IOP 4.7.3; DVB-DASH 4.7)

The client measures `offset = server_time - local_time` itself: the session records `sent_us` (the `now_us` at which it emitted the `HttpRequest`) and `received_us` (the `now_us` of
`on_response`, i.e. the first byte). The server time is stamped near response generation, so the sample is `(mono = sent + (received - sent) / 2, utc = parsed time)` with
`uncertainty = (received - sent) / 2 + resolution`, resolution = 500 ms for the `Date` header (RFC 9110 5.6.7: whole seconds, parsed with jiff `rfc2822`, +500 ms midpoint), the digit count for an
xs:dateTime / ISO body (`http-xsdate`, `http-iso`), 1 ms for `http-ntp` (RFC 5905 fraction). Samples with `received - sent > utc_max_rtt_us` (2 s) are discarded and retried once. This
also satisfies the IOP rule "account for transmission delay" and "shall account for a time-zone difference" (the xs:dateTime parser reads the offset).

| scheme | action | notes |
|---|---|---|
| `direct` | none | `@value` is the MPD generator's time; sampled at the MPD response midpoint; personalised, so lowest accuracy among UTCTiming |
| `http-xsdate`, `http-iso`, `http-ntp` | GET `@value`, body is the time | the common case: livesim2 and Akamai `time.akamai.com/?iso&ms` (verified: returns `2026-10-26T08:39:04.952Z`) |
| `http-head` | `HEAD` (RFC 9110 9.3.2) `@value`, read `Date`; on 405/501 re-issue as GET and `Close` after the headers | IOP 4.7.2: unreliable behind caches with unsynchronised clocks, so it is tried last among the HTTP schemes |
| `ntp`, `sntp` | none | no UDP action in the session; skipped with `Notice`-level log only (IOP 4.7.2 itself lists the schemes as optional) |

Several `UTCTiming` elements are alternatives in document order (5.8.5.7, 5.8 element rules): try them in order, each with `DashConfig::utc_timeout_us` (3 s); the first valid
sample wins. UTCTiming requests go through the session's normal request path (auth is origin-scoped so a UTC server on another origin gets no credentials; cookies, redirects and extra
headers behave as for every other request). Re-sync: every `utc_resync_us` (default 600 s) and once after a segment 4xx (A.7 "first check clock accuracy"; at most one per 30 s). A new
sample whose offset differs from the prediction by less than the sum of the two uncertainties only refines; a larger difference is a clock step and is adopted immediately. Because the segment
cursor is a position (a segment number), not a time, a step changes only gating and the join choice, never duplicates or skips already planned segments (tested, 3.6).

### 3.4 The ladder (what happens with and without a host clock)

| MPD | Source used | Result |
|---|---|---|
| `static` | none needed | works with no clock; `UTCTiming` is never fetched (availability is dynamic-only, 5.3.9.5.3) |
| `dynamic`, usable `UTCTiming` | UTCTiming | init segments are fetched concurrently while the sample is taken (an init is available from `AST + PeriodStart`, 5.3.9.5.2); the join waits at most `utc_wait_us` |
| `dynamic`, none/failed, `set_wall_clock` given | Host | `Output::ClockSynced{Host, uncertainty = DashConfig::host_clock_uncertainty_us (500 ms)}` |
| `dynamic`, none/failed, no host clock | MpdDate: the `Date` of the MPD response (IOP 4.8.5: servers SHOULD send it) | uncertainty = rtt/2 + 500 ms; early 404/425 are then expected and absorbed by section 5.5 |
| `dynamic`, no source at all | fatal `ErrorKind::ClockUnavailable` | only a server with neither UTCTiming nor `Date` and a host that never called `set_wall_clock` |

`DashClockPolicy::Auto` (default) is the order above (UTCTiming first because it IS the packager's time authority; IOP 4.7.3 "SHOULD use at least one announced method");
`PreferHost` takes the host anchor first (a host with a trusted NTP clock saves the round trip). A "timeline-anchored" fallback that joins at the end of a `SegmentTimeline` without a clock
was considered and rejected: the `Date` rung makes the no-clock case effectively never reachable, and the extra mode would be a second join algorithm to audit.

### 3.5 Exact public changes to `broadcast-client` (must land before 0.1.0 publishes; W-b)

| Type | Change | Breaking for a 0.1.0 consumer? |
|---|---|---|
| `Session` | NEW `set_wall_clock(&mut self, now_us: u64, unix_us: u64)`: "at the monotonic instant `now_us` the host UTC clock read `unix_us` microseconds since 1970-01-01T00:00:00Z (POSIX, no leap seconds)"; both reads back-to-back; callable any time, repeatedly (open, every 10 min, on a system clock-change notification); ignored by non-DASH sources except `Retry-After` HTTP-date | no (new method) |
| `Config` | NEW field `dash: DashConfig` (section 6.4); `Default` updated; `Config::validate` extended | yes for struct-literal construction without `..Default::default()` (the crate is unpublished) |
| `SourceKind` | NEW `Dash`; `Auto` picks it for a URL path ending `.mpd` (ASCII case-insensitive) | no (non_exhaustive) |
| `Event` | NEW `ClockSynced{source: ClockSource, uncertainty_us: u64}` (emitted when the anchor first appears and when its source changes, not per resync) | no (non_exhaustive) |
| `ErrorKind` | NEW `ClockUnavailable` (fatal) plus the DASH kinds of section 6.3 | no (non_exhaustive) |
| `Action::HttpRequest` | no type change; `method` may now be `"HEAD"` (http-head). Hosts must pass `method` through; documented in the README host contract | behavioural note for hosts |
| `Error::NotHls` | no rename; docs say "HLS or DASH" (`select_variant`, `select_audio_rendition` are valid for both) | no |
| `ClockSource` | NEW public enum re-exported from the crate root | no |
| `Session::open(url, config, now_us)` | UNCHANGED signature: open performs no I/O, so the host calls `set_wall_clock` between `open` and the first `poll` | no |

Rationale for an anchor PAIR instead of a bare `unix_us`: the host's two clock reads are separated by an unknown delay; passing both lets the host take them back-to-back and keeps the
session free of "what is now?" ambiguity. Rationale for a method instead of a `Config` field: the clock must be refreshable (drift, NTP step, sleep/wake).

### 3.6 Test strategy with simulated time (no real clock anywhere)

A `World{true_utc_us, host_skew_us, host_drift_ppm}` drives the test host: it derives `now_us` and `unix_us` from the world, and the fake origin computes availability from `true_utc_us`.
Cases (each asserts: no segment requested before its TRUE availability, which is the IOP 4.7.3 MUST; no gap or duplicate in the delivered number sequence; join within `delay +- 1 segment` of the live edge):
host skew +-{0.5, 5, 60} s with and without UTCTiming; UTCTiming server skewed against the origin; `Date`-only; step +-30 s mid-stream; 6 h at +-100 ppm with resync; rtt over `utc_max_rtt_us`;
garbage/empty/huge UTC bodies; HEAD answered 405; UTC fetch timeout falling to the next element then to the host clock; `direct`; `PreferHost`; a static MPD with no clock and no `Date`.
`UtcClock` property tests: mapping is monotone in `now_us`, replacement order matches 3.2, extremes saturate. Mutants: sign of the midpoint, 500 ms half-second omitted, anchor replaced by lower trust,
step threshold off by one, resync period ignored.
