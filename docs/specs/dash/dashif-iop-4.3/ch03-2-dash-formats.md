---
source: private/specs/dashif_iop_v4.3_guidelines.pdf
clauses: 3.2 DASH Formats (3.2.1-3.2.15; 3.2.9 trick mode is in dashif-iop-4.3-trickmode.md)
pdf_pages: 33-49
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch03-2-dash-formats.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# DASH-IF IOP v4.3 — 3.2 DASH formats (notes)

PDF p.N = printed p.N-21. Verbatim: private file above. Trick mode (3.2.9, PDF 41-42): [dashif-iop-4.3-trickmode.md](./dashif-iop-4.3-trickmode.md).

## 3.2.1 Segment format constraints (pp.33-35)

Rules for authors that a client may rely on (strength as in source):
- Fragmented ISO BMFF; each (Sub)Segment is a movie fragment with one track fragment, independently decodable.
- `trex` defaults may or may not be overridden in `tfhd`/`trun`. `moof` shall not use external data references; `default-base-is-moof` shall be set (movie-fragment-relative addressing); `data-offset` is used and `base-data-offset-present` shall not be.
- Within an Adaptation Set: fragmented files, (Sub)Segments aligned for seamless switching; each (Sub)Segment starts with an IDR-like SAP (type 2 or below).
- Segment durations should be nearly equal: per-segment tolerance +-50 % and accumulated deviation +-50 % of the signalled `@duration`; the last segment may be shorter. With a `SegmentTimeline` the timing shall be media-time accurate, with no deviation limit other than the maximum segment duration; varying durations are discouraged and `SegmentTimeline` should mainly signal occasional short segments or gaps. Note 1 recommends On-Demand or `SegmentTimeline` when accurate seek plus fast response is needed; Note 2 says the tolerance may later be replaced by a reference to Cor.3.
- Non-multiplexed Representations only (one media component each). Addressing limited to templates with number or time addressing.
- On-demand profiles: Indexed Media Segment (23009-1 6.3.4.4), `@indexRange` present, exactly one `sidx`. Notes: external Representation Indices are only defined for MPEG-2 TS; the single-sidx rule dates from v3 and aligns with DVB DASH; fetching the first 1000 bytes covers the movie header for all (2017) DASH-IF test vectors.
- With several video Adaptation Sets (3.2.13), exactly one is signalled main (Role `urn:mpeg:dash:role:2011` value `main`), unless the sets carry the same content at different quality/codec, in which case all of them are main.
- Presence rules of 3.2.4 apply. A conforming client can process content under these rules (client procedures: 3.3).

> **Notes:** (a) p.34 has an empty bullet between the `trex` and `moof` rules (printed artifact). (b) "the the" appears in the SegmentTimeline rule and Note 3; the hyphen of `data-offset` is a judgement (line-end split). (c) The Role literal is printed with a stray space after the opening quote on p.35.

## 3.2.2 MPD constraints for v1/v2 clients (pp.35-36)

- 3.2.2.1: MPD conforms to the ISO BMFF Common profile (23009-1 Amd.1 clause 8.9) except that Representations with `@mimeType` `application/xml+ttml` shall not be ignored. `Representation`s with `@subsegmentStartsWithSAP` or `@startsWithSAP` equal to 3 may be ignored. If a Period has several video Adaptation Sets, at least one has Role `main` and all Adaptation Sets carrying a `main` Role provide perceptually equivalent streams. "Ignored" means a client may drop the element and the MPD still conforms.
- 3.2.2.2 simple restricted offering. Live profile: `MPD@type` static or dynamic; `@profiles` contains `urn:mpeg:dash:profile:isoff-live:2011`; Periods/Adaptation Sets/Representations; `SegmentTemplate` with `$Number$` or `$Time$`; `@segmentAlignment` true. On-demand profile: `@type` static; `@profiles` contains `urn:mpeg:dash:profile:isoff-ondemand:2011`; `@subSegmentAlignment` true.

> **Note (disagreement):** 3.2.2.2 spells the on-demand URN `isoff-ondemand`, 3.2.3 (p.36) spells it `isoff-on-demand`; ISO/IEC 23009-1:2022 uses `isoff-on-demand` (and the string `isoff-ondemand` does not occur there). Not reconciled.

## 3.2.3 Segment format constraints (pp.36-37)

