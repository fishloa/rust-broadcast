---
topic: ISO/IEC 23009-1:2022 (Ed.5) clause 5.4 - Media Presentation Description updates and MPD Reset (own-words implementer notes)
clauses: 5.4.1, 5.4.2
printed_pages: 106-108
pdf_pages: 114-116
verbatim_source: private/specs-verified/dash/23009-1/05-4_to_5-7.md
status: own-words notes derived from the private verbatim conversion (pdf2md hybrid, exit 0, rendered-page verified, independently verified); not a copy of the spec text
---

# 5.4 MPD updates

## Scope

Only a dynamic MPD (`MPD@type="dynamic"`) may change during the presentation. Typical changes: more segments listed per Representation, a new Period, new Segment locations, or the end of the presentation.

## 5.4.1 Rules for an updated MPD

Publication of the new version:

- at the original MPD location when the MPD has no `Location` element (if `Location` elements exist: at every location they list);
- optionally *also* as an MPD Patch document announced by one or more `PatchLocation` elements; the patch is refreshed at the same moment as the MPD, so its timing follows the MPD update timing (see [`05-15-mpd-patch.md`](05-15-mpd-patch.md)).

Identity continuity (shall):

| Item | Must stay the same across updates |
|---|---|
| `MPD@id` (if present) | yes |
| `Period@id` | yes, unless the Period itself was removed |
| `AdaptationSet@id` | yes, unless its Period was removed |
| Representation with same `@id` in the same Period | attributes/elements functionally equivalent; Segments functionally identical and with the same indices |

Validity window:

- No `@minimumUpdatePeriod`: no update is expected; `MPD@mediaPresentationDuration` or the last `Period@duration` shall be present and the MPD stays valid to the end of the presentation.
- With `@minimumUpdatePeriod`: let *Texp(i)* be the presentation end time if version *i* is the last version, otherwise (wall-clock time at which version *i* is superseded by *i+1* at the MPD location) + `@minimumUpdatePeriod`. Version *i* shall remain valid until *Texp(i)*, meaning
  1. every Segment whose availability start time is before *Texp(i)* is available from its announced location at its availability start time;
  2. every Representation has a Segment with availability start time *Tavail* < *Texp(i)* whose duration is at least *Texp(i) - Tavail*.

Notes in the clause (informative): the real duration of the last Segment may be unknown until fetched (it can be shorter than previous ones, e.g. last in a Period or with a SegmentTimeline); a client that cannot know *Texp(i)* can lower-bound it as (wall-clock time of its MPD or patch request) + `@minimumUpdatePeriod`; condition 2 guarantees enough media to reach *Texp(i)* for a client playing each Segment at its earliest time; an MPD may have no Period or only an early available Period and then updates supply the first PeriodStart; an update may only extend validity without changing content; with `@minimumUpdatePeriod` = 0 all Segments with availability start before the request time are available.

## 5.4.2 MPD Reset

A server that cannot continue the previous timeline may restart its content at the same MPD URL. If it does, it shall signal this with a descriptor on MPD level:

| Property | Value |
|---|---|
| Carrier | EssentialProperty or SupplementalProperty (the print calls them "Essential/Supplemental Descriptor") |
| `@schemeIdUri` | `urn:mpeg:dash:reset:2016` (print spells the attribute `@schemeIdURI`) |
| Effect | the new MPD is exempt from the 5.4.1 update rules |
| Essential flavour | a client that does not understand the signal may terminate playback |

Client expectations (should/expected): keep using the previous MPD for as long as possible, but no longer than the new MPD's (`availabilityStartTime` + Period start); then play from the new MPD; conceal any media gap. If that sum is already before the current presentation time the new MPD is valid immediately and the client may cancel decoding/presentation of the overlapping previous media.

## Quirks

- The first sub-bullet of 5.4.1 is garbled (two location conditions run together without a verb).
- 5.4.1 note 7 refers to NOTE 2 for the `@minimumUpdatePeriod` = 0 consequence.

Verbatim text: see `private/specs-verified/dash/23009-1/05-4_to_5-7.md` sections 5.4.1-5.4.2 (printed pp. 106-108).
