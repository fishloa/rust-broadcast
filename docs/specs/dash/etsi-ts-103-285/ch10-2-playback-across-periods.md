---
title: "Clause 10.5 Playback across Periods (notes)"
source: ETSI TS 103 285 V1.4.1 (2023-09), private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
clauses: 10.5
pdf_pages: 76-79
printed_pages: 76-79
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/ch10-2-playback-across-periods.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# ETSI TS 103 285 V1.4.1 - Clause 10.5 Playback across Periods (notes)

Structured notes in the project's own wording (the standard's text is not reproduced; short quoted phrases only). Strength words are kept: **shall** = mandatory, **should** = recommended, **may** = optional. Verbatim, page-verified text: the private file named in the front matter. Back to [INDEX](INDEX.md).

Cross-reference: ISO/IEC 23009-1 period continuity (`urn:mpeg:dash:period-continuity:2015`) in [dash-mpd-rules](../../../../specs/rules/dash-mpd-rules.md); DASH-IF IOP multi-period guidance in [dashif-iop-4.3](../dashif-iop-4.3/).

## 10.5.1 Background (p.76)

Multi-Period MPDs serve ad splicing, segment-number resync, adding/removing Representations, AdaptationSets or CDNs, and short-segment signalling. Content may change codec, colour, language, protection at a boundary. Players are expected to play on; small overlaps or gaps arise when media is longer/shorter than the Period, or when earliest presentation time differs from `@presentationTimeOffset` (23009-1 7.2.1), and playout must be adjusted. Applies to static and dynamic MPDs, after remote Periods are resolved.

## 10.5.2 Content offering (p.77-79)

- General: Period duration should be as close as possible to Representation durations (recommended: the maximum of them). Content may continue after other Periods (e.g. after an ad).
- Association (10.5.2.2): two AdaptationSets in different Periods are associated when the Periods carry equivalent Asset Identifiers and the sets have the same `AdaptationSet@id`. Associated sets shall be identical in `@lang`, `@contentType`, `@par`, Role, Accessibility, Viewpoint, and for audio all attributes/elements of Table 4 (ch06). Protection may add requirements.
- Period continuity signalling (10.5.2.3): the first Period names AID (set id) and PID (its Period id); each later Period's set with `@id` = AID carries a SupplementalProperty `urn:dvb:dash:period_continuity:2014` or `urn:mpeg:dash:period-continuity:2015` with `@value` = PID. Then shall hold: all Representations of the first set share earliest presentation time EPT1, those of the later set share EPT2; the sets are associated per 10.5.2.2; each Representation's presentation duration (end minus earliest time) in the first Period is EPT2 - EPT1; Representations with the same `@id` across the Periods have functionally equivalent init segments and their concatenation (init + all media segments of both Periods) is a conforming Segment sequence (23009-1 4.5.4). Providers should signal continuity. The MPEG URN uses a looser association notion; do not rely on full 10.5.2.2 compliance when it is used.
- Non-continuous playback (10.5.2.4): players should bridge gaps up to a configurable maximum, or drop overlaps; they may pause across a gap or continue at once on overlap. Audio-only gap/overlap at a boundary: video playback shall not be interrupted (Figure 1a). Gap or overlap between all Representations below the tolerance: players shall keep playing, a gap may freeze briefly (Figure 1b). Aligned content preferred; misalignment preferably only on audio.

Figure 1a/1b (p.79), described: bars for video/audio of Period 1 and Period 2 around `Period2@start`, showing (1a) audio gap/overlap only, (1b) gap/overlap on all Representations.

## 10.5.3 Player behaviour (p.79)

- Player should play across Periods; a re-initialisation delay may occur and should be minimised.
- Static: delay must not skip content; reported media time stands still during the delay (the overall wall-clock run is longer).
- Dynamic: repeated delays make the player drift behind live; it may jump forward or play faster than real time to rejoin the edge.
- If the next Period has the same Asset Identifier, the player should find the associated AdaptationSet and keep playing it absent other information.
- If the set is period-continuous the player shall play seamlessly across the boundary within 10.4 limits. Prefer the same Representation `@id`; if unavailable switch to any other Representation of the set.

## Notes on the source

- 10.5.3 refers to "period-continuous (as defined in clause 10.5.2.2)"; the definition is in 10.5.2.3.
- Caption "Figure 1.b" vs text "Figure 1b"; heading typo "Non-continous".