- Representations follow 23009-1 clause 7.3. In Media Segments any `sidx`/`ssix` precede every `moof`. DVB DASH permits only one `sidx` per Segment; following that is strongly recommended.
- Static MPD with the on-demand profile: each Representation has one Segment that is an Indexed Self-Initializing Media Segment (23009-1 6.3.5.2).
- Presentation/decode times increase linearly with segment number in a Representation; to limit wrap, 64-bit codes or small timescales may be used, and a new Period may mark a wrap discontinuity.

## 3.2.4 Presence of attributes and elements (pp.37-38)

Inventory (shall be present unless noted):
| Scope | Item | Note |
|---|---|---|
| video AdaptationSet | `@maxWidth`/`@maxHeight`/`@maxFrameRate` (or `@width`/`@height`/`@frameRate` if equal for all), `@par` | max values describe target display size and may exceed the largest coded size; `@minWidth`/`@minHeight` should be absent (if present, may be smaller than the smallest) |
| video Representation | `@width`, `@height`, `@frameRate` (if not on the set), `@sar` | width/height/sar describe coded and cropped sample counts, not display size |
| video | `@scanType` | absent or `progressive` |
| audio AdaptationSet | `@lang` | |
| audio Representation | `@audioSamplingRate`, `AudioChannelConfiguration` (if not on the set) | |

## 3.2.5-3.2.6 (p.38)

No size limit on the MPD, but unnecessarily large MPDs should be avoided; DVB DASH adds limits (its section 4.5), and following them is recommended. Generic metadata uses `EssentialProperty`/`SupplementalProperty` (23009-1 5.8.4.7-8); identifiers are listed at `http://dashif.org/identifiers`, and clients are not expected to support them all unless required.

## 3.2.7 Timing model (pp.38-40)

Two timelines: the media presentation timeline (inside Segments) and the wall-clock availability timeline (when a Segment may be fetched). Static: all Segments available at AST. Terms and formulas:
- AST = `MPD@availabilityStartTime`; PS = `Period@start`; PST (wall clock) of the first Period = AST + PS(first); PTO = `Representation@presentationTimeOffset` (default 0).
- Each Segment has an MPD start time and duration (approximate, from the MPD) and an internal EPT (earliest presentation time) and DUR. Requirement: MPD start time lies within `EPT - PTO +- 0.5*DUR`.
- Live PTO is usually non-zero because encoders start before availability (encoder clock may be UTC-based); consecutive live Periods of continuous media usually share the same PTO. Periods are processed independently; a timestamp wrap is expected to be handled by a new Period restarting at 0. An ad Period's `Period@start` can be set to the UTC difference between AST and the encode time of the insertion point.
- Dynamic: latest availability = PST + MPD start time + MPD duration (the segment must be complete first).
- 3.2.7.4: `xs:duration` fields shall not use year or month units; all units are fixed size (60 s = 1 min, 60 min = 1 h, 24 h = 1 d, 30 d = 1 month, 12 months = 1 y); seconds are preferred.
- Figure 1 (p.39): wall-clock axis from AST with `Period@start` to PST1, Periods 1-3 with PST1-3, a Representation mapped from PTO to the Period start, and segments MS1..MSN with MPD start times, MS2 split into EPT, DUR and media (description only).

## 3.2.8 Bandwidth and MBT (pp.40-41)

- `MPD@minBufferTime` (MBT) and `Representation@bandwidth` (BW) are properties of the content encoding, not network instructions: over a constant-rate channel at BW, each presentation time PT is available at the client no later than PT + MBT at every SAP (here, every Segment start). Unless guided otherwise MBT should be the maximum GOP size, often the maximum (sub)segment duration; it may be smaller, should not be larger.
- Simple client selection: pick Representation i with `BW[i] <= rate * buffer / MBT`; a client working on whole Segments should use the maximum segment duration instead of MBT when that is larger. Real networks violate the model's idealisations, so values must be used with margin.

## 3.2.10 Adaptation Set constraints (pp.42-43)

- General: a client processes an Initialization Segment before the first Media Segment and again on each bitrate switch to another Representation. Segments in one set suit one decoder, start at SAP 1 or 2, and Representations are time aligned with the same `@timescale`. Edit lists in init segments should be identical across Representations. Reinitialising decoder/decryptor/display on a switch can cause artifacts; clients may diff init segments to minimise reconfiguration. See also 6.2.5 and 7.7.5.
- `AdaptationSet@bitstreamSwitching`: true means decoding continues without reinitialisation (init segment from the highest-bandwidth Representation, then any Media Segment); false means seamless switching needs reinitialisation. Authors should set true only if no re-initialisation is needed. With true: same `Track_ID` for all Representations and one track fragment per movie fragment. A Representation can appear in several Adaptation Sets (e.g. 720p in both 720p and 1080p sets) with the same init segment.

