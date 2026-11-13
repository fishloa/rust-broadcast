# dash-runtime 0.4.0

_Released 2026-11-14._

Breaking-class release (0.3 -> 0.4): breaking API changes are listed below, and the crate's sibling dependencies move to new caret epochs.

Version: 0.3.0 -> 0.4.0.

## Breaking changes and migration

None in this crate's own API. The bump follows the `broadcast-common` epoch move.

## Fixed

- Whole-second `xs:dateTime` centred half a second later with half-second uncertainty.
- A `sidx` reference of size 0 still advances the subsegment timeline.

## Also in this release

### Changed
- BREAKING: `DashifIngestSession::new()` takes a `DashifIngestConfig`; the `Default` impl is removed (#2514).
- BREAKING: `DashClient::new(url, dash, limits)` takes a `client::DashClientConfig` (`DashClientConfig::new(url).with_dash(..).with_limits(..)`) (#2514).
- HTTP `Date` parsing moved to `broadcast_common::http_date` (re-exported as `timing::http_date`).
- `mpd::base_url` joins each `BaseURL` chain entry through `broadcast-common::url_ref::resolve`, the workspace's one guarded resolver, and `mpd::base_url::first_forbidden_char` is the shared control-character check plus its own whitespace rule; a space is now refused too, matching the shared resolver (issue #2458).
- `timing::clock::Anchor::utc_at`/`mono_for_utc` map through `broadcast_common::time::MonoUtc` instead of repeating the anchor arithmetic; the saturation to `i64`/`u64` stays here, and the clock still never reads a time source.
- BREAKING: `SegmentPlan::index_at_or_before`, `available_from_us` and `available_until_us` return `Result<Option<_>, AddressError>` instead of folding an addressing failure into `None`.
- BREAKING: the MPD model, parser and `SegmentTemplate` grammar live in the new `broadcast-dash` crate. `mpd::model`, `mpd::descriptors`, `mpd::inherit` and `address::template` are re-exports of `broadcast_dash` (so `dash_runtime::mpd::Mpd` and friends keep resolving), and `limits::Limits` is a re-export of `broadcast_dash::limits::Limits`. Code that imported from `dash_runtime::mpd`, `dash_runtime::address::template` or `dash_runtime::limits` keeps compiling; new code should depend on `broadcast-dash` directly. `error::{Limit, MpdError, Unsupported}` are re-exports of `broadcast_dash::error`. The `xs:dateTime`/NTP readers moved to `broadcast_dash::xs_datetime` (still at `timing::datetime`) and the HTTP `Date` reader gained its own module, `timing::http_date`; `TimingError::DateTime` now carries only `broadcast_dash::xs_datetime::DateTimeError`, and a bad `Date` value is `TimingError::HttpDate(..)`.
- The MPD reader's namespace and hostile-input bound tests lost when the reader moved out of transmux are back, against `dash_runtime::mpd::Mpd` (issue #2340), and the DASH validator golden now has a test that a refused MPD is recorded as `dash-parse-error`, never as `clean`.
- BREAKING: `address::template::substitute` returns `Result<String, TemplateError>` and `SegmentPlan::get` returns `Result<Option<SegmentRef>, AddressError>`. A template naming `$Number$`/`$Time$` the caller has no value for, or a template that does not parse, is now an error instead of silently dropping the identifier (yielding a URL such as `chunk-.m4s`) or an empty string, which fetched the wrong segment.

### Added
- `ingest::DashifIngestConfig` (`max_object_bytes` 64 MiB, `max_buffered_object_bytes` 8 MiB, `buffered_budget_bytes` 64 MiB; `with_*` setters; `DEFAULT_*` consts) and `DashifIngestSession::config`: the limits a DASH-IF ingest host enforces now live with the session instead of in multimux (#2514).
- `ingest` module: DASH-IF Live Media Ingest (Interface-1) sans-IO engine (`DashifIngestSession`, `classify`, `RequestMeta`, `IngestObject`, ...) moved from `multimux` (issue #2480).
- `mpd::base_url` (`resolve`, `resolve_chain`, `first_forbidden_char`): RFC 3986 §5 URL-reference and `BaseURL`-chain resolution over `url::Url::join`, moved from `transmux`, whose only caller was this crate; `mpd::urls` now resolves through it instead of `transmux` (#2355).
- `address::template::match_url`: reverse segment-template matching (an uploaded segment URL to its representation id and `$Number$`/`$Time$`/`$Bandwidth$`), moved here from transmux so dash-runtime is the one MPD reader (#2340).
- `PresentationType::List` and `Period::imported_mpd` (`ImportedMPD`): the `MPD@type="list"` value and the Period element ISO/IEC 23009-1 6th edition adds. `DashClient` refuses a list MPD with its typed unsupported error rather than playing it as static.

### Fixed
- `TimelineIndex::build` still computed (and could fail with `AddressError::Overflow` on) the clock advance after the last `S`, where the new value is never used; it is now computed only when another `S` follows.
- `TimelineIndex::build` computed the run span (`@d * count`) even when a negative-`@r` run was cut by the next `S@t` and the span was unused, so a legal timeline with a huge `@d` failed with `AddressError::Overflow`; the span is now computed only when it advances the clock.
- A `SegmentTimeline` `S` with a negative `@r` followed by an `S@t` that falls inside its last repeat now ends that last Segment at the `S@t` instead of letting it overrun the next run; the overlap made a later Segment's `segment_available_from` time earlier than its predecessor's (found by the `dash_address` fuzz target).
- A segment-address failure after the initial plan (for example an `index * duration` overflow that index 0 does not hit) drops the Representation with `DropCause::Addressing`, or ends the presentation with `FatalKind::UnsupportedAddressing` when it was the only track, instead of the client silently waiting forever.
- A list presentation (`MPD@type="list"`) is refused with `FatalKind::Unsupported(Unsupported::PresentationTypeList)` instead of `MpdInvalid { reason: "presentation_type_list" }`; the refusal now carries the typed error every other unsupported feature uses.
- `init_reference` now propagates a non-`IndexRequired` plan error as `DropCause::Addressing` instead of treating the representation as having no init segment; `note_exhausted` drops the affected lane with the typed cause instead of silently excluding it from the tail-wait computation.
- `@duration` on a parent level and a `SegmentTimeline` on a child level (or the reverse) are refused as `InheritError::DurationAndTimeline`, as two attributes on one level already were: our reading of ISO/IEC 23009-1 §5.3.9.2.1 (`private/specs-verified/dash/23009-1/05-3-9-segment-information.md`) is that a Representation's effective Segment information may not carry both `@duration` and a `SegmentTimeline`, and the child's timeline used to silently replace the inherited duration.
- A Representation inherits its AdaptationSet's `SegmentTemplate` timeline when it declares none of its own; the segment plan used to see no timeline there (found by pinning media-doctor's DASH validator on the ISO 23009-1 examples, where example G22 produced 18 timeline findings instead of 3).

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `container-probe` 0.3 -> 0.4, `transmux` 0.30 -> 0.31.
- New sibling dependencies: `broadcast-dash` 0.1, `media-plane` 0.11.

---

Published from tag `dash-runtime-v0.4.0`.
