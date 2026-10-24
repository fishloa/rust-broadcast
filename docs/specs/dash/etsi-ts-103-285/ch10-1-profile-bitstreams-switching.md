---
title: "Clauses 10.0-10.4 Player profile support, bitstreams, switching (notes)"
source: ETSI TS 103 285 V1.4.1 (2023-09), private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
clauses: 10.0-10.4
pdf_pages: 72-76
printed_pages: 72-76
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/ch10-1-profile-bitstreams-switching.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# ETSI TS 103 285 V1.4.1 - Clauses 10.0-10.4 Player profile support, bitstreams, switching (notes)

Structured notes in the project's own wording (the standard's text is not reproduced; short quoted phrases only). Strength words are kept: **shall** = mandatory, **should** = recommended, **may** = optional. Verbatim, page-verified text: the private file named in the front matter. Back to [INDEX](INDEX.md).

## 10.1 Profile support (p.72)

- Players shall support the 2014 profile URN `urn:dvb:dash:profile:dvb-dash:2014`; may support `urn:dvb:dash:profile:dvb-dash:2017`, and shall support 2017 if they play HDR, HFR or NGA.
- 2017-only clauses: HDR 5.2.4-5.2.7, 10.14, 10.15, 11.14; HFR 5.2.8, 10.16, 11.15; NGA 6.3.2, 6.7, 6.8, 10.18, 11.17. Everything else applies to both profiles. Clauses 9.1.7 and 9.1.8 apply to both but are optional for content and players.
- A codec/technology clause is moot for a player that does not support that codec through DASH.
- Players shall play whatever the profile-specific MPD (23009-1 clause 8.1) describes.

## 10.2 ISO BMFF player constraints (p.73)

Players shall: handle `trun` with negative composition offsets; handle several sample entries in `stsd` selected by the sample description index in the track fragment header; present several AdaptationSets whose Representations share a `track_ID`; handle segments with more than one `moof`/`mdat` pair (1 to whole-segment samples each); handle combinations of AdaptationSets whose segment boundaries or `moof` counts differ.

## 10.3 Decoding capability (p.73-75)

- Conformance points to support (ETSI TS 101 154, not reproduced here): AVC and HEVC clause L.2, VVC 5.15, AVS3 5.16 (at least one each if the codec is supported).
- HD players shall decode/display the Table 25 progressive sizes (width x height): 1920x1080, 1600x900, 1280x720, 1024x576, 960x540, 852x480, 768x432, 720x404, 704x396, 640x360, 512x288, 480x270, 384x216, 320x180, 192x108 (as printed, odd 404/396 rows included).
- Table 26 interlaced sizes (required for AVC, not HEVC/VVC/AVS3): 1920x1080, 720x576, 704x576, 544x576, 352x288.
- UHDTV players add Table 27: 3840x2160, 3200x1800, 2560x1440. UHDTV2 players (HEVC/VVC/AVS3) add Table 27a: 7680x4320, 5120x2880.
- Players shall also support frame rates equal to the supported family rates divided by 2 and by 4.
- Variable decoded resolution inside one Representation is optional; players lacking it should fail gracefully (switch Representation or stop) when the decoded size differs from `@width`/`@height` (4.4 makes those mandatory unless on the set).

## 10.4 Switching between Representations (p.75-76)

- Switching shall not disturb other components (audio must not stutter during a video switch).
- Seamless video switching shall work across Representations differing only in: frame rate within a family (25/50/100; 30/1,001, 60/1,001, 120/1,001 (i.e. divided by 1.001); 30/60/120; 24; 24/1,001), bit rate, profile/level, resolution at the same picture aspect ratio. Nothing is required for other differences.
- Switch only seamlessly unless needed to avoid a stall; interlaced/progressive switching is not required unless seamless, but a non-seamless switch beats ending playback.
- "Seamless" video = audio alignment kept, no gap between last old frame and first new frame, no dropped/black frames, no pixel shift, no colour/luminance change from the player.
- Audio: seamless switching across bit-rate-only differences; no clicks/glitches, continuous, aligned.
- 23009-1 clause 4.5.1 seamless switching also applies.

## Notes on the source

- The sentence after Table 25 says Table 26 sizes are required "at all supported frame rates" for AVC; the repeat after Table 27 omits that phrase (p.73-74).
