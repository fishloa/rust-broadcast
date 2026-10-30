# dash-runtime: sans-IO DASH-pull client (design)

Status: DESIGN ONLY (no crate, no code in this PR). Plan-only; every behavioural statement cites a clause of ISO/IEC 23009-1:2022 (Ed.5,
"23009-1"), DASH-IF IOP v4.3 ("IOP"), ETSI TS 103 285 ("DVB-DASH") or a repo source. Spec notes: `docs/specs/dash/**`; verbatim private text:
`private/specs-verified/dash/**` (formulas below were cross-checked against the public notes; the private files are not checked out in CI, so the
work-breakdown step W-c re-reads them before coding). Sections 0-11 are written and committed one at a time; 12 to 12.3 record what each work-breakdown step built.

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
| `on_error(id)` | `on_failure(now_us, id, Failure) -> FailureClass` | **deviation**: DASH needs the status to tell "too early" from "gone" (A.7, section 5.4); `Failure::{Status(u16), Transport, Timeout, TooLarge}` |
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
`FragmentDamaged{..}` (the last three exactly as HLS). Samples carry the container media timeline (tfdt based), not a Period-rebased one (HLS parity; section 5.5).

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

### 3.3 UTCTiming (23009-1 5.8.4.11, 5.8.5.7 Table 35; IOP 4.7.3; DVB-DASH 4.7)

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
| `dynamic`, none/failed, no host clock | MpdDate: the `Date` of the MPD response (IOP 4.8.5: servers SHOULD send it) | uncertainty = rtt/2 + 500 ms; early 404/425 are then expected and absorbed by section 5.4 |
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

## 4. MPD parsing: what exists, what is missing, where it lives

### 4.1 `transmux::dash_parse` today (read from the source, `transmux/src/dash_parse.rs`)

A v1 subset written as the inverse of `DashPackager`: `Mpd{profiles, mpd_type, mediaPresentationDuration, minimumUpdatePeriod, availabilityStartTime (RAW string), timeShiftBufferDepth,
first BaseURL, periods}`, `Period{id,start,duration, first BaseURL}`, `AdaptationSet{mimeType, contentType, first BaseURL, effective SegmentTemplate}`, `Representation{id,bandwidth,codecs,width,height,
frameRate (string), audioSamplingRate, mimeType, first BaseURL, effective SegmentTemplate}`, `SegmentTemplate{timescale, initialization, media, startNumber, duration, presentationTimeOffset, timeline}`,
`SegmentTimeline{S{t,d,r}}` with negative `r`. It is not namespace-aware (prefixes stripped) and skips everything else as opaque subtrees. The structs are NOT `#[non_exhaustive]`.

| Feature | dash_parse today | dash-runtime 0.1.0 | Clause |
|---|---|---|---|
| `MPD@publishTime/minBufferTime/maxSegmentDuration/suggestedPresentationDelay/availabilityEndTime/@id`, `Location` | missing | parsed (xs:dateTime via jiff, any offset; no zone = UTC) | 5.3.1.2 Table 3; IOP 4.4.4 |
| `UTCTiming` (all 7 schemes) | missing | parsed, ordered; 5 acted on (section 3.3) | 5.8.5.7 |
| `BaseURL` alternates, `@serviceLocation`, `@availabilityTimeOffset/Complete`, DVB `@priority/@weight` | first only | all levels, all alternates, deterministic failover (section 7.2) | 5.6.5; DVB-DASH 10.8.2 |
| Period chaining (`start` absent, previous `duration`), early-available/terminated Periods | `start`/`duration` only | PeriodStart derivation 5.3.2.1 (steps 1-5), early terminated honoured, early-available ignored until a start exists | 5.3.2.1, 5.3.2.5 |
| `AdaptationSet@id/group/lang/segmentAlignment/startWithSAP`, `Role`, `Accessibility`, `Label` | missing | parsed (Role/lang feed audio selection) | 5.3.3, 5.8.5.5 |
| `ContentProtection` (+ `cenc:default_KID`) | skipped | parsed; presence on a set/rep => that set is `UnsupportedEncryption` unless an unprotected alternative exists | 5.8.4.1, 5.8.5.2 |
| `Essential/SupplementalProperty` | skipped | parsed generically; understood: `urn:mpeg:dash:period-continuity:2015`, `...:period-connectivity:2015`, `urn:mpeg:dash:reset:2016`, `http://dashif.org/guidelines/last-segment-number`; an unrecognised `EssentialProperty` on a set/Representation drops that parent element (5.8.4.8 client rule); at MPD level an id-group none of whose members is understood ends the presentation (fatal `MpdInvalid`) | 5.3.2.4, 5.4.2; IOP 4.4.3 |
| `SegmentTemplate` `$RepresentationID$/$Number$/$Time$/$Bandwidth$`, `%0Nd`, `$$` | yes | reuse `SegmentTemplate::resolve` for substitution; unknown identifier => Representation dropped (5.3.9.4.4 step 2) | 5.3.9.4.4 Table 21 |
| `$SubNumber$`, `S@k`, Ed.6 `Pattern`/`S@p/@pE/@ssp` | literal / ignored | Representation dropped with `Notice::SegmentSequenceUnsupported` + `TrackDropped` | 5.3.9.6.4/5, ED6-SCHEMA-DIFF |
| `S@n`, negative `r` | no `@n`; negative `r` yes | both, lazily expanded (4.3) | 5.3.9.6.1, 5.3.9.6.2 |
| `SegmentTemplate@endNumber/@index/@bitstreamSwitching`, `@eptDelta/@pdDelta/@presentationDuration`, `Initialization`/`RepresentationIndex` | missing | parsed; `endNumber` bounds the plan; PTO/eptDelta feed continuity; bitstream switching ignored | 5.3.9.2.2 Tables 16-17 |
| `SegmentList` + `SegmentURL@media/@mediaRange/@index/@indexRange`, `@duration/@startNumber` | missing | parsed and planned | 5.3.9.3 Table 19 |
| `SegmentBase@indexRange` + `Initialization@range` (on-demand profile, one file + sidx) | missing | parsed; sidx fetched and expanded (4.4) | 5.3.9.2, 5.3.9.5.4; 14496-12 8.16.3 |
| `urn:mpeg:dash:urlparam:2014` (`UrlQueryInfo`) | skipped | minimal: `@useMPDUrlQuery`, literal `@queryString` (7.1) | Annex I.2 |
| `xlink:href/@actuate` on Period/AdaptationSet/SegmentList | missing | `urn:mpeg:dash:resolve-to-zero:2013` removes the element; any other href: element skipped + `Notice::XlinkSkipped` (not resolved in 0.1.0) | 5.5.3 rule 4 |
| `EventStream`, `InbandEventStream` | skipped | parsed only to count; `Notice::EventStreamIgnored`; emsg boxes ride along in segments untouched | 5.10 |
| `Preselection`, `Resync`, `ServiceDescription`/`Latency`, `ProducerReferenceTime`, `PatchLocation`, `ContentSteering` | skipped | skipped; one `Notice` per kind per session | 5.3.11, 5.3.13, Annex K, 5.12, 5.15 |
| `MPD@type="list"` (Ed.6) | `Static` default | fatal `UnsupportedAddressing{"presentation_type_list"}` | ED6-SCHEMA-DIFF |

### 4.2 Decision (D5): new module, `transmux` untouched

