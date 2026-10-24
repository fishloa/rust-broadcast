---
title: "Clauses 10.14-10.19 Colour, HDR, temporal layers, compatibility, NGA, decryption (notes)"
source: ETSI TS 103 285 V1.4.1 (2023-09), private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
clauses: 10.14-10.19
pdf_pages: 101-103
printed_pages: 101-103
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/ch10-8-hdr-hfr-nga-decryption.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# ETSI TS 103 285 V1.4.1 - Clauses 10.14-10.19 Colour, HDR, temporal layers, compatibility, NGA, decryption (notes)

Structured notes in the project's own wording (the standard's text is not reproduced; short quoted phrases only). Strength words are kept: **shall** = mandatory, **should** = recommended, **may** = optional. Verbatim, page-verified text: the private file named in the front matter. Back to [INDEX](INDEX.md).

Related: colour descriptors of 5.2.5 ([ch05](ch05-video.md)), Preselection signalling of 6.7 ([ch06](ch06-audio.md)), EssentialProperty/SupplementalProperty 23009-1 5.8.4.8 ([dash-mpd-rules](../../../../specs/rules/dash-mpd-rules.md)).

## 10.14 HEVC colour description (p.101) - applies to all HEVC players

- Players shall process the 5.2.5 EssentialProperty/SupplementalProperty descriptors (the text states this once for 2017-profile players and once generally) and shall ignore any video AdaptationSet whose EssentialProperty names colour primaries, matrix coefficients or transfer characteristics the player does not support. Examples: no BT.2020 support -> ignore sets with `urn:mpeg:mpegB:cicp:ColourPrimaries` value 9; no BT.2100 PQ support -> ignore sets with `urn:mpeg:mpegB:cicp:TransferCharacteristics` value 16.
- A player counts as supporting BT.2020 if any holds: integrated display shows a different picture for BT.2020 than for BT.709 signalling; BT.2020 is sent over HDMI in BT.2020 mode to a sink declaring support; or the picture is converted before HDMI to another colorimetry. Players without a display consider the connected display's capabilities.
- VUI/SEI colour data in the bitstream overrides MPD signalling; MPD signalling only helps choose sets.

## 10.15 Display composition (p.101)

Players shall account for the video's colour/HDR format when compositing subtitles and graphics. Subtitles in DVB DASH are standard dynamic range; wrong mapping on HDR could give extremely bright text.

## 10.16 HEVC temporal layers (p.101)

HEVC players shall decode Representations with more than one temporal layer, including ones whose SPS signals profile/level above the Representation's own, provided the SPS values are correct for the layers included.

## 10.17 Compatibility (p.102)

- Ignore any AdaptationSet/Representation with an EssentialProperty whose scheme or value is not understood (except where several EssentialProperty share an `@id` and at least one is understood), or whose `@codecs`, `@frameRate`, `@width`, `@height`, `@par` or `@sar` cannot be handled.
- 2017-profile MPDs may carry non-backward-compatible features; correct EssentialProperty and `@codecs` handling is how a player finds the compatible part.
- Players shall support the CMAF `cmfc` brand constraints (ISO/IEC 23000-19 clause 7; CMAF Track 7.3.2.2, Track File 7.3.3.3).
- Players shall ignore unknown boxes in init, media and self-initialising segments.
- Players without low-latency support shall ignore `SegmentTemplate@availabilityTimeOffset` when `@availabilityTimeComplete="false"`.

## 10.18 NGA player requirements (p.102)

- Shall support SRSP and SRMP; may support MRMP (not natively possible with W3C Media Source Extensions at the time).
- Shall support Preselection elements and descriptors and select one Preselection from the available set for SRMP/MRMP content (not useful for SRSP).
- Shall not fetch an AdaptationSet carrying an unknown EssentialProperty.
- For sets with a Preselection descriptor (Essential or Supplemental), use only the Preselection parameters to choose, not the set's own elements/attributes.
- `@selectionPriority` has the lowest precedence between Preselections (after Role, `@lang` etc.).
- If MRMP is supported, download from at least 3 audio Representations in different AdaptationSets of one bundle.

## 10.19 Decryption (p.102-103)

Players that decrypt shall support `cenc` and may support `cbcs`; players not supporting `cbcs` with a supported DRM shall ignore AdaptationSets signalled with it. The DRM is identified by the 8.4 ContentProtection descriptors.

## Notes on the source

- Typos: "carried those messages", "all part the media presentation", "temporal layers include in", "precendence".
