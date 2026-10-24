---
title: "Clauses 10.12-10.13 Error reporting and play-position reporting (notes)"
source: ETSI TS 103 285 V1.4.1 (2023-09), private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
clauses: 10.12-10.13
pdf_pages: 96-100
printed_pages: 96-100
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/ch10-7-reporting.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# ETSI TS 103 285 V1.4.1 - Clauses 10.12-10.13 Error reporting and play-position reporting (notes)

Structured notes in the project's own wording (the standard's text is not reproduced; short quoted phrases only). Strength words are kept: **shall** = mandatory, **should** = recommended, **may** = optional. Verbatim, page-verified text: the private file named in the front matter. Back to [INDEX](INDEX.md).

Context: the DASH metrics mechanism is ISO/IEC 23009-1 Annex D / 5.9 ([dash-mpd-rules](../../../../specs/rules/dash-mpd-rules.md)). Worked examples: [annex-d-e](annex-d-e-examples.md).

## 10.12.1-10.12.2 Overview and support (p.96)

Providers learn about client problems through a new metric list `DVBErrors` (entries = error events) plus a reporting scheme that turns each list entry into one HTTP GET query string; an optional `@probability` samples the player population. Players shall support the scheme and `DVBErrors`. Low-latency players shall also support the Table 30 metrics:
| Metric | Required scope |
|---|---|
| `PlayList` | `representationid` and `subreplevel` fields may be omitted; no entries needed at a Representation switch and no `representation switch` stop reason; `trace` needs only one AdaptationSet |
| `RepSwitchList` | in full |
| `HttpList` | only MediaSegment transactions; a key with `type=MediaSegment` or no type both mean MediaSegment; `tcpid` may be omitted if `TcpList` is unsupported |
Other metrics in the MPD may be ignored by the player, but it must not accidentally ignore supported combinations.

## 10.12.3 Reporting descriptor (p.96-98)

- Scheme: `Reporting@schemeIdUri="urn:dvb:dash:reporting:2014"`, `@value` 1. Descriptor needs `@reportingUrl`; if it is missing the descriptor (and a Metrics element whose only descriptor it is) may be ignored. Reports are also limited by any `Range` element (23009-1 5.9.2) and by `@probability`.
| Attribute (DVB schema) | Type | Default | Meaning |
|---|---|---|---|
| `@reportingUrl` | absolute `http`/`https` URI (pattern `https?:.*`) | - | where reports go; mandatory for this scheme |
| `@probability` | positive integer 1-1000 | 1000 | thousandths of players that report |
- Sampling: 1000 means always a reporting player; otherwise decide at random with chance probability/1000 (e.g. draw 1-1000, report if not above the value); the decision is fixed for the MPD's lifetime across updates; non-reporting players report nothing. Becoming a reporting player is itself an event (`S00`). Omit the descriptor or Metrics to disable.
- Making a report: one GET per entry of the top-level list, sent as soon as practical, separate reports for simultaneous entries. Build `key=value` for each field (null/unset = `key=` only), join with `&`, append to `reportingUrl` after `?`. For metrics other than `DVBErrors` add `metricname=<name>` (optional for `DVBErrors`). Response body may be discarded. If a report cannot be sent (bad URL, unreachable, non-2xx) the player stops being a reporting player for that MPD.
- URLs of at least 2047 bytes shall be supported; longer ones are truncated at a `,` or `&` boundary (minimum cut, leaving 12 characters of room) and `&truncated=1` appended.
- Table 31 value encodings: Integer = plain decimal; String = URL-encoded; Real-Time = ISO 8601 UTC date-time; Media-Time = milliseconds as plain decimal; Enum = URL-encoded symbolic name (23009-1); List = comma-separated objects; Object = URL-encoded name=value pairs with `=`, `%`, `,` additionally escaped; Real = optional minus, digits, optional `.digits`, no exponent.

## 10.12.4 DVBErrors metric (p.98-99)

An error event is created when: a 10.8.5 error occurs; the player changes BaseURL; or any other error with a Table 32 code occurs (SSL failure, unknown HTTP status, corrupt media).
Entry fields (Table 32): `mpdurl` (String; original MPD URL, not changed by updates), `errorcode` (String), `terror` (Real-Time), `url` (String; request URL; may be empty for corrupt media or BaseURL change), `ipaddress` (String; address `url` resolved to; same emptiness rule), `servicelocation` (String; the BaseURL's value, for a BaseURL change the one left).
`errorcode` values: HTTP status for HTTP errors and unknown statuses; `SSL` + alert value; `C00` DNS failure; `C01` host unreachable; `C02` connection refused; `C03` other connection error; `M00` ISO BMFF container unparsable; `M01` other corrupt media; `F00` BaseURL changed due to errors; `S00` became a reporting player.

## 10.13 Exposing duration and position to applications (informative, p.100)

- Play position = Period start + durations of preceding segments + durations of samples before the current one in the segment; Period start from `Period@start` or the sum of previous `Period@duration`s back to the last explicit start (23009-1 5.3.2).
- Static MPD without xlink: duration = `@mediaPresentationDuration` (xlink may make it wrong). Dynamic: use `@mediaPresentationDuration` if present; else report "indeterminate" if the API can, otherwise the extent currently available (growing).
- Live: Periods can leave the start of the MPD; retain the total of removed Period durations and keep the original datum. Recompute when an update changes a Period or inserts one.
- APIs should also expose `availabilityStartTime` as first seen and the seekable range (`timeShiftBufferDepth`), should use more than 32 bits for millisecond times (live services), and may use MPD Anchor fields for richer timing.

## Notes on the source (internal inconsistencies)

- 10.12.3.1 says @probability is defined in 10.12.3.2 (it is in 10.12.3.3); 10.12.3.5 points to 10.8.4 for the error list (10.8.5); 10.12.4 points to 11.9.2 for BaseURL-change rules (10.8.2.3).
- The title of Table 32 mentions "ErrorURL" though it defines DVBErrors fields; `1 000` vs `1000` spelling.
