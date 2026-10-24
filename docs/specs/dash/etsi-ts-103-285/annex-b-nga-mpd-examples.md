---
title: "Annex B (informative) NGA MPD examples (notes)"
source: ETSI TS 103 285 V1.4.1 (2023-09), private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
clauses: Annex B (B.1-B.3)
pdf_pages: 133-141
printed_pages: 133-141
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/annex-b-nga-mpd-examples.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# ETSI TS 103 285 V1.4.1 - Annex B (informative) NGA MPD examples (notes)

Structured notes in the project's own wording (the standard's text is not reproduced; short quoted phrases only). Strength words are kept: **shall** = mandatory, **should** = recommended, **may** = optional. Verbatim, page-verified text: the private file named in the front matter. Back to [INDEX](INDEX.md).

The annex is nine MPD `Period` skeletons only (no prose). The complete XML is in the private verbatim file; below is the structure a client has to parse. Normative Preselection rules: clause 6.7 ([ch06](ch06-audio.md)) and 10.18 ([ch10-8](ch10-8-hdr-hfr-nga-decryption.md)).

## B.1 SRSP - one Representation, one Preselection per set (pp.133-134)

Three examples (AC-4 `ac-4.02.01.00`, MPEG-H `mhm1.0x0B`, DTS-UHD `dtsy`), each a Period with two AdaptationSets: English (Role `main`, `@lang="en"`, `audio_01/`) and German dub (Role `dub`, `@lang="de"`, `audio_02/`), 48 kHz, `SegmentTemplate` with `$Time$` and a `SegmentTimeline` `S t="0" d="96000" r="36"`, one 64000 bps Representation each, `AudioChannelConfiguration` (`urn:mpeg:mpegB:cicp:ChannelConfiguration` value 2, or `tag:dts.com,2018:uhd:audio_channel_configuration` value 6 for DTS). No Preselection elements.

## B.2 SRMP - one Representation, several Preselections (pp.134-137)

- One AdaptationSet carries everything (256000 bps) with a `SupplementalProperty` `urn:mpeg:dash:preselection:2016`, Role `main`, plus a Dolby `tag:dolby.com,2017:dash:audio_frame_rate:2017` (value 25) in the AC-4 case.
- Five `Preselection` children of the Period (ids 10, 20, 30, 40, 50; tags 101-105 or P1-P5) with `@preselectionComponents="1"`, codecs (`ac-4.02.01.03`/`.00`, `mhm1.0x0C`, `dtsy`), `@audioSamplingRate="48000"`, `@lang`, Role and channel configuration: primary 5.1.2 (Role main), audio description (Role commentary + Accessibility `urn:tva:metadata:cs:AudioPurposeCS:2007` value 1), clean audio (Role alternate, Accessibility value 2), French dub stereo (Role dub, `@lang="fr"`), English stereo main.
- Channel configuration values used: Dolby `tag:dolby.com,2015:dash:audio_channel_configuration:2015` value `0000C7`; CICP 14 (MPEG-H 5.1.2) and 2 (stereo); DTS `A03F` and 6.

## B.3 MRMP - several Representations, several Preselections (pp.137-141)

- Main AdaptationSet (id 1, Role main, SupplementalProperty preselection:2016, 256000 bps, English 5.1.2) plus three auxiliary sets (ids 2-4, 64000 bps, `EssentialProperty` preselection:2016 so they are not playable alone; no `@lang`).
- Preselections reference component sets by `@id`: primary `"1"`; audio description `"1 2"`; clean audio `"1 3"`; French dub `"1 4"`; stereo `"1"`.
- Variants: AC-4 (`ac-4.02.01.03` / `.00`, codec values per Preselection), MPEG-H (`mhm2.0x0C`), DTS-UHD (`dtsy`).

## Notes on the source (not corrected)

- Several examples are not well-formed XML as printed: the DTS-UHD B.1 German set has `< AudioChannelConfiguration` with a stray space and no closing, and its first `SegmentTemplate` is closed twice (p.134); comment terminators are broken across lines in B.3; a trailing space sits inside a DTS `schemeIdUri` value (p.141).
- Init-segment paths wrap as `audio-` / `02/init.mp4` in the PDF (MPEG-H and DTS B.1 German sets, pp.133-134) while the AC-4 example uses `audio_02`; Representation ids are repeated (`r0`) across DTS sets.
- B.3 DTS-UHD: Preselections 10 and 20 both use `tag="P1"`, then tags P2-P4 (p.140-141); "Auxilliary" spelling.
