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
