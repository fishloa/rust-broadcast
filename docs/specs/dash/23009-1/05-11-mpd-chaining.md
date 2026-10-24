---
topic: ISO/IEC 23009-1:2022 (Ed.5) clause 5.11 - MPD chaining (own-words implementer notes)
clauses: 5.11.1-5.11.3
printed_pages: 146-148
pdf_pages: 154-156
verbatim_source: private/specs-verified/dash/23009-1/05-11_to_5-15.md
status: derived from the page-verified private transcription; not a copy of the spec text
---

# 5.11 MPD chaining

## What it defines

A descriptor on the MPD that points to a second ("chained-to") MPD to play after the current ("chained-from") one: either **regularly** at its end, or as a **fallback** when the current presentation dies early. Each MPD keeps its own independent media timeline; the client stitches the two into one sequential presentation.

## Regular chaining (5.11.2)

| Property | Rule |
|---|---|
| Signalling | Essential or Supplemental property on MPD level (print: "Essential or Supplementary Descriptor"), `@schemeIdUri` = `urn:mpeg:dash:mpd-chaining:2016` |
| Count | at most one such descriptor per MPD |
| `@value` | whitespace-separated parameters in order (Table 44); if only the first is given no whitespace |
| Types | chained-from and chained-to may each be static or dynamic |
| Typical use | pre-roll ads or a programme sequence: static chained-from, dynamic chained-to; the client joins the dynamic MPD at the live edge, or at the time given by an anchor (annex C.4); the chained-from MPD may contain an XLinked Period for ad personalisation |

Value parameters (Table 44):

| Parameter | Use | Gloss |
|---|---|---|
| url | M | location of the chained-to MPD; after playing the whole chained-from MPD the client downloads it and plays it without delay (like starting a new Period); a response that is not a valid MPD, or an HTTP error, makes the client treat the chained-to MPD as invalid; an anchor on the URL anchors the chained-to MPD (C.4) |
| earliestTimeToResolve | O | recommended earliest time, as a distance in milliseconds before the end of the chained-from presentation, at which the client may fetch the chained-to MPD; absent = no recommendation |

## Fallback chaining (5.11.3)

- Triggers (error conditions): failure to load an MPD (the print says "Failure to of an MPD", a word is missing); failure to download a media (sub)segment of a given earliest presentation time after exhausting all alternatives (all BaseURLs and all Representations of the Adaptation Set) when `MPD@type` is static; failure to retrieve a chained-to MPD; an unspecified error signalled by a Presentation Termination event whose value asks for fallback (see [`05-10-events.md`](05-10-events.md), value 2).
- Signalling: a SupplementalProperty on MPD level (print: "Supplementary Descriptor"), `@schemeIdUri` = `urn:mpeg:dash:fallback:2016`, at most one per MPD; `@value` = one URL or a whitespace-separated list of URLs of the chained-to MPD, listed in order of preference (first = highest).
- The fallback MPD is played only when one of the error conditions makes continued playback of the current presentation impossible.

Verbatim text: see `private/specs-verified/dash/23009-1/05-11_to_5-15.md` section 5.11 (printed pp. 146-148).
