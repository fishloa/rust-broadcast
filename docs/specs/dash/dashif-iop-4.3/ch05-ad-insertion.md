---
source: DASH-IF Interoperability Points v4.3 (15 Nov 2018), private/specs/dashif_iop_v4.3_guidelines.pdf
clauses: "5 Ad Insertion in DASH (5.1-5.6)"
pdf_pages: "143-164"
printed_pages: "122-143"
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch05-ad-insertion.md
verified_against_render: true
notes_original_wording: true
---

# IOP v4.3 clause 5 - Ad insertion (client-relevant notes)

Structured notes in our own words. Exact wording, the example MPD, figures and page-level detail are in the private verbatim file above. "p." = PDF page.

## 5.1 Concepts a client must implement (pp.143-147)

| Tool | What the client does | Cite |
|---|---|---|
| Remote element (`@xlink:href` + `@xlink:actuate`) | Element not fully in the MPD; `onLoad` = resolve at MPD parse time, `onRequest` = resolve later, ideally near the Period's expected playout / segment download time (no normative timing). | 5.1.3.2, p.144 |
| Resolution | HTTP GET the `@xlink:href` URL. Error response, or a syntactically invalid entity: the in-MPD element is removed. Valid entity: replaces the in-MPD element. A resolved entity may itself carry `@xlink:href` with `onRequest` for repeated resolution. | p.144-145 |
| `urn:mpeg:dash:resolve-to-zero:2013` | Special `@xlink:href` value: no request is issued, element is removed. Used when a remote element can be resolved only once per MPD version. | p.144 |
| Only channel to the resolver | The URL itself; splice time or cue data must be encoded in it. | p.145 |
| Period timing | Period start explicit (`Period@start`) or derived from previous durations; real duration of Period i = start(i+1) - start(i); a client starts Period i+1 at its stated start even if Period i media is longer. `Representation@presentationTimeOffset` gives presentation time at PeriodStart. | 5.1.3.3.1, p.145 |
| Segment availability | Period-level `BaseURL@availabilityTimeOffset="INF"` means segments of that Period stay available at least as long as the current MPD is valid (stored ads). Also settable on AdaptationSet/Representation. | 5.1.3.3.2 |
| Seamless transitions | MPD guarantees nothing across Periods; use period continuity/connectivity signalling (see clause 3.2.12 notes). | 5.1.3.3.3 |
| Period labelling | Period-level `AssetIdentifier` tells ads from content (progress bar, random access). | 5.1.3.3.4 |
| Events | MPD EventStream events or inband `emsg` boxes; inband `emsg` boxes sit at the very start of a Segment. Three client-handled events (MPD Validity Expiration, MPD Patch, MPD Update) all mean "refresh the MPD"; all other events go to the application or are dropped. | 5.1.3.4, p.145-146 |
| MPD updates | Needs `MPD@minimumUpdatePeriod` for synchronous refresh; `MPD@publishTime` versions MPDs (later publishTime supersedes). Inband events can trigger asynchronous updates. | 5.1.3.5 |
| Session identity | Cookies (RFC 6265) are the simple mechanism. | 5.1.3.6 |
| Callback events | Client promises one HTTP GET to a URL at an offset from PeriodStart; response body ignored. | 5.1.3.7 |

Workflow classes: linear (break length known, must be filled) vs elastic (break length open, may be zero); architectures: server-based (generic client, all ad logic server side) vs app-based (an app drives one or more clients) (5.2, p.147).

## 5.3 Server-based architecture (pp.148-158)