## 3.2.11 Media time of a Segment (pp.44-46)

- Estimate EPT from the MPD as segment availability start minus segment duration; get the exact value from a `sidx` (`earliest_presentation_time`, then subtract PTO), or from the fragment header and init segment edit list when no `sidx`.
- No edit list: SAP type 1 -> EPT = base media decode time + decode time + composition offset of the first sample. SAP type 2 -> the first sample may not be first in presentation order; EPT = base decode time + the minimum over samples of (decode time + composition offset).
- `@presentationTimeOffset` shall be used to map presentation time to the Period start; content must work with timing and lip sync if edit lists are ignored.
- Three worked examples (closed GOP, 14 samples/two segments with SAP 1, then 12 samples/two segments with SAP 2, then SAP 2 with negative composition offsets; all deltas 10): SAP 1 gives EPT 10 and 80 (base times 0, 70); SAP 2 gives EPT 10 and 70 (base 0, 60); negative offsets give EPT 0 and 60. CT = DT + composition offset in every column. (Cell values are clipped in the PDF; full values in the private verbatim.)

## 3.2.12 Periods (pp.46-47)

- Single-Period content: start at zero is suitable (aligned with Amd.3). Multiple Periods serve splicing/ads, segment numbering resync, adding/removing Representations, Adaptation Sets or CDNs, short segments, robustness (4.8).
- Association: Adaptation Sets in different Periods are associated via equivalent Asset Identifiers on the Periods and identical `AdaptationSet@id`; a client may continue an associated set.
- Period-continuous (all of): associated; PTO + presentation duration of the Representations equals the next Period's PTO; equal-`@id` Representations have functionally equivalent init segments whose concatenation with consecutive media segments over both Periods forms a conforming Segment sequence and the same `@mimeType`.
- Signalling (should): SupplementalProperty on the Adaptation Set with `urn:mpeg:dash:period-continuity:2015` (value = Period id per this clause) and the same `AdaptationSet@id`; MPDs with identical Asset Identifiers should signal it, in the later Period referring to any earlier one. Period-connected: as continuous but timestamps may jump, compensated by PTO; scheme `urn:mpeg:dash:period-connectivity:2015`. Continuity implies connectivity. Clients should exploit the signals for seamless play.

> **Notes:** the clause cites "4.5.4" (no such clause; 4.5 has 4.5.1-4.5.3); "preceeding" is a misspelling. 3.9.4.8 says the descriptor value is an Adaptation Set id (disagreement, p.68).

## 3.2.13-3.2.14 (pp.47-48)

One Adaptation Set = one media type. Identification: video `@mimeType="video/mp4"`; audio `audio/mp4`; subtitle `application/mp4` with Role `subtitle` and `@codecs` per Table 23 (or `application/ttml+xml`); metadata `application/mp4` with a sample-entry `@codecs`. `@codecs` limited to those defined in the document. Typically a client picks at most one set per media type and uses `@codecs` to test platform playability. Thumbnail navigation: separate trick-mode video set (needs a second decoder) or tiled images (6.2.6). Printed typos: "Adaptaion", "Table 23or".

## 3.2.15 MPD location and base URLs (pp.48-49)

Refs: 23009-1 5.3.1.2 (`MPD.Location`), 5.4.1, 5.6.4-5. `MPD.Location` may redirect to another update server; BaseURLs may spread content over CDNs. Client algorithm for an MPD update request, in priority order: use any `MPD.Location` value; else, if the HTTP request was redirected with 3xx, use the redirect URL; else the original manifest URL if known; else updates are impossible and the client terminates the service. The resulting manifest URL gives the implicit base; `BaseURL` elements do not apply to MPD updates. For segment requests: use an absolute BaseURL/URL if present; else, if the base is known, resolve the relative one against the MPD URL; else the service may be terminated.

> **Notes:** the second bullet is introduced as "requests the MPD" though it concerns segment URLs (p.49); printed typos "anc replicate", "any the value", "the is service".