Extending `dash_parse` in place would add public fields to non-`#[non_exhaustive]` structs (breaking), and any `transmux` version move forces every workspace dependent through the epoch-purity rule
(a caret bucket spanning two epochs; CLAUDE.md "Epoch-pure compat buckets"). So the full model lives in `dash_runtime::mpd` (consume-only: no serializer; the workspace round-trip invariant is met against the
transmux generator by a differential test, the same exception `dash_parse` itself relies on). It is namespace-aware (`quick_xml::NsReader`: `urn:mpeg:dash:schema:mpd:2011`, `xlink`, `cenc`,
`urn:dvb:dash:dash-extensions:2014-1`), skips foreign-namespace content (the schema's `any`), and is bounded: input `DashConfig::max_mpd_bytes` (4 MiB), nesting depth 32, element count 100 000,
attribute length 8 KiB, Periods 1 000, timeline entries via the same 100 000 expansion cap `transmux` uses. Failure taxonomy: `MpdError::{Xml, NotMpd, Invalid{what}, TooLarge}`.
It is designed to be hoisted into a `broadcast-dash` syntax crate later (as `broadcast-hls` was, issue #878) without an API change: the module has no dependency on the client.

Reuse from `transmux` (all existing public API, zero change): `parse_iso8601_duration` (xs:duration; rejects calendar `Y`/`M` and negatives, which for Period/MPD times is correct),
`SegmentTemplate::resolve` (substitution, widths up to `MAX_FORMAT_WIDTH`, `$$`), `SegmentIndexBox::parse_body` (sidx v0/v1), `Fmp4Demux`. `SegmentTimeline::resolve` is NOT the production
expander: it materialises up to 100 000 entries per call and has no `@n`; dash-runtime's lazy expander (4.3) is checked against it as an ORACLE in a differential test whenever no `@n` is present.

### 4.3 Segment information: inheritance and the plan

Inheritance is attribute-by-attribute down Period > AdaptationSet > Representation, lower level wins (5.3.9.1; the repo's `fixtures/dash/manifest-inheritance.mpd` is the shape fixture);
`@duration` and `SegmentTimeline` are exclusive, so a child that introduces a timeline does not inherit `@duration` (5.3.9.2.1); a SegmentTemplate at one level forbids a SegmentList below and
vice versa (5.3.9.1), violation drops the Representation. The result per Representation is one `SegmentInfo` = `Template | List | Base`.

`address::plan` turns a `SegmentInfo` plus (PeriodStart, Period end, `utc_us`, MPD fetch context) into a lazy ordered view of `SegmentRef{number, media_time_ticks, duration_ticks, presentation_us
(Period-relative = time - PTO), url, byte_range, avail_start_us, avail_end_us}`. Formulas (all in integer microseconds/i128, ticks via `@timescale`, which defaults to 1, 5.3.9.2.2):

- `$Number$` with `@duration`: `MPDstart(n) = (n - NumberStart) * d`, `n = NumberStart + k - 1` (5.3.9.5.3; section 11 explains the printed `(k-1)+(kstart-1)` misprint and the real-content check). Last segment of a Period: `PeriodEnd - start`
  (A.3.3 case 1); `@endNumber` caps (5.3.9.5.3). Neither `@duration` nor timeline: exactly one segment, `start 0` (A.3.3 case 3).
- `$Time$` / timeline: running `time`, `@t` absolute (gap allowed, never smaller than the previous end), `@n` resets the number, `r >= 0` => `r + 1` segments, `r < 0` => to the next `S@t`, else to Period end
  (dynamic: to `FetchTime + MUP`, A.3.2); `presentation_us = (time - PTO)/timescale` (5.3.9.6.1 step 1). Period end on the media timeline is `PTO + (PEwc - PSwc)` (the contract documented in `SegmentTimeline::resolve`).
- Availability, dynamic only: `avail_start = AST + PeriodStart + MPDstart + MPDduration - ATO`, `avail_end = avail_start_without_ATO + MPDduration + TSB` (5.3.9.5.3; A.3.4 prints the equal form `start + TSB + 2*dur`;
  IOP 4.3.3.2 SAET). ATO is the sum over every level that contributed to the URL (Segment information and BaseURL levels) and is applied only when `use_availability_time_offset` (D9).
  `Initialization` available from `AST + PeriodStart` (5.3.9.5.2).
- A timeline-listed segment is available at publish time by the IOP 4.3.6 sanity rule, so with a timeline the clock gates only the JOIN and the earliest-in-window check, not each request.

### 4.4 SegmentBase / on-demand profile

`Representation` with `BaseURL` + `SegmentBase@indexRange` (+ `Initialization@range`): init = `FetchInit` with the byte range; `FetchIndex` fetches `indexRange`; `SegmentIndexBox::parse_body`
gives references (type 0 only; a hierarchical type-1 reference => `SegmentSequenceUnsupported`) which become `SegmentRef`s whose byte range is `first_offset + sum(referenced_size)`, anchored at the first byte
after the sidx box (14496-12 8.16.3 as noted in `transmux/src/timing.rs`); `presentation_us = earliest_presentation_time + cumulative duration - PTO`. Responses honour the shared range rules (206 + exact `Content-Range`, or a 200 sliced
after the whole body, W-a `body.rs`). Static MPDs only in practice; the dynamic variant works identically.

## 5. Client algorithms (live and VOD)

Phases of `DashClient`: `FetchingMpd` -> (dynamic) `Syncing` (UTC sample, section 3; inits are fetched meanwhile) -> `Running` -> `Ended` | `Fatal`. VOD (static MPD) is the same loop with no clock,
no refresh and no availability gating (A.2: "static is a subset of the same behaviour").

### 5.1 Selection (per Period, re-run on every Period change and on `select_*`)

1. Candidate sets: `mimeType`/`contentType` `video/mp4`|`audio/mp4` (or the Representations' own), not dropped by D10 (text, TS/WebM mime, `dependencyId` representations, protected sets
   with no clear alternative in the same `@group`, unknown `EssentialProperty`, templates failing 5.3.9.4.4 step 2). Each dropped set yields one `TrackDropped`/`UnsupportedEncryption`/`Notice`.
2. Video: first remaining video set with Role `main` (else document order); Representation = highest `@bandwidth` satisfying `RepresentationPolicy` (`max_bandwidth`, `max_height`, codec-prefix list), else the lowest.
   `ManifestSelection.variants` = every Representation of every selectable video set in MPD order (so `select_variant(i)` can cross sets), `chosen` = that index.
3. Audio: renditions = selectable audio sets of the Period. Pick: first set whose `@lang` BCP 47 prefix matches `preferred_audio_language` (ASCII case-insensitive) and whose Role is not `commentary`/`description`;
   else Role `main`; else the first. Representation = highest `@bandwidth` the codec policy allows. DASH has no DEFAULT/AUTOSELECT flag, Role `main` plays that part (5.8.5.5 Table 34). A muxed Representation
   (audio and video in one init) yields several local tracks and needs no separate pick.
4. `TrackId`s: 1-based in selection order (video sets then audio) on first encounter of a (media kind, AdaptationSet identity); stable across Periods and Representation switches, so a Period that drops a kind does NOT recycle its id for a different kind (the `Tracks` event re-announces the new `TrackSpec`);
   local `moov` track ids are remapped (every Representation's init typically uses id 1, multimux `dash_pull` "Track id remapping" note).
   An AdaptationSet's `@id` is part of that identity when it is present: DASH signals Period continuity/connectivity through equal `AdaptationSet@id` across Periods (ISO/IEC 23009-1 5.3.2.4), so a renumbered `@id` denotes a new identity and allocates a fresh id range even when `@lang` and `Role` are unchanged; when `@id` is absent the identity falls back to media kind plus `@lang` and `Role` descriptors.

### 5.2 MPD refresh (dynamic only)

- Next refresh at `fetch_time + max(MUP, min_mpd_refresh_us)` where `fetch_time` is the UTC the SERVER processed the request, approximated by the client-clock midpoint of the exchange (A.2, IOP 4.4.4 step 1: not the
  arrival time); `min_mpd_refresh_us` defaults to 1 s so `MUP=0` (instant updates, IOP 4.4.3) does not become a request storm; no `MUP` => no refresh (5.4.1: valid to the end; `mediaPresentationDuration` or the last `Period@duration` must exist).
- Forced early refresh (bounded by the same floor): the plan is exhausted before the presentation end; a segment 404/410/5xx inside its availability window (A.7); a clock step; a failed segment alternate-BaseURL rotation.
- The request is conditional (`If-None-Match`/`If-Modified-Since` from the previous `ETag`/`Last-Modified`, IOP 4.4.4 step 1); a 304 or byte-identical body is a success that only advances `fetch_time` (A.2).
  An MPD whose `publishTime` is older than the current one is a stale edge answer: ignored, retried at the floor (IOP 4.5.1: a later `publishTime` identifies an update).
- Update rules enforced (5.4.1): `MPD@id`/`Period@id`/`AdaptationSet@id` continuity, `Representation@id` equivalence assumed, `availabilityStartTime` and `Period@start` unchanged. A changed AST, a removed current Period, or the
  `urn:mpeg:dash:reset:2016` descriptor (5.4.2) is an MPD RESET: `Output::Discontinuity`, drop the plan, rejoin at the live edge (departure from 5.4.2's "keep playing the old MPD as long as possible", documented).
  A Representation or set that disappears is re-selected within its Period (section 5.1) and re-announced.
- MPD URL for updates: first `Location`, else the redirect target, else the original (A.11); `BaseURL` never applies to MPD requests (A.11).
- MPD patch (5.15) is not used: `PatchLocation` is ignored and the whole MPD refetched; `Notice::PatchIgnored` once.
- Dynamic -> static (IOP 4.6.4): the new MPD ends the presentation; the plan closes at the last segment, `EndOfStream` follows its delivery.
- Refresh failure: never given up (like a playlist); shared backoff, `Status::Reconnecting`; the last MPD keeps driving a duration-template plan meanwhile, a timeline plan simply stops growing.

### 5.3 Live edge and join (23009-1 5.3.9.5.3, A.2 step 4, IOP 4.3.4.1/4.3.4.2)

- Presentation delay `D` = `MPD@suggestedPresentationDelay` when present (A.2 step 4; IOP 4.3.4.1 step 5 "may be used"), else `max(4 s, 3 x maxSegmentDuration)` (IOP 4.3.3.2.2: SPD "2-4 x SDURATION and not under 4 s"),
  clamped to `TSB - 2 x maxSegmentDuration` when a `timeShiftBufferDepth` exists. `DashConfig::live_delay_us` overrides.
- Join media time `T_join = utc_now - D - (AST + PeriodStart)` (Period-relative). For EACH selected Representation choose the segment that CONTAINS `T_join` on that Representation's own timeline (audio and video
  segment boundaries differ, e.g. the livesim2 audio timeline mixes 96 256- and 95 232-tick segments), clamped to `[oldest in window, newest complete]` where complete means `avail_start <= utc_now`. Number-based
  oldest/newest: with `n = floor((utc_now - AST - PS + ATO) / d)` complete segments, newest = `startNumber + n - 1`; oldest = `max(startNumber, startNumber + ceil((utc_now - AST - PS - TSB)/d) - 2)` from `avail_end >= utc_now`.
- The delivered burst up to `D` of media is what the BC-5 live playout gate then measures (lead based, `min(3 x max segment duration, live_start_lead_us)`), so start-up latency is the parallel fetch time, not `D`.
- Cursor semantics: after the join, each Representation advances by segment identity (number), never by the clock. The clock decides only (a) the join, (b) `avail_start` gating, (c) `Expired` detection.
  Request time of segment `k`: `avail_start(k) [- ATO if enabled] + availability_margin_us` (default 0; IOP 4.7.3 "must not request a segment before its availability start measured by any chosen method"); already-available backlog is
  emitted at once and the adapter's in-flight window caps it. `next_wake()` is the earliest unmet `avail_start`.
- Static MPD: start at the first segment of the first Period; no seek in 0.1.0 (the `#t=` fragment of Annex C.4 and IOP 4.3.3.5 is not interpreted).

### 5.4 Segment request failures (A.7, IOP 4.8.2.1/4.8.5, DVB-DASH 10.8)

`on_failure(now, id, Failure) -> FailureClass`; the core classifies, the adapter applies the shared retry policy (section 2):

| Condition | Class | Counted by `FailureCounter`? | Cadence |
|---|---|---|---|
| 404/410/425 and `utc_now < avail_start + early_grace_us` (default 1 s); any 425 (RFC 8470 "Too Early", observed from livesim2) | `NotYetAvailable{retry_at}` | no | `early_retry_us` 250 ms doubling to 1 s; never the 2/4/8 s table (IOP 4.3.6: "back off slightly") |
| 404/410 inside `[avail_start + grace, avail_end]` | `Missing` | yes | shared backoff; the FIRST one per Representation also forces an MPD refresh and one UTC re-sync (A.7 "check clock accuracy, then refetch the MPD"; at most one re-sync / 30 s) |
| `utc_now > avail_end` (seen before requesting, or 404/410 after it) | `Expired` | no | none: the core skips to the oldest in-window segment, emits `Output::Discontinuity` and one `Notice::FellBehind`; three in a row => rejoin at the live edge |
| 5xx, 429, 503, transport error, timeout, a 200 that fails `media_fragment` | `Server` | yes | shared backoff + `Retry-After`; after 2 consecutive failures on one BaseURL alternate rotate to the next and refresh the MPD (A.7; DVB-DASH 10.8.2.3) |
| 401/403/407, redirects | handled by the session auth/redirect layers | n/a | unchanged |

Give-up at `hls_max_segment_failures` consecutive counted failures (HLS Round 8 contract, same counter semantics): static => fatal `SegmentFetchFailed`; dynamic => `skip` (marks delivered, `Discontinuity`) plus one
non-fatal `SegmentFetchFailed`. An init or index failure uses the init counter (`InitFetchFailed`, fatal after `max_key_failures`, HLS Round 7 F1).

### 5.5 Periods

PeriodStart by 5.3.2.1; segments of consecutive Periods are planned back to back by identity. At a boundary: if every selected set carries `urn:mpeg:dash:period-continuity:2015` naming the previous Period and the
same `AdaptationSet@id` (5.3.2.4), and the new Representation's resolved init URL equals the old one, nothing changes (no init, no `Discontinuity`); otherwise the new init is fetched, `Tracks` is re-announced if the config changed and
`Output::Discontinuity` precedes the first sample (A.9: period-connected => tell the decoder about the discontinuity; unsignalled Periods are resync points, IOP 4.3.4.3). Samples keep the container media timeline (tfdt), as for HLS;
`Output::PeriodChanged{id, start_us, continuous}` exposes the Period mapping for a host that wants to rebase. No trimming of a segment that overshoots `PeriodEnd` (23009-1 7.2.1 permits small overlaps).

### 5.6 Representation switching and ABR hooks

No automatic ABR in 0.1.0 (HLS parity: `VariantPolicy` at start, `select_variant` after). A switch inside a set takes effect at the next segment boundary: the new Representation's plan continues from the segment containing the next unrequested
media time; allowed when the set is `segmentAlignment` true with `startWithSAP` 1 or 2 (A.6 row 1), else it is performed as a rejoin with a `Discontinuity`. The new init is always fetched (bitstream switching is ignored: always correct).
`Output::Manifest` carries per-Representation `bandwidth` so a host can build its own ABR on measured throughput (A.10) and call `select_variant`.

### 5.7 Low-latency DASH (D9) and everything else deliberately out

LL-DASH (`availabilityTimeOffset` + `availabilityTimeComplete=false`, CMAF chunked transfer, `ServiceDescription/Latency`, DVB-DASH 10.20) is OUT as a feature for 0.1.0, with a safe degradation: such MPDs
play at segment granularity (requests are scheduled at the complete-segment time, so no chunk is ever consumed half-delivered, DVB-DASH 10.20.2 notes progressive arrival only matters when requesting early) and emit `Notice::LowLatencyIgnored`.
Reason: chunk-level delivery needs an incremental `Fmp4Demux` feed and a partial-body event in the session that the HLS adapter also lacks (it delivers on `on_closed`); doing it once for both is a 0.2 story (it would be shared with
LL-HLS parts in `hls-runtime`). `use_availability_time_offset=true` is available for experimentation and still delivers whole segments. Out: subtitles (typed `TrackDropped`), trick modes (`Representation@maxPlayoutRate`, A.5), segment sequences, seeking, CENC decryption, MPEG-2 TS and WebM segments (typed `UnsupportedContainer{"mp2t_segments"|"webm"}`), `EventStream` surfacing (HLS does not surface SCTE-35 either; `mp4-emsg`/`timed-metadata` are the future route).

## 6. Mapping into `broadcast-client` (source/dash.rs)

New `source/dash.rs` (adapter over `DashClient`, the `source/hls.rs` analogue) and `source/dash_session.rs` only if a second client is ever needed (it is not: D7). It owns exactly what `hls.rs` owns after W-a:
`Purpose{Mpd, Utc(id), Resource(ResourceId)}`, `Req`, the `fail_*` routing, `go_fatal`, `drain_outputs`, and uses the `fetch/` pieces for everything else. The auth/cookie/redirect layer (`AuthState`, `redirect.rs`) is
reused untouched.

### 6.1 Output -> Event

| `Output` | `Event` / effect |
|---|---|
| `Manifest` | `Event::Variants(MasterSelection)`: `VariantInfo{url = the Representation's initialization URL (a real absolute URL), bandwidth, average_bandwidth: None, resolution, codecs, audio_group = audio set @group or @id, subtitles_group: None}`; audio sets -> `RenditionInfo{media_type: Audio, group_id, name = Label else @lang else Representation@id, language, default = Role main, autoselect: true, url = Some(init URL), None for a muxed Representation}`; then `Event::AudioRendition` for the active one (HLS BC-3b semantics) |
| `Tracks` | `Event::Tracks` (ids are session-global already; no `AUDIO_TRACK_ID_OFFSET`) |
| `Init` | dropped |
| `Samples` | one `Event::Sample{track, sample, join}` per sample; `join` from the per-video-track `LiveJoin` wrapper (`join.rs`, shared with HLS/TS: AVC/HEVC/VVC and the sync-sample rule for the rest) |
| `DisplayChanged` | `Event::DisplayChanged` |
| `Discontinuity` | `Event::Discontinuity` (re-arms joins, resets the jump detector and the playout gate exactly as for HLS) |
| `EndOfStream` | `Status::Ended` (terminal, as HLS ENDLIST: `reconnect()`/`select_*` become no-ops afterwards) |
| `ClockSynced` | `Event::ClockSynced` |
| `Notice` | one non-fatal `Event::Error{UnsupportedFeature{feature}}` per feature per session (`preselection`, `xlink`, `mpd_patch`, `segment_sequence`, `event_stream`, `low_latency`); `FellBehind` -> `LiveEdgeLost` (non-fatal, once per occurrence run) |
| `TrackDropped` / `TrackSkipped` / `FragmentDamaged` | `ErrorKind::TrackDropped{track_id (0 for a set that was never fetched), stream_type: None, fourcc: Some(first token of @codecs)}` / `skipped_kind` (encv/enca => `UnsupportedEncryption`) / `UnsupportedContainer{damaged_fragment}`, deduplicated as HLS F4/F1/F9a |

Status: `Establishing` until the first sample then `Live` once; `NotFound` for 404/410 of the FIRST MPD (per occurrence, with backoff, as HLS main playlist); `Reconnecting{attempt}` from the shared retry; `Ended`.
An audio set that never delivers does not block `Ended` forever: ONE client, so the end is the end of all selected plans (unlike HLS BC-3b where a failing rendition blocks it).

Buffering: static MPD => BC-5 playout gate in VOD mode (`vod_start_lead_us`); dynamic => BC-5 gate in live mode with `min(3 x max_segment_duration_secs(), live_start_lead_us)` (the `target_duration_secs` role). The BC-2
`buffering-runtime` PDV controller is NOT used (it models TS arrival jitter; HLS does not use it either). Timeline-jump detection stays on (VOD and live), rendered-track defaults are unchanged (lowest id per kind).
An explicit `set_rendered_tracks` passes through (HLS rule in `rendered.rs`).

### 6.2 `Session` surface for DASH

`select_variant(i)` -> `DashClient::select_representation(i)` (index into `ManifestSelection.variants`; unknown index `Error::NoSuchVariant`); `select_audio_rendition(Option<usize>)` -> `select_audio`
(`None` stops separate audio; manual choice replaces automatic selection for the session, kept across Period changes by `(Label/@lang, Role)` else re-picked); `Config::variant_policy`, `preferred_audio_language`, `auto_select_audio` are
reused; `reconnect()` restarts the MPD fetch and rejoins (key/cookie jar kept); `close()` as HLS. `Session::set_wall_clock` per section 3.5. After a fatal error everything is ignored (HLS rule).

### 6.3 `ErrorKind` additions (all `#[non_exhaustive]`, `name()` snake_case + `impl_spec_display!`, `label_coverage` guard updated)

`MpdParse{reason: &'static str}` (XML/size/namespace problems; non-fatal, one per kind until a success, fatal after `max_key_failures` consecutive with no good MPD between, the init-decode rule), `MpdInvalid{reason}`
(no Period, no playable set, unprocessable essential group; fatal when nothing can play), `UnsupportedAddressing{reason}` (`presentation_type_list`, `data_url`, `open_ended_range`, `hierarchical_sidx`; fatal only when no Representation is left),
`UnsupportedFeature{feature: &'static str}` (non-fatal, 6.1), `LiveEdgeLost` (non-fatal), `ClockUnavailable` (fatal). Reused unchanged: `UnsupportedEncryption`, `TrackDropped`, `UnsupportedContainer`, `SegmentFetchFailed`, `InitFetchFailed`,
`InitDecodeFailed`, `BodyTooLarge{field}` (fields renamed per Q3), `UnknownRenderedTrack`.

### 6.4 `DashConfig` (inside `Config::dash`; defaults; every zero timeout/limit rejected by `Config::validate` with `Error::InvalidConfig`)

| Field | Default | Meaning |
|---|---|---|
| `clock_policy` | `Auto` | `Auto` / `PreferHost` (3.4) |
| `utc_timeout_us` / `utc_wait_us` | 2 s / 5 s | per UTCTiming request / total before the ladder falls through |
| `utc_max_rtt_us`, `utc_resync_us`, `host_clock_uncertainty_us` | 2 s, 600 s, 500 ms | 3.3 |
| `live_delay_us` | `None` (derive, 5.3) | override of `D` |
| `availability_margin_us` | 0 | added to `avail_start` before requesting |
| `early_retry_us`, `early_grace_us` | 250 ms, 1 s | 5.4 |
| `min_mpd_refresh_us` | 1 s | 5.2 floor |
| `use_availability_time_offset` | false | D9 |
| `cross_origin_urls` | true | `false`: any MPD-derived URL (BaseURL, segment, UTCTiming) whose origin differs from the MPD's fails as `UnsupportedAddressing{"cross_origin"}`; for server-side pulls (section 10, R3) |

Shared with HLS and therefore documented as "every segment-pull source": `max_redirects`, `extra_headers`, cookies, `variant_policy`, the audio-language pair, `hls_backoff_ms`, `hls_backoff_cap_ms`,
`hls_max_segment_failures`, `hls_max_inflight_segments`, `hls_request_timeout_us` (MPD, init, index, UTC class), `hls_segment_timeout_us`, `max_playlist_bytes` (MPD), `max_segment_bytes`, `max_key_failures` (init/MPD-parse counter),
`playout`, `timeline_jump`, `max_queued_events`. The `hls_` prefix is a wart for DASH; renaming them to neutral names before 0.1.0 is owner question Q3.

## 7. Transport details

7.1 **URL building.** Every URL resolves by RFC 3986 5.2 through the chain document URL (the final MPD URL, A.11) > `MPD.BaseURL` > `Period.BaseURL` > `AdaptationSet.BaseURL` > `Representation.BaseURL` > template/`SegmentURL`
(5.6.4; an absolute `BaseURL` overrides higher levels, 5.6.5), with `url::Url::join` (never string concatenation, 5.6 informative note). The query of the MPD URL is NOT propagated to segment URLs by default (RFC 3986 drops it; credentials
travel as origin-scoped `Authorization`/cookies/`extra_headers` instead). The one normative propagation mechanism, `urn:mpeg:dash:urlparam:2014` (Annex I.2: `up:UrlQueryInfo@useMPDUrlQuery` and a literal `@queryString`, at MPD/AdaptationSet/Representation level), IS implemented in its minimal form so a token-in-the-MPD-URL service plays; `@queryTemplate` placeholders, `xlink` query info and the 2016 `ExtUrlQueryInfo`/`ExtHttpHeaderInfo` scheme (Annex I.3, headers and MPD requests) are not understood: as an `EssentialProperty` they drop the parent, as a `SupplementalProperty` they are ignored. Only `http`/`https` are fetched; a `data:` URL (5.3.9.1 b) is `UnsupportedAddressing{"data_url"}`; the URL fragment is ignored.

7.2 **BaseURL alternates (5.6.5, DVB-DASH 10.8.2).** Candidate order: DVB `@priority` ascending (default 1, namespace `urn:dvb:dash:dash-extensions:2014-1`) then document order; `@weight` is parsed but NOT used, because
weighted random selection (10.8.2.2) needs an RNG and the core is deterministic and replayable (a documented departure; multimux can layer randomisation outside the core). After 2 consecutive failures on one location its `@serviceLocation`
(default: the URL) joins the blacklist together with every other entry of the same priority (10.8.2.1), the next location is used and the MPD is refreshed (A.7); the blacklist lives as long as the MPD URL; when every alternate is blacklisted it is
cleared once and the cycle restarts (DVB-DASH 10.8.2.2 would stop playback; a live client keeps trying and the counters decide).

7.3 **Byte ranges.** `byte_range` is `(offset, length)` as for HLS. Sources: `Initialization@range`, `indexRange`, `SegmentURL@mediaRange`, sidx subsegments (4.4). Grammar `a-b` (RFC 7233 2.1 single range); open-ended `a-` => `UnsupportedAddressing{"open_ended_range"}`.
206 + exact `Content-Range`, or a 200 sliced after the whole body (shared `body.rs`). `BaseURL@byteRange` (Annex E.2) is ignored.

7.4 **Auth, cookies, redirects, headers.** The `AuthState` layer applies to every request (MPD, UTCTiming, init, index, segment) exactly as for HLS: credentials, cookies and `extra_headers` only to the session origin (the MPD's), segments or UTC servers on
a CDN origin carry none; per-request redirect budget; a 401/407 from the session origin enters `AuthRequired`/wait with the request parked. The final URL of an MPD redirect becomes the base (`set_mpd_base`) and the update URL unless a `Location` element exists
(A.11 priorities 1-2; risk: a temporary signed-URL redirect target is then reused, the spec text is followed and the risk recorded in section 10).

7.5 **Content checks.** MPD: `application/dash+xml` is the registered type (Annex C) but not required (many origins send `text/xml`); the body must have root `MPD` in the MPD namespace (`NotMpd`). Segments/inits: no `Content-Type` check; a 200 that is not
`media_fragment` is a failed fetch (`NotMedia`, HLS Round 8 F1); an init without a complete `moov` is `InitDecodeFailed`; a progressive MP4 served as a media segment is the fatal `UnsupportedContainer{non_fragmented_mp4}` (HLS F9a).
Self-initializing Media Segments (no Initialization element, 5.3.9.5.2) are demuxed alone.

7.6 **Init handling.** One init per Representation, identity = resolved URL without query + byte range (HLS Round 7 F2 rule); inits for all selected Representations are fetched CONCURRENTLY (`ResourceId::Init{rep}` carries identity, unlike HLS's one-at-a-time
`ResourceId::Init`); a segment that arrives before its init is held (bounded, `PendingKeyedOverflow` analogue); each segment is demuxed as `init ++ segment` with a fresh `Fmp4Demux` (the `hls-runtime` `demux_and_emit` and multimux `dash_pull` pattern); `Tracks` is re-announced only when the
`TrackSpec` set changed; a Representation switch fetches the new init before its first segment and re-announces. There are no keys (D10), so no key-per-run analogue.

7.7 **MPEG-2 TS segments in DASH** (`urn:mpeg:dash:profile:mp2t-main:2011`, 23009-1 8.6, segment formats 6.4): out, typed `UnsupportedContainer{"mp2t_segments"}`. DVB-DASH and the DASH-IF live profiles are ISOBMFF-only; the hls-runtime TS path is entangled with `EXT-X-DISCONTINUITY`
handling, so reuse would first need a `TsSegmentDemux` extraction: a follow-up if anyone asks.

## 8. Test plan

Rules carried over from the repo: real data first (no inline-bytes-only suites), every assertion mutation-checked and the mutation actually recompiled, fixtures only through `test_fixtures`, tests that can fail (no skip-on-missing for
public fixtures), the r8 fault harness designed in from the first adapter PR.

### 8.1 Real data inventory

| Source | What it gives | Status |
|---|---|---|
| `fixtures/dash/manifest.mpd` + `init-stream*.m4s`/`chunk-stream*-0000N.m4s` | real ffmpeg static timeline presentation (video 90 kHz, audio 44.1 kHz); first chunk tfdt 0, second 90 000; `S@t=2070` while tfdt=0 (section 11) | committed |
| `fixtures/dash/manifest-inheritance.mpd` | Period/AdaptationSet/Representation attribute inheritance (5.3.9.1) | committed (derived, documented) |
| `fixtures/dash/livesim2/segtimeline*_{a,b}.mpd` | real DASH-IF livesim2 dynamic MPDs, `$Time$` and `$Number$`+timeline, huge absolute `t`, `startNumber=895967208`, `UTCTiming http-xsdate`; the a/b pairs are consecutive refreshes (`publishTime` 23:34:38 -> 23:34:46) = a real MPD-update sequence | committed |
| `docs/specs/dash/23009-1/annex-g-examples.md` | the spec's own example MPDs (G.1...) as parser inputs; W-c extracts them to `fixtures/dash/annex-g/*.mpd` with provenance | P0 |
| IOP v4.3 Table 10 (`ch04-02`) | a complete numeric vector: 43 s, SPD 15 s, MBT 5 s, TSB 25 s, 5 s segments => `k1=1, k2=9, SAST[k]=START+5k, SAET[k]=SAST[k]+30 s`, init available `START..START+75 s` | exact-value test of `address::plan` + `timing::availability` |
| CMAF segments from repo media | `transmux`'s `DashPackager`/CMAF segmenter over `fixtures/ts/h264_aac_40s.ts` and `fixtures/mp4/cmaf/*` produce contiguous segments with known continuous tfdt; the loopback origin serves them under a generated dynamic MPD (template, timeline, multi-Period, SegmentBase+sidx, SegmentList variants) | built in W-d |
| DASH-IF livesim2 (BSD licence, `LICENSE.md` verified in the module) | third-party origin: `go install github.com/Dash-Industry-Forum/livesim2/cmd/livesim2@v1.15.2` BUILT here (29.6 MB binary, `--help` runs); the module ships 33 MB of test assets (`cmd/livesim2/app/testdata/assets`: `testpic_2s`, `testpic_6s`, `timeline_number_2s`, `testpic_2s_low_delay`, ...) usable with `--vodroot`. W-g served it locally (`--vodroot` at those assets) and played 30 s from it, six shapes at once | plan below |
| public livesim2 / testassets.dashif.org | fetched ONLY small samples into `/private/tmp/claude-501/dashassets` (not committed): MPDs for `testpic_2s`, `segtimeline_1`, `startrel_-20`, `utc_direct-head`, one segment, one init, `time.txt` headers; the service is rate-limited (`livesim2-requests: 1 (max 10000)`), so never CI | used for section 11 |

livesim2 e2e (W-g, built and run): `broadcast-client/tests/e2e_livesim2.rs`, a blocking `reqwest` client as a dev-dependency only (no TLS: the server is on loopback); the test spawns the pinned binary (`LIVESIM2_BIN`, else `PATH`) on a free port with `--vodroot` at the module's assets (`LIVESIM2_VODROOT`, else the Go module cache of the pinned version), checks the `Dash-If-Livesim2` response header names the pinned version, then drives six real `Session`s in parallel for 30 s with the real monotonic clock and `set_wall_clock` from `SystemTime`. Shapes (all with a loopback `UTCTiming`, never `time.akamai.com`, which the default livesim2 MPD names): `utc_head` under `PreferHost` with an exact host clock (strict: no segment requested before it exists, no `425`/`404`), `utc_head` under `Auto`, `utc_direct`, `segtimelinenr_1`, `startrel_-20` and `periods_20`. Asserts: tracks announced, samples on both tracks, at least 10 segments per track in order with no gap or repeat, at least three MPD requests (two refreshes), no request loop (4 per second per URL), no errors, no `Discontinuity`. Finding: livesim2's `startrel_N` moves `availabilityStartTime` with every request, so each refresh is a Reset; that shape asserts the rejoin is announced instead of continuity. `utc_xsdate`/`utc_iso` produce no `UTCTiming` element; `utc_head` (HEAD on the server's own `/static/time.txt`) is the hermetic one. Locally the test skips loudly when livesim2 is absent and fails under `REQUIRE_LIVESIM2=1`; the `dash-e2e` CI job installs the pinned module with Go and sets it.

### 8.2 Layers

1. **Unit/vector:** every formula of sections 4.3 and 5.3 with hand-computed numbers (the Table 10 vector; `$Number$` with `startNumber` 0 and 1; PeriodStart rules 1-5; timeline with `@n`, gaps, negative `r`; sidx anchor; template widths and invalid identifiers; BaseURL join/override; `urlparam:2014`).
2. **Differential:** (a) new parser vs `transmux::dash_parse` on the shared fields for every `DashPackager` configuration and every committed MPD, and the URL lists vs `Mpd::resolve_segment_url`; (b) lazy timeline expander vs `SegmentTimeline::resolve` as oracle (property: equal whenever no `@n`);
   (c) vs `multimux` `dash_pull::plan_timeline`/`build_plan` on the subset multimux supports (this test lives in `multimux`, the highest crate, after W-f: the cross-crate-tests rule); (d) generator round trip: `DashPackager` output of the source fixture, served by the loopback origin, consumed by `DashClient`, must yield the byte-identical `Sample` sequence of a direct `Fmp4Demux` of the same segments.
3. **Whole-client sans-IO** with the `World` of section 3.6: join, refresh sequence (the livesim2 a/b pairs), Period transition continuous/connected/unsignalled, representation switch, audio switch, early/gone/expired cadence, give-up VOD/live, MPD reset.
4. **Session level** (`broadcast-client/tests/dash_*.rs`): oracle = a bare `DashClient` driven with the same bodies; chunking invariance of the Event sequence (1 B, 1316 B, 64 KiB body chunks); auth scoping (credentials never reach a CDN origin or a UTC server), per-request redirects, cookies, `extra_headers`, Retry-After table, bounded bodies, request timeouts, in-flight window table, gate (VOD lead, live lead) from real segment durations, jump detector, `select_variant`/`select_audio_rendition`, status sequence, terminal rules after fatal.
5. **Loopback real wire:** one test in `multimux` (hyper, W-f) for the socket path; the in-crate fake host covers the rest.
6. **Fuzz:** `fuzz_mpd_parse` (bytes -> parse -> plan iteration, no panic, bounded work), `fuzz_address` (structured arbitrary `SegmentInfo` + times: monotone numbers, no overflow, bounded expansion), `fuzz_session_dash` (config header + scripted World + arbitrary response bodies mutated from annex-G/livesim2/ffmpeg seeds + real fMP4 segments; the r6 `Config` knob randomisation), seeds from `fuzz/seed-corpus.py`; every fuzz finding gets a verbatim-artifact regression test that is revert-verified.

### 8.3 Fault-injection matrix (r8 template, designed in at W-e, widened at W-g)

Template: `.../scratchpad/r8/r8_matrix.rs` (fault enum `F` x source `Src` x permanent/transient, a `Sim` that owns the session, a simulated clock, per-cell `classify` with hang, request-loop, panic and sample-order/duplication checks). New file `broadcast-client/tests/support/dash_matrix.rs`.
Sources: static timeline (ffmpeg fixture), static template, live `$Number$`, live timeline (a/b refresh), live multi-Period, live + separate audio set, on-demand sidx. Faults: the 22 of r8 (500, 503, 429, 429-huge, 404, 401, 403, conn-fail, reset-mid, short-body, truncated-chunk, stall, no-response, garbage, HTML, empty, huge, late, dup-response, unknown-conn, closed-twice, redirect-loop)
PLUS: early-404, early-425, gone-410-in-window, expired (client fell behind), host-clock-skew (+-5 s, +-60 s), clock-step, UTC garbage/timeout/rtt-too-large/`Date`-missing, MPD 304, stale `publishTime`, truncated MPD, AST changed, type flip dynamic->static, Period removed, Representation vanishes, alternate-BaseURL 503, 200-ignores-Range, sidx/size mismatch, huge MPD. Each cell asserts: no panic, no hang (terminal state or bounded progress), request rate bounded
(r8 found 604 requests in 602 s: the cap here is <= 4 per second per request class), delivered numbers contain no duplicate and no unexplained gap, transient faults recover without loss, permanent ones end with the specified `ErrorKind`.

### 8.4 Mutation-resistant list (each mutant must be killed by a named test; the mutation must be shown to recompile)

| Feature | Mutants |
|---|---|
| `$Number$` | `startNumber + k` instead of `+ k - 1`; width ignored; `$$` kept doubled |
| Availability | `+ duration` dropped from `avail_start`; ATO sign; TSB omitted from `avail_end`; `<=` vs `<` at the boundary |
| Timeline | negative `r` count `floor` vs `ceil`; `@n` ignored; `@t` gap treated as contiguous; PTO not subtracted |
| Live edge/join | newest off by one; join picks segment AFTER `T_join`; audio joined by number not time |
| Clock | midpoint sign; 500 ms Date half-second dropped; lower-trust source replacing higher; step threshold; resync period |
| Refresh | `MUP=0` unthrottled; stale `publishTime` accepted; 304 treated as failure; conditional headers omitted |
| Errors | 425 counted; early window boundary; Expired retried; give-up count `>` vs `>=`; Retry-After ignored/unclamped |
| Periods | continuity signal ignored; `Discontinuity` missing/duplicated at a boundary; init not refetched |
| URLs | query propagated when it must not be (and not when `useMPDUrlQuery`); absolute BaseURL not overriding; alternate not rotated; blacklist not cleared |
| Containers | non-fragmented MP4 accepted; `media_fragment` bypassed; init without `moov` accepted |

## 9. Work breakdown (agent-sized PRs)

Order is by dependency then value-for-effort; each PR follows the repo loop (baseline commit, brief with ungameable real-fixture gates, audit on own gate run, adversarial review by a different model). Sizes: S < 1 day, M 1-2, L 3-5, XL > 5 agent-days.

| Step | Scope | Depends on | Size | Exit gate (beyond the 6-gate suite + Apple-target check) |
|---|---|---|---|---|
| P0 fixtures + spec prep | extract Annex G MPDs to `fixtures/dash/annex-g/`; produce REAL on-demand (`SegmentBase`+sidx, `-single_file`), `SegmentList`, multi-Period and period-continuity MPDs+media with GPAC `MP4Box`/ffmpeg (provenance + licence quoted; `fixtures/dash/PROVENANCE.md`); transcribe RFC 8470 (425) and the xs:dateTime accepted subset into `docs/specs/`; re-read the private verbatim 5.3.9.5.3, 5.3.9.6, 5.3.2.1 against the public notes | none | S-M | every fixture parses with an external oracle (MP4Box/ffprobe) and has provenance; no hand-made media |
| W-a0 rename (Q1) | mechanical `hls_*` -> neutral names in `Config` + tests (sed-only test edits) | none | S | `git diff` of tests is pure renames; all green |
| W-a shared extraction | section 2: `source/fetch/` pieces, `hls.rs` rebased on them; `container-probe` segment checks + hls-runtime uses them | W-a0 | M + S | tests dir diff EMPTY (after W-a0), mutation list killed, hls-runtime tests unchanged |
| W-c dash-runtime core | crate skeleton (README, CHANGELOG, examples >= 2, docs/), `mpd` model/parse/inherit, `address` (template validation, lazy timeline, list, sidx), `timing` (`UtcClock`, availability, live edge, PeriodStart), selection; fuzz `mpd`+`address` | P0 | L | annex-G all parse; Table 10 vector; differentials 8.2(a)(b); fuzz run clean 10 min each; CRATE-ACCEPTANCE items except release lane |
| W-b clock + config surface in broadcast-client | `Config::dash`, `Session::set_wall_clock`, `ClockSource`, `Event::ClockSynced`, new `ErrorKind`s, `Config::validate`, README host contract (HEAD method, clock) | W-c (types), W-a | M | public-API diff = exactly section 3.5 table; unit tests; `label_coverage` |
| W-d1 `DashClient` static/VOD | actions/outputs, selection, init/index/segment fetch, demux, periods, switching, failures classification (no clock) | W-c | L | generator round trip 8.2(d) static; ffmpeg fixture byte-equal to `Fmp4Demux` oracle; on-demand sidx fixture |
| W-d2 `DashClient` live | clock ladder, UTC sampling, refresh, join, availability gating, early/expired, MPD reset, dynamic->static | W-d1, W-b | L-XL | World tests 3.6; livesim2 a/b refresh pairs; every row of the 8.4 table |
| W-e adapter + Session | `source/dash.rs`, `SourceKind::Dash` + `Auto` `.mpd`, Variants/AudioRendition mapping, gate integration, errors, matrix harness v1 | W-d2, W-a | L | session suites 8.2(4); matrix on static + live-number + live-timeline with r8's 22 faults |
| W-g fuzz + matrix + docs + release lane | `fuzz_session_dash`, matrix widening (all sources and DASH faults), `release-dash-runtime.yml` (copy of `release-hls-runtime.yml`), CHANGELOG, `docs/release-notes/`, README coverage table, `docs/crates-index.toml`, `CRATES.md`, `tests/no_handroll_guard.rs` + a `no_clock_guard.rs` banning `SystemTime::now`/`Instant::now`/`jiff::Timestamp::now` outside `#[cfg(test)]`, livesim2 e2e + `dash-e2e` CI job | W-e | M-L | docs.rs green; the e2e actually serves and plays 30 s; guards bite |
| W-f multimux adoption | `dash_pull` rebased on `dash-runtime` (keeps the tokio loop and the media-plane mapping); removes `jiff::Timestamp::now` and the first-Period/first-Representation limits; listed behaviour changes (policy-core style); differential 8.2(c); loopback wire test | W-d2 (published) | L | multimux suite green; behaviour-change list accepted by the owner |
| W-h audit rounds | adversarial rounds like r5-r8: (1) formulas and clock, (2) failure/refresh/period state machine, (3) session mapping + fault matrix; fixes in one merged brief per round | after each of W-d2, W-e, W-f | 3 x M | each round: findings fixed, a test per finding, matrix re-run |

Release order (CI-only, owner sign-off, no workstation publish): `container-probe` minor, `hls-runtime` patch, `dash-runtime` 0.1.0, `broadcast-client` 0.1.0, then `multimux` (W-f) and `multimux-cli`. `transmux` is untouched, so no epoch moves there.
What must precede `broadcast-client` 0.1.0 publishing: strictly only the non-additive changes, i.e. `Config` (new `dash` field, the Q1 renames, the Q3 `#[non_exhaustive]`); everything else in section 3.5 is additive on non-exhaustive enums and could follow in 0.1.x, but W-b lands the whole surface so 0.1.0 does not ship a clock-less API that DASH then bolts onto.
`SourceKind::Dash` itself only lands with W-e (no stub variant that returns `UnsupportedScheme`).
Needs spec clarification FIRST: nothing blocks (the public notes cover every clause used); P0 closes the three gaps it lists (on-demand and period fixtures, RFC 8470, xs:dateTime subset).

## 10. Risks and open questions

### 10.1 Owner-level questions (each with a recommendation)

| # | Question | Recommendation |
|---|---|---|
| Q1 | Rename the shared `hls_*` knobs (`hls_backoff_ms`, `hls_max_segment_failures`, `hls_max_inflight_segments`, `hls_request_timeout_us`, `hls_segment_timeout_us`, `max_playlist_bytes`) to source-neutral names before 0.1.0? | Yes (W-a0): a published `hls_` prefix on DASH knobs is an API wart forever; now it is a sed |
| Q2 | `Event::Variants` stays the hls-runtime `MasterSelection` with DASH mapped into it (`VariantInfo.url` = init URL), or a neutral source-independent type now? | Map now (hosts already speak it, `select_variant`/`select_audio_rendition` keep working); neutral type as a breaking 0.2 if a second adaptive source (e.g. MSS) appears |
| Q3 | Make `Config` `#[non_exhaustive]` now (hosts then build it with `Default` + field assignment; later knobs become additive)? | Yes: DASH is the third source to add knobs and it will not be the last |
| Q4 | Ship DASH 0.1.0 without chunk-level LL-DASH delivery (segment-granular, safe degradation)? | Yes; chunk delivery is a 0.2 story shared with LL-HLS parts |
| Q5 | Clock ladder default: UTCTiming > host clock > MPD `Date` (`Auto`), `PreferHost` available? | Yes; UTCTiming is the packager's own authority and costs one overlapped round trip |
| Q6 | No automatic ABR in 0.1.0 (policy at start + `select_variant`, bandwidth exposed)? | Yes; the player owns adaptation (zenith) |
| Q7 | CENC/CBCS decryption, MPEG-2 TS and WebM segments, subtitles, seeking, EventStream surfacing: all out of 0.1.0 with typed errors? | Yes |
| Q8 | `dash-runtime::mpd` is consume-only (no serializer), an exception to the symmetric Parse/Serialize invariant, justified by the differential against `transmux`'s generator (the same exception `dash_parse` has)? | Accept; revisit if the module is hoisted to `broadcast-dash` |
| Q9 | CI gains a Go toolchain job to run livesim2 (BSD, pinned `@v1.15.2`) as a hard e2e gate? | Yes; skip-loudly locally, hard-fail in CI like the private-fixtures rule |
| Q10 | multimux behaviour changes when it adopts the core (all Periods and every selected set instead of the first Period / first Representation; clock from UTCTiming instead of the host wall clock; early-404 cadence)? | Accept, listed per the policy-core precedent in W-f |

### 10.2 Risks

- **R1 clock errors.** A wrong clock requests too early (404/425, absorbed by `NotYetAvailable`, never counted) or joins too far behind (`Expired`/rejoin). Mitigations: ladder, uncertainty reported, A.7 re-sync on the first in-window 404, simulated-skew matrix.
- **R2 `Date` rung.** Edge caches can serve a stale `Date` (IOP 4.7.2 warns for `http-head`); it is the last resort and flagged by `ClockSource::MpdDate`.
- **R3 SSRF / hostile MPD.** BaseURL, segment and UTCTiming URLs are chosen by the MPD's server (the same property as the `resource_metadata` note in the client design). A player tolerates it; a SERVER-side pull (multimux) must not fetch `169.254.169.254` because an MPD says so: `DashConfig::cross_origin_urls=false` restricts every derived URL to the MPD origin, and multimux sets it by default (W-f; note the existing `dash_pull` has the same exposure today). UTC and MPD bodies are size-bounded (UTC bodies 4 KiB), MPD bounds in 4.2, timelines expand lazily, `MUP=0` is floored at 1 s.
- **R4 host clock semantics.** The host's monotonic clock must keep counting across device sleep (Apple: continuous clock) or the host must call `set_wall_clock` after wake; the `now_us` jump catch-up rules of the client design (audit #1484) bound the damage, documented in the host contract.
- **R5 two MPD parsers.** `transmux::dash_parse` and `dash_runtime::mpd` coexist until W-f; the differentials are the guard; deprecating `dash_parse` is a later, separate transmux decision.
- **R6 A.11 redirect rule.** Using a redirect target as the permanent update URL (A.11) can pin a temporary signed URL; the spec is followed, `Location` wins when present.
- **R7 `S@t` vs tfdt.** Clock/addressing code must use `S@t` for `$Time$` and presentation arithmetic, never tfdt (section 11 item 2); a regression test uses the ffmpeg fixture whose `t=2070` differs from tfdt 0.
- **R8 livesim2 drift.** Pinned version, e2e asserts structure not timing constants.

## 11. Spec quirks that affect implementation

1. **`$Number$` formula (5.3.9.5.3).** The URL bullet prints the replacement as `(k-1) + (kstart-1)` (`kstart` = `@startNumber`, default 1) while the `@duration` bullet one page later writes `(k-1) + NumberStart` (`docs/specs/dash/23009-1/05-3-9-5-segment-information-derivation.md`, quirk note). Real content, checked this session:
   livesim2 v1.15.2 `testpic_2s` (`startNumber="0"`, `duration="2"`, no `@timescale`): `V300/896501965.m4s` has `tfdt = 161 370 353 700 000 / 90 000 = 1 793 003 930 s = 896 501 965 x 2`, i.e. `MPDstart(Number) = (Number - startNumber) x duration`; the same server answers a number 4 ahead with `425 "earl"` and a number 80 behind with `410 Gone`.
   The committed ffmpeg fixture (`startNumber="1"`): `chunk-stream0-00001.m4s` tfdt 0, `-00002` tfdt 90 000. Both packagers use `Number = startNumber + k - 1`, the `NumberStart` reading, which is also what Annex A.3.3 (`(i-1) + @startNumber`) and IOP 4.3.2.2.7 print (`Address = @startNumber` for the first segment, then +1). The `(kstart-1)` reading would yield number -1 for livesim2's `startNumber=0`. Implement `startNumber + k - 1`; pinned by a test on both real captures.
2. **`S@t` is a presentation time, not tfdt.** The ffmpeg fixture has `<S t="2070" d="90000" r="2"/>` for video while the first segment's `tfdt` is 0 (the 2 070-tick difference is the composition offset); `$Time$` substitutes `S@t` (5.3.9.4.4 Table 21, 5.3.9.6.1) and the spec's sidx rule equates `S@t` with the sidx `earliest_presentation_time`. Never derive `$Time$` or Period-relative times from tfdt.
3. **Annex A.3.3 pseudo-code counts `r+2` segments per `S`** (`r[s]=@r+1` then a loop `j=1..r[s]` after emitting one) and uses an undefined `d[0]`; the normative 5.3.9.6.1/5.3.9.6.2 give `r+1`. Follow the clause, not the informative code.
4. **Availability end.** 5.3.9.5.3 `SegAvailEnd = SegAvailStart + MPDduration + TSB`, Annex A.3.4 `TavailEnd = AST + PeriodStart + start + TSB + 2 x duration`, IOP `SAET = SAST + TSB + SDURATION` are the same quantity (SegAvailStart already contains one duration).
5. **IOP 4.3.4.3 multi-Period list is unreachable in its third branch** and mixes Period end with start; implement the per-Period `SAST/SAET` model of 4.3.2.2 (the IOP notes say so).
6. **Epoch zero.** livesim2 uses `availabilityStartTime="1970-01-01T00:00:00Z"` and `publishTime="1970-01-01T00:00:00Z"`: unix 0 is a VALUE, never "absent".
7. **`@timescale` default 1**: `testpic_2s` has `duration="2"` with no timescale; do not require it.
8. **Early/expired status codes.** Specs say "4xx" (A.7) or 404 (IOP 4.8.5); livesim2 uses 425 (early, body `earl`) and 410 (expired). Classification accepts 404/410/425 (section 5.4).
9. **`Date` resolution** is whole seconds (livesim2 `time.txt` `date: ... 08:39:04 GMT`), `http-xsdate` carries milliseconds (`time.akamai.com/?iso&ms` returned `2026-10-26T08:39:04.952Z`): uncertainty bookkeeping in 3.3.
10. **`@startNumber`/`@endNumber` width**: unsignedInt in Ed.5, unsignedLong in Ed.6 (`ED6-SCHEMA-DIFF.md`); parse as u64 (livesim2 numbers already reach 8.9e8).
11. **Ed.6 differences that matter** (schema only, prose paywalled, `ED6-SCHEMA-DIFF.md`): `MPD@type="list"` (fatal typed), `S@d` optional and `SegmentTimeline/Pattern`, `S@p/@pE/@ssp`, `@k` (segment sequences: Representation dropped), `Period@minBufferTime` (read), `Location` as a complex type (read the text), `ContentSteering` and `RequestParam` (ignored). Everything else is identical.
12. **5.3.9.4.4 table cross-references are wrong** ("Table 20"/"Table 16" for identifiers; Table 21 is operative); an invalid template drops the containing Representation, not the MPD.
13. **5.8.5.7:** the HTTP-VALUE sentence lists `ntp` where `http-ntp` is meant; IOP 4.7.2 omits `sntp`. Both `ntp` and `sntp` are skipped here; `http-ntp` is implemented.
14. **`MPD@minimumUpdatePeriod=0`** means every announced segment is already available and the MPD may change before each request (IOP 4.4.3): hence the 1 s refresh floor rather than a literal zero.

## 12. W-d1 implementation notes (static client, as built)

`dash-runtime::client` implements sections 1, 5 and 7 for static MPDs. Where it is narrower than, or differs from, the text above:

| Topic | As built | Section |
|---|---|---|
| Entry points | `new`, `poll(now_us)`, `next_output`, `on_mpd`, `on_mpd_failure`, `retry_mpd`, `on_init`/`on_index`/`on_segment(now_us, id, bytes)`, `on_failure(now_us, id, Failure) -> Result<FailureClass>`, `retry`, `skip`, `select_representation`, `set_mpd_base`. `ResponseMeta`, `on_mpd_unchanged` and `Action::WaitUntil` belong to the clock and refresh work and arrive with W-d2 (the enums are `non_exhaustive`). `now_us` is accepted everywhere and unused by a static presentation. | 1.2 |
| Verdicts | A rejected body returns `Error::Rejected { id, reason, class }` and a reported failure returns `FailureClass`; both carry the verdict synchronously. `Output::Fatal` is the only verdict that is also an output, because it ends the presentation. `skip` marks a parked segment delivered with a `Discontinuity` (used by an adapter for dynamic playlists; a static presentation gives up instead). | 5.4 |
| Counting | `max_segment_failures`/`max_init_failures` follow `broadcast-client`'s `verdict`: give up when the consecutive count reaches the cap, `0` and `1` make the first failure final, `u32::MAX` never. A `425` is counted for a static presentation (no availability window exists to be "too early" of). | 5.4 |
| Missing Period tail | 404/410 on the last segment of a Period whose planned duration is under 50 ms is `FailureClass::EndTail` (GPAC `multiperiod-restart`: `Period@duration` 6.013 s over 6.000 s of media). Everything else about a missing segment stays counted. Rationale in the crate README. | 5.5 |
| Periods | Periods are fetched in order; the next Period's inits are requested while the previous one is in flight. Boundary is silent only when every selected lane carries `period-continuity` naming the previous Period's `@id`, on the same `AdaptationSet@id`, with an unchanged init identity (URL without query, byte range). `period-connectivity` is not given separate handling and is treated as a discontinuity. | 5.5 |
| Outputs | Emitted per Period: notices and `TrackDropped`, `Manifest`, `PeriodChanged`, `Discontinuity` (not for the first Period or a continuous boundary), `Init` per lane (not for a continuous boundary), `Tracks` when the set changed, `Samples`. `DisplayChanged` is not produced (the fMP4 demuxer carries no display events; `HlsClient` does not emit it for fMP4 either). | 1.2 |
| Track ids | 1-based, video lane then audio lane, from the number of modelled tracks of each lane's init; samples map to them by the demuxed `tkhd.track_ID`, so a muxed init yields consecutive ids. A Representation switch keeps the lane's ids; tracks beyond the lane's original count are dropped. | 5.1 |
| Init handling | One fetch per distinct (URL without query, byte range), owned by the first Representation that asked; all inits of a Period are fetched concurrently and the Period starts delivering when they have arrived. A Representation with no `Initialization` is dropped (`DropCause::NoInitialization`): self-initializing segments are not supported yet. | 7.6 |
| In-flight window | `max_inflight_segments` counts requested and arrived-but-undelivered segments; the lane whose next segment starts earliest on the Period timeline is requested first; delivery is in request order per lane. Init, index and MPD requests are not counted. | 5.3 |
| BaseURL alternates | Candidates ordered by `@dvb:priority`; after 2 consecutive counted failures of a Representation's resource the next candidate is used (wrapping, so no blacklist is kept). `@weight` is not used. | 7.2 |
| Switching | `select_representation(i)` is valid while the Period being played is the one being fetched; it continues from the segment containing the next unrequested media time and marks a switch that is not `@segmentAlignment` with `@startWithSAP` 1 or 2 as a `Discontinuity`. Selecting an audio set is not implemented yet. | 5.6 |
| Origin pin | `cross_origin_urls = false` compares scheme, host and port of every derived URL with the document URL in force (after `set_mpd_base`); a Representation whose init, index or first segment leaves the origin is dropped (`DropCause::CrossOrigin`), any later escape is `FatalKind::UnsupportedAddressing`. | 6.4, R3 |
| Ranges | A response to a ranged request must have exactly the requested length; the adapter slices a `200` that ignored `Range` (shared `body.rs`) before delivering. | 7.3 |
| Configuration | `DashClient::new(url, DashConfig, PullLimits)`: `DashConfig` is W-b's (DESIGN 6.4, `cross_origin_urls` is read from it); `PullLimits` carries the knobs 6.4 lists as shared with HLS (selection policy, MPD limits, body cap, in-flight window, segment and init failure caps) which the session maps from its own config. Not a second DashConfig. | 6.4 |

### 12.1 W-d2 implementation notes (live client, as built)

`dash-runtime::client` plays dynamic MPDs (sections 3, 5.2 to 5.5). The decisions the text above left open, and the points where the code differs from it:

| Topic | As built | Section |
|---|---|---|
| Entry points | `on_mpd(now, bytes)` stays as the no-headers convenience; `on_mpd_with(now, bytes, &ResponseMeta)` is the real entry (`ResponseMeta{sent_us, received_us, date, etag, last_modified, final_url}`), `on_mpd_unchanged(now, &meta)` is the `304`, `on_utc_response`/`on_utc_failure(UtcId)` answer `Action::FetchUtcTiming{id, scheme, url, method}`, `set_wall_clock(now_us, unix_us)` is the host anchor, `awaits(id)` and `state_size()` are diagnostics. `Action::FetchMpd` carries `validators: MpdValidators` (the held `ETag`/`Last-Modified`). | 1.2, 3.5 |
| No `Action::WaitUntil` | `next_wake()` (absolute, monotonic) replaces it: after polling to `None` the caller waits for a response or for that instant. A wake is never returned in the past: a join that is impossible (every listed segment already expired on the client's clock) arms a forced refresh and a re-sync instead of a busy timer. | 1.2 |
| MPD failures | `on_mpd_failure` returns `FailureClass::Retry{attempt, kind}` (consecutive MPD failures, never `GiveUp`); the caller backs off and calls `retry_mpd`. After a refresh failure or a rejected body no refresh is scheduled until `retry_mpd`. | 5.2 |
| `UTCTiming` candidates | `direct` and the `http-*` schemes in document order, `http-head` last (IOP 4.7.2); `@value` lists split on whitespace, only `http`/`https` URLs, resolved against the document URL; `ntp`/`sntp`/unknown skipped with `Notice::UtcSchemeUnsupported`; with `cross_origin_urls = false` another origin is skipped with `Notice::UtcCrossOrigin`. A `direct` value is sampled at the MPD exchange midpoint without a request. A round trip over `utc_max_rtt_us` is discarded and the same candidate retried once; an unreadable, empty, oversized or non-UTF-8 body goes to the next candidate; `utc_timeout_us` is enforced by the core (a late answer is `Error::UnrequestedUtc`). A refused `HEAD` (`405`/`501`) is re-issued as `GET`, only the `Date` is read. | 3.3 |
| Ladder | At MPD acceptance: the response `Date` is offered as `MpdDate`; if the policy and the elements call for it the sampler runs with the join gated for at most `utc_wait_us`; when it yields, fails or times out the ladder resolves to the best anchor present, or `FatalKind::ClockUnavailable`. `PreferHost` with a host anchor never samples (not at the join, not on a re-sync, not on a forced one) and a later `set_wall_clock` replaces a `UTCTiming` anchor. `Output::ClockSynced` is emitted when the ladder resolves and when the source of the anchor changes, not for the provisional anchor while the sampler runs. | 3.4 |
| Re-sync | Every `utc_resync_us` after the last sample, and forced (at most once per 30 s, `FORCED_RESYNC_MIN_GAP_US`) by the first `Missing` answer of a Representation, by an early answer the client's own clock says should have been served, and by an impossible join. A step adopted from a re-sync, from `set_wall_clock` or from the `Date` forces an MPD refresh. | 3.3, 5.4 |
| Gating | Every segment is gated by the formula, timeline-listed ones included. Section 4.3's last bullet (a listed segment is available at publish time) is not used: the IOP 4.7.3 MUST is cheaper to meet than to argue, and the test world asserts it. `availability_margin_us` delays every request; `use_availability_time_offset` moves the request earlier by the summed `availabilityTimeOffset` but delivery stays whole-segment (D9). | 4.3, 5.3 |
| Classification | `425` is always early. `404`/`410` is early when the client clock is before `avail_start + early_grace_us`, expired when after `avail_end`, else `Missing`. Early is not counted; the core re-requests at 250 ms doubling to 1 s from `poll` itself (`FailureClass::NotYetAvailable{retry_at_us}`, nothing for the caller to do). `Missing` is counted with the caller's backoff; a `5xx`, a transport error or a rejected body is counted as in a static presentation; a BaseURL rotation forces a refresh. | 5.4 |
| Give-up | A live segment that reaches `max_segment_failures` is skipped by the core, not by the caller: `FailureClass::SegmentSkipped`, `Output::SegmentLost{id, status}` and a `Discontinuity` (consecutive ones collapse into one). An init or index still ends the presentation (`InitFetchFailed`). A static presentation is unchanged. | 5.4 |
| Expired and rejoin | An expired segment (seen before the request or answered after it) makes the lane skip to the oldest segment still in the window with `Notice::FellBehind` and a `Discontinuity`; the third consecutive expiry on a lane, or a window with nothing in it, rejoins the live edge. Deliveries reset the count. | 5.4 |
| Refresh | Next fetch at `exchange midpoint + max(minimumUpdatePeriod, min_mpd_refresh_us)`; a forced refresh (the listed segments ran out, the first in-window error of a Representation, a step, a rotation) is at most once per floor, and after exhaustion no sooner than the last segment's duration. A byte-identical body is detected by comparison with the held body (up to `max_bytes`), an older `publishTime` is ignored and retried at the floor, an MPD without Periods or without `availabilityStartTime` is ignored. Update URL: first `Location`, else the redirect target the caller reports in `ResponseMeta::final_url`, else the original; the redirect target is also the base for relative references. | 5.2, A.11 |
| Presentation end | A dynamic presentation is open (waits for refreshes instead of ending) while `minimumUpdatePeriod` exists and `mediaPresentationDuration` does not. A Period whose end is known but whose lists do not yet reach it (last segment end within `MISSING_TAIL_MAX_US` of the end) stays open until 5.4.1 makes them final: `max(minimumUpdatePeriod, 2 x maxSegmentDuration)` after the Period end. Without that rule a Period closed by an early announced successor was abandoned with its last segments unlisted. | 5.2, 5.5 |
| Periods | The Period a join starts in is the last one whose start is not after `utc - delay`; while the UTC sample is in flight the inits of the Period the provisional clock (else the last Period) selects are fetched, and a wrong guess is dropped unstarted. A refresh remaps live runs by `Period@id` (else PeriodStart), `AdaptationSet@id` (else position) and `Representation@id`; a vanished current Representation is re-selected and switched like `select_representation`; an unfinished Period that left the MPD is a reset. `Output::PeriodChanged::index` is the MPD position at that moment (it shifts when earlier Periods expire); `id` is stable. Finished Period runs and the inits only they referenced are dropped. | 5.5 |
| Reset | A changed `MPD@id`, a changed `availabilityStartTime`, the `urn:mpeg:dash:reset:2016` descriptor or a removed unfinished Period drops the timeline, rejoins the live edge and announces a `Discontinuity` (the departure from 5.4.2's "play the old MPD as long as possible" stands); the requests of the old timeline are no longer awaited (`awaits`). | 5.2 |
| Dynamic to static | The presentation ends with the plans closed by the new MPD: the remaining segments are played, `EndOfStream` follows the last, no refresh is made. | 5.2 |
| Bounds | `done` remembers at most 4096 delivered resources (an older one answers `UnrequestedResource`, not `DuplicateResource`); everything else is bounded by the window and the Periods in play (`StateSize`, tests `live_longrun`). | 8 |
| Known limit | The in-flight window is shared by the lanes (W-d1). A lane stuck on a permanently missing segment holds slots until it is given up (about 16 s with the simulated backoff) and the other lane is starved meanwhile. Per-lane windows would fix it and are a W-e/W-h item. | 5.3 |
| Host clock vs `Date` | `Auto` trusts a host clock over the response `Date`; a host clock beyond the time-shift buffer from the packager's, with no `UTCTiming`, plays wrongly (ahead: waits and then skips with a `Discontinuity`; behind: late). Cross-checking the two sources was considered and left out: `Date` is whole seconds and edge caches serve stale ones. | R1, R2 |


## 12.2 W-e implementation notes (broadcast-client adapter, as built)

`broadcast-client/src/source/dash.rs` is the adapter over `DashClient` (section 6); `Session` drives HLS and DASH through one internal interface
(`source/adaptive.rs`). The points the design left open or got wrong, with the decision taken:

| Topic | Decision | Section |
|---|---|---|
| `select_audio` did not exist | Added to the core as an additive API (`DashClient::select_audio(Option<usize>)`, `Error::NoSuchAudio`): `Some(i)` switches the audio lane to the highest-bandwidth Representation of set `i` of the latest `Output::Manifest`, `None` stops requesting audio (segments already requested are still delivered; the adapter filters them), a later `Some` starts it again. It shares `switch_lane` with `select_representation`, which is behaviour-identical. The live refresh keeps a manual audio choice in `PeriodRun::selection`. Tests: `dash-runtime/tests/client_select_audio.rs`. | 1.2, 12 |
| Core already counts failures and owns the window | The adapter does not use `fetch::{FailureCounter, Window, Lanes, RetryGate}`: the core's `max_segment_failures` / `max_init_failures` (mapped from `max_segment_failures` / `max_key_failures`) and `max_inflight_segments` do that job, and a single gate would stall MPD refreshes, UTC samples and segments behind each other. The adapter keeps per-resource retry instants (`retries`, one MPD instant) fed by `FailureClass::Retry { attempt }` and the shared `Backoff` / `retry_after_us`, and reuses `ConnAlloc`, `Deadlines`, `BodyAcc`. | 2, 6 |
| `Output::DisplayChanged` | The core never produces it (12 W-d1); there is nothing to map. | 6.1 |
| `Event::Variants` renditions | All selectable audio sets of the Period, in MPD order, not filtered by the chosen variant's `audio_group` (every DASH set has its own group); `select_audio_rendition(i)` indexes that list. `VariantInfo::url` is the init URL (empty when the Representation has none). | 6.1, 6.2 |
| Manual choices across Periods | A manual variant is re-applied in each later Period by `(bandwidth, resolution, codecs)`, a manual audio set by `(name, language)`; otherwise the Period's own pick stands. The Period's `Manifest` is emitted after the first segment requests of that Period, so the choice takes effect from the next unrequested segment. | 6.2 |
| `auto_select_audio = false` | At the first `Manifest` with audio sets the adapter calls `select_audio(None)`; audio inits and segments already in flight are fetched and their samples filtered. | 6.2 |
| `select_variant` and the playout gate | The HLS session resets the gate on a variant change; DASH does not (the switch is at a segment boundary and the core emits `Discontinuity` itself when it is not aligned). | 6.1 |
| Timeouts | `UTCTiming` requests use `dash.utc_timeout_us` (not `request_timeout_us`) and a 4 KiB body cap; init, index and MPD use `request_timeout_us`; segments `segment_timeout_us`. | 6.4 |
| Redirects | The adapter follows `max_redirects` per request with the HLS rules and reports the final URL of an MPD in `ResponseMeta::final_url`; the core applies A.11 (`Location` over redirect over original). `set_mpd_base` is not used. | 7.4 |
| `Status::NotFound` | Only for a 404/410 of an MPD request before any MPD was accepted. | 6.1 |
| MPD errors | `MpdError::Unsupported` becomes `UnsupportedAddressing { reason }` (fatal when no MPD was accepted yet); every other parse error is `MpdParse { reason }` (`invalid_utf8`, `xml`, `not_mpd`, `invalid`, `too_large`), non-fatal once per kind until samples arrive, fatal at `max_key_failures` consecutive. A transfer cut short of its `Content-Length` is a transport failure, not a parse error. | 6.3 |
| Drops | Every `TrackDropped` becomes one non-fatal error per distinct `ErrorKind` (so all non-addressing drops collapse into one `TrackDropped { track_id: 0 }`, `fourcc: None` because the output carries no codecs); `UnsupportedEncryption` drops are `UnsupportedEncryption { method: "content_protection" }`, and when nothing else plays the fatal `no_playable_representation` is reported as `UnsupportedEncryption` (the HLS user message). | 6.1 |
| Notices | `UtcSchemeUnsupported` and `UtcCrossOrigin` (added by W-d2) map to the features `utc_timing_scheme` and `utc_cross_origin`; the `UnsupportedEncryption` notice is not mapped (the drop carries it); `FellBehind` is `LiveEdgeLost`, once per run until samples arrive. | 6.1 |
| `reconnect()` | Builds a fresh `DashClient` (the host anchor re-applied), announces `Event::Discontinuity` first, keeps cookies and credentials; ignored after `Ended` or a fatal error. | 6.2 |
| One UTC element, no other source | A single `UTCTiming` element that fails once, on a session with no host clock and an MPD response without `Date`, is fatal `ClockUnavailable` (the ladder of section 3.4 as built). Real origins send `Date` (IOP 4.8.5) and hosts call `set_wall_clock`; the fault matrix sources send `Date`. | 3.4 |
| Known limit kept | The in-flight window is still shared by the lanes (12.1); not changed here. | 5.3 |

## 12.3 W-f implementation notes (multimux `dash_pull` on the core, as built)

`multimux/src/source/dash_pull/` is a host over `DashClient`, laid out like the other adapters: `session.rs` (the media-plane `IngestSession`, no clock, no sleep), `drive.rs` (the tokio loop), `stitch.rs`
(one sample timeline), `legacy_plan.rs` (the old planner, compiled for tests only) and the tests. The decisions the design left open, and the points it got wrong or incomplete:

| Topic | Decision | Section |
|---|---|---|
| Where time is read | `multimux::clock::HostClock` (`mono_us`, `unix_us`) is the only reader of the machine clocks in the pull path; the loop reads it and hands the core every instant as an argument, a test hands it a clock with a known skew. `tests/no_handroll_guard.rs` fails if `Instant::now`, `SystemTime::now`, `Timestamp::now`, `UNIX_EPOCH` or `.elapsed()` appears in the production files of `dash_pull`. The brief's "no `Instant::now` in dash_pull" is read as "not in the module", which the seam makes literally true. | 3.1, 6 |
| Host anchor | `set_wall_clock(mono, unix)` at open and again every `DashConfig::utc_resync_us` (600 s); `DashClockPolicy::Auto`, so a `UTCTiming` answer outranks the host clock, which outranks the MPD `Date`. | 3.4 |
| Origin pin | `DashConfig::cross_origin_urls = false` by default (design R3); a route opts out with `DashPullRoute::with_cross_origin_urls(true)` / `"allow_cross_origin": true`. A refused Representation is dropped, and when none is left the attempt ends. | 6.4, 10.2 R3 |
| Limits | MPD body 4 MiB (the core default; the old module allowed 64 MiB), segments 64 MiB (`MAX_HTTP_BODY_BYTES`), `UTCTiming` bodies 4 KiB, a window of 4 segment requests (half of `MAX_INFLIGHT_FETCHES`, leaving room for the MPD, inits and `UTCTiming`). | 6.4 |
| Failure caps and backoff | The core classifies, the loop paces. Delay 0.5 s doubling to 4 s per consecutive failure of one resource (multimux's own constants: the shared `hls_backoff_*` knobs belong to `broadcast-client`). A segment is given up after 4 consecutive failures (a static presentation ends the attempt, a live one skips it with a `Discontinuity`), an init or index after 4, an MPD refresh after 8 in a row. The first MPD failing, or being rejected, ends the attempt at once: the route supervisor owns reconnect, and there is nothing to keep playing. `Retry-After` is not honoured (it was not before). | 5.4, 6 |
| `401` | An `Unauthorized` answer from any request ends the attempt with `MultimuxError::Auth` at once, as before; every other status goes to the core as `Failure::Status`. | 7.4 |
| Output mapping | `Established` with the first accepted MPD (as before: the connection is up, there is no media handshake); the first `Output::Tracks` is `NewProgram`, later ones `TracksChanged`; one program (`ProgramId(0)`) for the whole presentation; `Manifest`, `PeriodChanged`, `Init`, `Notice`, `ClockSynced` are logged, not published. `SegmentLost` increments `multimux_pull_fragment_abandoned_total`. | 6.1 |
| Sample timeline across Periods (gap in the design) | The core delivers the container timeline and leaves rebasing to the host (section 5.5). A GPAC multi-Period MPD restarts each Period's clock at zero, which a `Trunk` feeding segmenters cannot take. `stitch.rs` removes every `Output::Discontinuity`: the first sample after it is placed where the previous content ended (the later of the tracks' ends), one offset for every track so A/V alignment inside the Period survives, rounded per track once per discontinuity. A continuous boundary and a presentation that never signals a discontinuity are untouched. A lost segment is closed rather than kept as a gap, a deliberate choice for a segmenter. | 5.5 |
| Real defect found (not a design gap) | `transmux::Fmp4Demux` ignored the init movie's `trex`: GPAC's real multi-Period fixture (`fixtures/dash/multiperiod-restart/`) has fragments whose `trun` lists sizes only, and every such sample came out with duration 0 at the fragment's first `dts`, and sync-flagged. Fixed in `transmux` (a `tfhd` that leaves a default out takes the `trex` value; a `trex` size of 0 is not applied), regression tests on the real fixture. dash-runtime, hls-runtime and broadcast-client suites pass unchanged on the fix. | 7.6 |
| `Date` resolution | `http-head` is timed from a `Date` header of whole seconds, placed in the middle of the second: a pull timed by it can ask up to about a second before a segment exists (a 425 the core absorbs without counting). Millisecond sources (`http-xsdate`) stay within the round trip. The tests assert both bounds. | 3.3, 11 item 9 |
| Not done | Per-lane in-flight windows (12.1 "Known limit" stands: a lane stuck on a missing segment holds slots until it is given up, about 3.5 s with these constants), automatic ABR, `Retry-After`, `403` handled as anything but a counted failure. | 5.3 |

### Behaviour changes against the pre-core `dash_pull` (Q10 accepted, policy-core precedent)

| Area | Before | Now |
|---|---|---|
| Clock source | A `jiff::Timestamp::now()` read whole seconds, used only to place a `$Number$` live edge; no clock at all for timelines. | A monotonic clock plus the host UTC clock, anchored at open and every 600 s; for a dynamic MPD the ladder `UTCTiming` > host clock > response `Date`. A dynamic MPD with no usable source, or without `availabilityStartTime`, ends the attempt with a typed error (the old code played from `@startNumber`). |
| Where playback joins | `$Number$`: three segments behind the host-clock edge. Timeline: the last three listed segments. | `suggestedPresentationDelay`, else the larger of 4 s and three maximum segment durations, behind the UTC edge (clamped inside the time-shift window), per Representation on its own timeline. A join delivers that much media at once. |
| Early requests | A `404` on a dynamic MPD retried every 500 ms, up to 240 times (about two minutes), then abandoned. | No segment is requested before its availability start; a `425`, or a `404`/`410` within 1 s of it, is retried at 250 ms doubling to 1 s and never counted; a `404`/`410` inside the window is a counted failure with backoff; a segment past the window is skipped to the oldest one still in it. |
| Failures | Any non-success answer, timeout or oversize body ended the attempt (a tolerated live `404` aside). | Counted per resource and retried with the backoff above; the caps decide (4 segment, 4 init or index, 8 MPD refresh). `401` still ends the attempt as `Auth`. |
| MPD errors | Any unparsable MPD, first or refresh, ended the attempt. | The first one ends it; a refresh that does not parse is retried, and eight in a row end it. |
| Periods | Only the last Period of the MPD. | Every Period, in order, back to back, with the timeline stitched across boundaries. |
| Tracks | The first Representation of every AdaptationSet that had a `SegmentTemplate`. | One video AdaptationSet (Role `main`, else the first) at the highest bandwidth the policy allows, and one audio AdaptationSet, per Period. Text, MPEG-2 TS, WebM, trick-mode, dependent and protected sets are dropped with a typed reason in the log. |
| Addressing | `SegmentTemplate` with `@duration`, or with a `SegmentTimeline`; one `BaseURL`. | Also `SegmentList`, `SegmentBase` + `sidx` (byte-range requests), `BaseURL` alternates at every level, `$Bandwidth$`, widths, `@endNumber`, `S@n`, `urn:mpeg:dash:urlparam:2014`. Typed unsupported: segment sequences, `type="list"`, `data:` URLs, open-ended ranges, hierarchical `sidx`. A Representation without `Initialization` is dropped (it was a fatal error for the route). |
| Requests | Plain `GET`s, always the configured MPD URL. | Conditional MPD refreshes (`If-None-Match`, `If-Modified-Since`, `304` absorbed), byte ranges, `HEAD` for `http-head`; the redirect target of an MPD is the base of its relative URLs and the refresh URL (A.11); `UTCTiming` requests never carry credentials. |
| Refresh cadence | After the plan was exhausted, `minimumUpdatePeriod` (default 2 s) from the last fetch. | `fetch time + max(minimumUpdatePeriod, 1 s)` whatever the plan holds, plus forced refreshes (plan exhausted, first in-window error, clock step, BaseURL rotation). |
| Origin | Any URL the MPD names was fetched. | Only URLs on the MPD's origin unless the route sets `allow_cross_origin`. |
| Protected content | Demuxed as it came (or failed deep in the demuxer). | `MultimuxError::Encrypted` when no Representation can be played for lack of decryption. |
| Limits | MPD up to 64 MiB; one segment request per Representation. | MPD up to 4 MiB; a window of 4 segment requests. |
| Error kinds and text | `dash-pull: mpd parse: ...`, `... HTTP 404`, `... not ready yet`. | `dash-pull: mpd rejected: ...`, `dash-pull <kind>: HTTP n`, `dash-pull: presentation cannot continue: <FatalKind>`, `... needs a UTC clock ...`; `Auth` and `Encrypted` kept. |
| Metrics | `multimux_pull_fragment_abandoned_total` counted a segment dropped after 240 tolerated `404`s. | Same name, counted when the core gives up a live segment (`SegmentLost`); no metric was added or renamed. |
| Samples | Container timeline, sync flags from `trun` only. | The same, with `trex` defaults applied (the transmux fix above) and the timeline stitched at discontinuities. |
| Public API | `DashAction`, `DashResourceId`, `RepIndex`, `DashIngestSession::with_clock` (wall-clock closure), `InputSpec::DashPull { url, auth }`. | Removed: the first four. `DashIngestSession` is fed `DashInput`, hands out `dash_runtime::client::Action`, takes `set_wall_clock(mono, unix)`. `InputSpec::DashPull` gains `allow_cross_origin` (a struct-variant field: a constructor or an exhaustive pattern without `..` breaks, a multimux minor under 0.x rules); new `DashPullRoute::with_cross_origin_urls`, `dash_config`, `pull_limits`. |
