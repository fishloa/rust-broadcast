---
title: "Clauses 10.6-10.7 Trick modes and buffering (notes)"
source: ETSI TS 103 285 V1.4.1 (2023-09), private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
clauses: 10.6-10.7
pdf_pages: 80-83
printed_pages: 80-83
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/ch10-3-trickmode-buffering.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# ETSI TS 103 285 V1.4.1 - Clauses 10.6-10.7 Trick modes and buffering (notes)

Structured notes in the project's own wording (the standard's text is not reproduced; short quoted phrases only). Strength words are kept: **shall** = mandatory, **should** = recommended, **may** = optional. Verbatim, page-verified text: the private file named in the front matter. Back to [INDEX](INDEX.md).

Trick-mode MPD signalling (`http://dashif.org/guidelines/trickmode`, `@maxPlayoutRate`, `@codingDependency`) is in [dashif-iop-4.3-trickmode](../dashif-iop-4.3/dashif-iop-4.3-trickmode.md) and not repeated.

## 10.6 Trick mode (informative, p.80-83)

- Support for speeds other than 0 and 1 is optional, as is the choice of technique.
- Player-enabled techniques (10.6.1): pause/resume by stopping and restarting segment requests at the next segment; slow motion by repeating output frames; audio fast-forward possibly pitch-corrected, often muted in reverse/fast video; timing of non-sequential segments from `tfdt` plus `Period@presentationTimeOffset`, with composition offsets (negative offsets in v1 `trun` recommended) to avoid delaying video behind audio; edit lists/`sidx` are awkward for adaptive switching and bitstream switching; progress bar from presentation duration (static) or first-to-last available segment (dynamic); current position = composition time minus `@presentationTimeOffset`; resume normal play at the nearest access point.
- Fast-forward by lower-resolution decoding (10.6.1.1): `@maxPlayoutRate` on a Representation/SubRepresentation says it can be played at up to that multiple with the same decoder profile/level.
- Fast-forward by discarding unreferenced pictures (10.6.1.2) or by I-picture-only access (10.6.1.3, resume on IDR; ISO files do not distinguish I-picture types without sample groups); reverse via I pictures (10.6.1.4); random access inside a segment needs decoding from an I picture and is not seamless (10.6.1.5); partial-segment fetches by byte range using `sidx`/`ssix` (10.6.1.6).
- Content-enabled (10.6.2): trick-play Representations are IDR-only, low frame rate/size, long segments (example: 60 one-hertz images per minute-long segment gives 60x), kept in a separate AdaptationSet with `@maxPlayoutRate`, `@codingDependency="false"`, optionally an EssentialProperty and a Role "alternate". A Representation/SubRepresentation with `@codingDependency="false"` suits fast-forward and rewind. SubRepresentations with `@maxPlayoutRate` allow fast-forward with periodic frame dropping; in On Demand profile, SubRepresentation plus `sidx`/`ssix` give compact byte-range requests.

## 10.7 Buffering (p.83)

Player should not buffer more than about 300 s of normal play ahead of the play position.

## Notes on the source

- Typos: "Period@presentationTimeOffest", speeds written "0,0 and 1,0"; 10.6.2.2 has two near-duplicate sentences about On Demand SubRepresentations.