- One ad = one `Period`. Periods that will be interrupted should carry explicit `Period@start` rather than durations, so new Periods can be inserted without editing old ones; the client starts the next Period at the stated time (5.3.2.1.1).
- Remote Periods resolve into one or more Periods; in elastic flows an untaken break resolves to a zero-duration Period without AdaptationSets, so clients must tolerate zero-length Periods (5.3.2.1.2, 5.3.3.3).
- For `onRequest` resolution the client needs enough time to (a) fetch the updated MPD and (b) dereference before the splice; the guideline notes `onRequest` is operationally harder for live (5.3.2.1.2).
- Timing consistency across dereferencing: simplest is for the MPD update to add `Period@duration` to the currently playing Period, and the resolver to put `Period@duration` in each returned Period. Alternative: pass the wanted first `Period@start` in the XLink URL (query parameters, 5.3.5; the parameter set is defined by SCTE 214-1: splice time, SCTE 35 cue, syscode) (5.3.2.1.3, p.157-158).
- `AssetIdentifier`: Periods with the same value should have identical AdaptationSets, initialization segments and DRM info, so init data can be reused and playback stays seamless when inserted Periods have zero length (5.3.2.2).
- MPD events/updates: DASH events with scheme `urn:mpeg:dash:event:2013` carry the MPD Validity Expiration trigger; if several `emsg` boxes are in one segment the validity-expiration one must come first. Conditional GET (304) reduces polling cost (5.3.2.3).
- Opaque events (companion ads, tracking) are passed to an external module via event `@schemeIdUri` (5.3.2.4).
- Linear workflow facts (5.3.3.2): MPD generator and packager both watch the SCTE 35 feed; MPD generator adds a remote Period whose `Period@start` is the translated `splice_time()` and puts cue-derived parameters into `Period@xlink:href`; packager may prepend an MPD Validity Expiration `emsg` to the first available segment (optimisation; presentation_time_delta plus segment start must leave the client time to fetch the MPD and resolve). Repeated warnings for one splice are ignored after the first. Cancelation = newer MPD without the Period (or with zero duration) plus an `emsg`. Early termination handled like a break start; easier when only `Period@start` (no duration) is used. Informational SCTE 35 cues should not trigger validity expiry. Stateless cue translation = whole `splice_info_section` as base64url (RFC 4648) in the XLink URL.
- Ad decision flow (Fig. 15, p.154): client GET MPD, GET XLink URL, resolver asks ad decision server, returns Period(s), client GETs ad segments. Resolved Periods may be cached by the resolver for a time T.
- On-demand/VoD (5.3.3.3-5.3.3.6): cue locations known so a static MPD with all remote elements and `Period@start` works; elastic breaks need `Period@duration`; do not dereference remote Periods until playout needs them, and on random access use durations plus asset identifiers to pick the start Period. Slates = in-MPD Period that also has XLink attributes (replaced only if resolution succeeds). Blackouts/alternative content use the same default-content idea.
- Reporting: Period (local or remote) may carry an EventStream whose event holds an IAB VAST 3.0 `Ad` element, scheme `http://dashif.org/identifiers/vast30`; alternatively callback events. Inband callback events must not be used (5.3.3.7, p.156).
- Example MPD (Fig. 16, pp.156-157) is a dynamic `isoff-live` MPD with two mid-roll remote Periods that each contain default content and `BaseURL@availabilityTimeOffset="INF"`; first break passes SCTE 35 cue and time as query parameters, second passes proprietary parameters `u`, `z`. It is not copied here. Note (our observation, p.156-157): the printed XML is not well-formed as written (identifier wrapped as `$Band-`/`width%`, typographic quotes, a bare `&`, undeclared `xlink:` prefix, `<!-` followed by an em dash in one comment); treat it as illustrative only.

## 5.4 App-based architecture (pp.158-162)

- Each ad decision yields its own MPD; an MPD holds either main or inserted content; the ad management module sits in the client and an app drives two clients (main/ads) (Fig. 17). One decoder may suffice with careful conditioning but the app must be able to save and restore player state.
- SCTE 35 cues travel as events: inband `emsg` or in-MPD `Event`.
  - Inband (Fig. 18): `scheme_id_uri="urn:scte:scte35:2013:bin"`, `value=1001`, `timescale=90000`, `presentation_time_delta=540000`, `duration=5400000`, `id=0`, message_data = binary cue (example = 6 s warning, 60 s break).
  - In-MPD (Fig. 19): `EventStream@schemeIdUri="urn:scte:scte35:2014:xml+bin"`, `Event` with `presentationTime` relative to Period start (example 54054000 at 90000 = 600.6 s) and a `scte35:Signal/scte35:Binary` base64 payload.
  - Normative carriage: SCTE 214-1 (MPD) and 214-3 (inband). Skeleton only; payload bytes not reproduced.
- Packager turns cues into inband user-defined events (cue in `emsg` message_data; start-of-segment time plus `presentation_time_delta` must map to the splice time); the client forwards events to the app via registered callbacks; the app talks to the ad server (e.g. VAST, out of scope), gets an MPD URL, pauses main content, plays the inserted content, then resumes; pause/resume interface relates to 23009-1 Annex C.4 (MPD fragment interface). Elastic workflows come for free; for VoD prefer a static MPD with in-MPD SCTE 35 events (5.4.3).

## 5.5 Extensions (pp.162-163)

| Item | Rule |
|---|---|
| `AssetIdentifier` | Required on main content, optional on inserted content; unique IDs recommended. |
| Schemes | `urn:eidr` (value = canonical EIDR); Ad-ID: `urn:smpte:ul:060E2B34.01040101.01200900.00000000` (SMPTE 2092-1, value = canonical full Ad-ID); DASH-IF fallback `urn:org:dashif:asset-id:2014` with a MovieLabs ContentID URN as value (same for all parts of an asset); private schemes allowed. |
| Do not use | On one-off assets wholly inside one Period, on Periods with no non-remote AdaptationSet, on zero-length Periods. |
| `Period@xlink:actuate="onRequest"` | Must not be used when `MPD@type="dynamic"` (5.5.2, p.163). Compare 5.3.2.1.2 which allows it with care; the two statements differ. |
| App-driven cues | SCTE 35 events only; the PTS offset inside SCTE 35 is ignored, only DASH event timing counts (5.5.3.1). |
| Event schemes recommended (5.5.3.3) | `urn:scte:scte35:2013:bin` (inband, complete binary section); `urn:scte:scte35:2014:xml+bin` (MPD, base64 `Signal.Binary`); `http://dashif.org/identifiers/vast30` (VAST 3.0 response); `urn:mpeg:dash:event:callback:2015` (callbacks). |

## 5.6 Interoperability aspects (pp.163-164)

Server-based claim = follow 5.3.2, 5.5.1 and 5.5.2 (offerings and clients); app-based = tools of 5.4/5.5 are available but the ad logic is outside the DASH client.
