# DVB A001r18 (draft TS 101 154 v2.7.1) — Use of Video and Audio Coding in Broadcast and Broadband Applications

> **✓ Accuracy-verified against the PDF — 2026-06-13.** Verified via direct PDF-page reads (Read tool) of every table page. All header rows rebuilt from the rendered PDF; split-cell artifacts removed; enum/width/mnemonic/reserved-range confirmed row-by-row; cosmetic trailing empty cells trimmed. One table added that the geometry extractor missed entirely (Table 18b). The large Annex A / Annex B / Annex C / Annex D blocks that the extractor incorrectly appended inside Table 29's section have been removed — they are informative annexes not part of Table 29.

Reference transcribed from the canonical PDF (`specs/dvb_a001r18_draft_ts_101_154_v02.07.01_av_coding.pdf`) by the
geometry-based extractor in `tools/dvb-si-audit/` — field rows aligned to
their bit-widths by page geometry, reproduced verbatim. The PDF in `specs/`
is the authoritative source.

## Contents

- [Table 3 — Values for display_horizontal_size](#table-3-values-for-display_horizontal_size)
- [Table 4 — Resolutions for Full-screen Display from 25 Hz MPEG-2 SDTV IRD](#table-4-resolutions-for-full-screen-display-from-25-hz-mpeg-2-sdtv-ird)
- [Table 5 — Values for display_horizontal_size](#table-5-values-for-display_horizontal_size)
- [Table 6 — Resolutions for Full-screen Display from 30 Hz MPEG-2 SDTV IRD](#table-6-resolutions-for-full-screen-display-from-30-hz-mpeg-2-sdtv-ird)
- [Table 7 — time_scale and num_units_in_tick for Progressive and Interlace Frame Rates for 25 Hz H.264/AVC SDTV](#table-7-time_scale-and-num_units_in_tick-for-progressive-and-interlace-frame-rates-for-25-hz-h264avc-sdtv)
- [Table 8 — Resolutions for Full-screen Display from 25 Hz H.264/AVC SDTV IRD](#table-8-resolutions-for-full-screen-display-from-25-hz-h264avc-sdtv-ird)
- [Table 9 — Time_scal and num_units_in_tick for Progressive and Interlace Frame Rates for 30 Hz H.264/AVC SDTV](#table-9-time_scal-and-num_units_in_tick-for-progressive-and-interlace-frame-rates-for-30-hz-h264avc-sdtv)
- [Table 10 — Resolutions for Full-screen Display from 30 Hz H.264/AVC SDTV IRD](#table-10-resolutions-for-full-screen-display-from-30-hz-h264avc-sdtv-ird)
- [Table 11 — Resolutions for Full-screen Display from H.264/AVC HDTV IRD and SVC HDTV IRD](#table-11-resolutions-for-full-screen-display-from-h264avc-hdtv-ird-and-svc-hdtv-ird)
- [Table 12 — Time_scal and num_units_in_tick for Progressive and Interlace Frame Rates for 25 Hz H.264/AVC HDTV](#table-12-time_scal-and-num_units_in_tick-for-progressive-and-interlace-frame-rates-for-25-hz-h264avc-hdtv)
- [Table 13 — Time_scal and num_units_in_tick for Progressive and Interlace Frame Rates for 30 Hz H.264/AVC HDTV](#table-13-time_scal-and-num_units_in_tick-for-progressive-and-interlace-frame-rates-for-30-hz-h264avc-hdtv)
- [Table 14 — Resolutions for Full-screen Display from 25 Hz VC-1 SDTV IRD](#table-14-resolutions-for-full-screen-display-from-25-hz-vc-1-sdtv-ird)
- [Table 15 — Resolutions for Full-screen Display from 25 Hz VC-1 HDTV IRD](#table-15-resolutions-for-full-screen-display-from-25-hz-vc-1-hdtv-ird)
- [Table 16 — Resolutions for Full-screen Display from 30 Hz VC-1 SDTV IRD](#table-16-resolutions-for-full-screen-display-from-30-hz-vc-1-sdtv-ird)
- [Table 17 — Resolutions for Full-screen Display from 30 Hz VC-1 HDTV IRD](#table-17-resolutions-for-full-screen-display-from-30-hz-vc-1-hdtv-ird)
- [Table 18 — Resolutions for Full-screen Display from MVC Stereo HDTV IRD](#table-18-resolutions-for-full-screen-display-from-mvc-stereo-hdtv-ird)
- [Table 18a — HEVC IRD conformance points specified in the present document](#table-18a-hevc-ird-conformance-points-specified-in-the-present-document)
- [Table 18b — HEVC UHDTV Bitstream conformance points and capable IRDs](#table-18b-hevc-uhdtv-bitstream-conformance-points-and-capable-irds)
- [Table 19 — Progressive and Interlaced Frame Rates for HEVC Bitstreams](#table-19-progressive-and-interlaced-frame-rates-for-hevc-bitstreams)
- [Table 20 — Resolutions for Full-screen Display from HEVC HDTV IRD](#table-20-resolutions-for-full-screen-display-from-hevc-hdtv-ird)
- [Table 21 — Resolutions for Full-screen Display from HEVC UHDTV IRD](#table-21-resolutions-for-full-screen-display-from-hevc-uhdtv-ird)
- [Table 21a — Resolutions for Full-screen Display from HEVC HDR UHDTV IRD](#table-21a-resolutions-for-full-screen-display-from-hevc-hdr-uhdtv-ird)
- [Table 21b — Progressive Frame Rates for HEVC HFR UHDTV Bitstreams](#table-21b-progressive-frame-rates-for-hevc-hfr-uhdtv-bitstreams)
- [Table 21c — Resolutions for Full-screen Display from HEVC HDR UHDTV2 IRD](#table-21c-resolutions-for-full-screen-display-from-hevc-hdr-uhdtv2-ird)
- [Table 22 — drc_decoder_mode_id supported by AC-4](#table-22-drc_decoder_mode_id-supported-by-ac-4)
- [Table 23 — (E-)AC-3 profiles supported by AC-4](#table-23-e-ac-3-profiles-supported-by-ac-4)
- [Table 28 — DTS-UHD BroadcastChunk](#table-28-dts-uhd-broadcastchunk)
- [Table 29 — DTS-UHD Syncwords](#table-29-dts-uhd-syncwords)

## Table 3 — Values for display_horizontal_size
_§5.1.4, PDF pp. 57-57_

| horizontal_size × vertical_size | Source aspect ratio | Display_horizontal_size |
|---|---|---|
| 720 × 576 | 16:9 | 540 |
| 544 × 576 | 16:9 | 408 |
| 480 × 576 | 16:9 | 360 |
| 352 × 576 | 16:9 | 264 |
| 352 × 288 | 16:9 | 264 |

## Table 4 — Resolutions for Full-screen Display from 25 Hz MPEG-2 SDTV IRD
_§5.1.4, PDF pp. 58-58_

| Luminance resolution (horizontal × vertical) | Aspect Ratio | 4:3 Monitors | 16:9 Monitors |
|---|---|---|---|
| 720 × 576 | 4:3 | × 1 | × 3/4 (see note 1) |
| | 16:9 | × 4/3 (see note 2) | × 1 |
| | 2.21:1 | × 5/3 (see note 3) | × 5/4 (see note 4) |
| 544 × 576 | 4:3 | × 4/3 | × 1 (see note 1) |
| | 16:9 | × 16/9 (see note 2) | × 4/3 |
| | 2.21:1 | × 20/9 (see note 3) | × 5/3 (see note 4) |
| 480 × 576 | 4:3 | × 3/2 | × 9/8 (see note 1) |
| | 16:9 | × 2 (see note 2) | × 3/2 |
| | 2.21:1 | × 5/2 (see note 3) | × 15/8 (see note 4) |
| 352 × 576 | 4:3 | × 2 | × 3/2 (see note 1) |
| | 16:9 | × 8/3 (see note 2) | × 2 |
| | 2.21:1 | × 10/3 (see note 3) | × 5/2 (see note 4) |
| 352 × 288 | 4:3 | × 2 | × 3/2 (see note 1) |
| | 16:9 | × 8/3 (see note 2) | × 2 |
| | 2.21:1 | × 10/3 (see note 3) (and vertical up sampling × 2) | × 5/2 (see note 4) (and vertical up sampling × 2) |

NOTE 1: Up sampling of 4:3 pictures for display on a 16:9 monitor is optional in the IRD, as 16:9 monitors can be switched to operate in 4:3 mode.
NOTE 2: The up sampling with this value is applied to the pixels of the 16:9 picture to be displayed on a 4:3 monitor.
NOTE 3: The up sampling with this value is applied to the pixels of the 2.21:1 picture to be displayed on a 4:3 monitor. Up sampling from 2.21:1 pictures for display on a 4:3 monitor is optional in the IRD.
NOTE 4: The up sampling with this value is applied to the pixels of the 2.21:1 picture to be displayed on a 16:9 monitor. Up sampling from 2.21:1 pictures for display on a 16:9 monitor is optional in the IRD.

## Table 5 — Values for display_horizontal_size
_§5.3.3, PDF pp. 63-63_

| horizontal_size × vertical_size | Source aspect ratio | Display_horizontal_size |
|---|---|---|
| 720 × 480 | 16:9 | 540 |
| 640 × 480 | 16:9 | 480 |
| 544 × 480 | 16:9 | 408 |
| 480 × 480 | 16:9 | 360 |
| 352 × 480 | 16:9 | 264 |
| 352 × 240 | 16:9 | 264 |

## Table 6 — Resolutions for Full-screen Display from 30 Hz MPEG-2 SDTV IRD
_§5.3.5, PDF pp. 65-65_

| Luminance resolution (horizontal × vertical) | Aspect Ratio | 4:3 Monitors | 16:9 Monitors |
|---|---|---|---|
| 720 × 480 | 4:3 | × 1 | × 3/4 (see note 1) |
| | 16:9 | × 4/3 (see note 2) | × 1 |
| | 2.21:1 | × 5/3 (see note 3) | × 5/4 (see note 4) |
| 640 × 480 | 4:3 | × 9/8 | × 27/32 (see note 1) |
| 544 × 480 | 4:3 | × 4/3 | × 1 (see note 1) |
| | 16:9 | × 16/9 (see note 2) | × 4/3 |
| | 2.21:1 | × 20/9 (see note 3) | × 5/3 (see note 4) |
| 480 × 480 | 4:3 | × 3/2 | × 9/8 (see note 1) |
| | 16:9 | × 2 (see note 2) | × 3/2 |
| | 2.21:1 | × 5/2 (see note 3) | × 15/8 (see note 4) |
| 352 × 480 | 4:3 | × 2 | × 3/2 (see note 1) |
| | 16:9 | × 8/3 (see note 2) | × 2 |
| | 2.21:1 | × 10/3 (see note 3) | × 5/2 (see note 4) |
| 352 × 240 | 4:3 | × 2 | × 3/2 (see note 1) |
| | 16:9 | × 8/3 (see note 2) | × 2 |
| | 2.21:1 | × 10/3 (see note 3) (and vertical up sampling × 2) | × 5/2 (see note 4) (and vertical up sampling × 2) |

NOTE 1: Up sampling of 4:3 pictures for display on a 16:9 monitor is optional in the IRD, as 16:9 monitors can be switched to operate in 4:3 mode.
NOTE 2: The up sampling with this value is applied to the pixels of the 16:9 picture to be displayed on a 4:3 monitor.
NOTE 3: The up sampling with this value is applied to the pixels of the 2.21:1 picture to be displayed on a 4:3 monitor. Up sampling from 2.21:1 pictures for display on a 4:3 monitor is optional in the IRD.
NOTE 4: The up sampling with this value is applied to the pixels of the 2.21:1 picture to be displayed on a 16:9 monitor. Up sampling from 2.21:1 pictures for display on a 16:9 monitor is optional in the IRD.

## Table 7 — time_scale and num_units_in_tick for Progressive and Interlace Frame Rates for 25 Hz H.264/AVC SDTV
_§5.6.2.3, PDF pp. 75-75_

| Frame Rate | Interlaced or Progressive | time_scale | Num_units_in_tick |
|---|---|---|---|
| 25 | P | 50 | 1 |
| 25 | I | 50 | 1 |

## Table 8 — Resolutions for Full-screen Display from 25 Hz H.264/AVC SDTV IRD
_§5.6.3.1, PDF pp. 76-76_

| Luminance resolution (horizontal × vertical) | Source Aspect Ratio | Aspect_ratio_idc | 4:3 Monitors | 16:9 Monitors |
|---|---|---|---|---|
| 720 × 576 | 4:3 | 2 | × 1 | × 3/4 (see note 1) |
| | 16:9 | 4 | × 4/3 (see note 2) | × 1 |
| 544 × 576 | 4:3 | 4 | × 4/3 | × 1 (see note 1) |
| | 16:9 | 12 | × 16/9 (see note 2) | × 4/3 |
| 480 × 576 | 4:3 | 10 | × 3/2 | × 9/8 (see note 1) |
| | 16:9 | 6 | × 2 (see note 2) | × 3/2 |
| 352 × 576 | 4:3 | 6 | × 2 | × 3/2 (see note 1) |
| | 16:9 | 8 | × 8/3 (see note 2) | × 2 |
| 352 × 288 | 4:3 | 2 | × 2 | × 3/2 (see note 1) |
| | 16:9 | 4 | × 8/3 (see note 2) (and vertical up sampling × 2) | × 2 (and vertical up sampling × 2) |

NOTE 1: Up sampling of 4:3 pictures for display on a 16:9 monitor is optional in the IRD, as 16:9 monitors can be switched to operate in 4:3 mode.
NOTE 2: The up sampling with this value is applied to the pixels of the 16:9 picture to be displayed on a 4:3 monitor.

## Table 9 — Time_scal and num_units_in_tick for Progressive and Interlace Frame Rates for 30 Hz H.264/AVC SDTV
_§5.6.3.3, PDF pp. 77-77_

| Frame Rate | Interlaced or Progressive | time_scale | Num_units_in_tick |
|---|---|---|---|
| 24 000/1 001 | P | 48 000 | 1 001 |
| 24 | P | 48 | 1 |
| 30 000/1 001 | P | 60 000 | 1 001 |
| 30 | P | 60 | 1 |
| 30 000/1 001 | I | 60 000 | 1 001 |
| 30 | I | 60 | 1 |

## Table 10 — Resolutions for Full-screen Display from 30 Hz H.264/AVC SDTV IRD
_§5.7.1.1, PDF pp. 78-78_

| Luminance resolution (horizontal × vertical) | Source Aspect Ratio | aspect_ratio_idc | 4:3 Monitors | 16:9 Monitors |
|---|---|---|---|---|
| 720 × 480 | 4:3 | 3 | × 1 | × 3/4 (see note 1) |
| | 16:9 | 5 | × 4/3 (see note 2) | × 1 |
| 640 × 480 | 4:3 | 1 | × 9/8 | × 27/32 (see note 1) |
| | 16:9 | 14 | × 3/2 | × 9/8 |
| 544 × 480 | 4:3 | 5 | × 4/3 | × 1 (see note 1) |
| | 16:9 | 13 | × 16/9 (see note 2) | × 4/3 |
| 480 × 480 | 4:3 | 11 | × 3/2 | × 9/8 (see note 1) |
| | 16:9 | 7 | × 2 (see note 2) | × 3/2 |
| 352 × 480 | 4:3 | 7 | × 2 | × 3/2 (see note 1) |
| | 16:9 | 9 | × 8/3 (see note 2) | × 2 |
| 352 × 240 | 4:3 | 3 | × 2 | × 3/2 (see note 1) |
| | 16:9 | 5 | × 8/3 (see note 2) (and vertical up sampling × 2) | × 2 (and vertical up sampling × 2) |

NOTE 1: Up sampling of 4:3 pictures for display on a 16:9 monitor is optional in the IRD, as 16:9 monitors can be switched to operate in 4:3 mode.
NOTE 2: The up sampling with this value is applied to the pixels of the 16:9 picture to be displayed on a 4:3 monitor.

## Table 11 — Resolutions for Full-screen Display from H.264/AVC HDTV IRD and SVC HDTV IRD
_§5.7.2.2, PDF pp. 80-80_

| Luminance resolution (horizontal × vertical) | Source Aspect Ratio | aspect_ratio_idc | 16:9 Monitors Horizontal up sampling |
|---|---|---|---|
| 1 920 × 1 080 | 16:9 | 1 | × 1 |
| 1 440 × 1 080 | 16:9 | 14 | × 4/3 |
| 1 280 × 1 080 | 16:9 | 15 | × 3/2 |
| 960 × 1 080 | 16:9 | 16 | × 2 |
| 1 280 × 720 | 16:9 | 1 | × 1 |
| 960 × 720 | 16:9 | 14 | × 4/3 |
| 640 × 720 | 16:9 | 16 | × 2 |

## Table 12 — Time_scal and num_units_in_tick for Progressive and Interlace Frame Rates for 25 Hz H.264/AVC HDTV
_§5.7.2.2, PDF pp. 80-80_

| Frame Rate | Interlaced or Progressive | time_scale | num_units_in_tick |
|---|---|---|---|
| 25 | P | 50 | 1 |
| 25 | I | 50 | 1 |
| 50 | P | 100 | 1 |

## Table 13 — Time_scal and num_units_in_tick for Progressive and Interlace Frame Rates for 30 Hz H.264/AVC HDTV
_§5.7.3.3, PDF pp. 81-81_

| Frame Rate | Interlaced or Progressive | time_scale | Num_units_in_tick |
|---|---|---|---|
| 24 000/1 001 | P | 48 000 | 1 001 |
| 24 | P | 48 | 1 |
| 30 000/1 001 | P | 60 000 | 1 001 |
| 30 | P | 60 | 1 |
| 30 000/1 001 | I | 60 000 | 1 001 |
| 30 | I | 60 | 1 |
| 60 000/1 001 | P | 120 000 | 1 001 |
| 60 | P | 120 | 1 |

## Table 14 — Resolutions for Full-screen Display from 25 Hz VC-1 SDTV IRD
_§5.9.5, PDF pp. 101-101_

| Luminance resolution (horizontal × vertical) | Source Video Aspect Ratio | 4:3 Monitors | 16:9 Monitors |
|---|---|---|---|
| 720 × 576 | 4:3 | × 1 | × 3/4 (see note 1) |
| | 16:9 | × 4/3 (see note 2) | × 1 |
| 544 × 576 | 4:3 | × 4/3 | × 1 (see note 1) |
| | 16:9 | × 16/9 (see note 2) | × 4/3 |
| 480 × 576 | 4:3 | × 3/2 | × 9/8 (see note 1) |
| | 16:9 | × 2 (see note 2) | × 3/2 |
| 352 × 576 | 4:3 | × 2 | × 3/2 (see note 1) |
| | 16:9 | × 8/3 (see note 2) | × 2 |
| 352 × 288 | 4:3 | × 2 | × 3/2 (see note 1) |
| | 16:9 | × 8/3 (see note 2) (and vertical up sampling × 2) | × 2 (and vertical up sampling × 2) |

NOTE 1: Up sampling of 4:3 pictures for display on a 16:9 monitor is optional in the IRD, as 16:9 monitors can be switched to operate in 4:3 mode.
NOTE 2: The up sampling with this value is applied to the pixels of the 16:9 picture to be displayed on a 4:3 monitor.

## Table 15 — Resolutions for Full-screen Display from 25 Hz VC-1 HDTV IRD
_§5.10.6, PDF pp. 103-103_

| Luminance resolution (horizontal × vertical) | Source Aspect Ratio | 16:9 Monitors Horizontal up sampling |
|---|---|---|
| 1 920 × 1 080 | 16:9 | × 1 |
| 1 440 × 1 080 | 16:9 | × 4/3 |
| 1 280 × 1 080 | 16:9 | × 3/2 |
| 960 × 1 080 | 16:9 | × 2 |
| 1 280 × 720 | 16:9 | × 1 |
| 960 × 720 | 16:9 | × 4/3 |
| 640 × 720 | 16:9 | × 2 |

## Table 16 — Resolutions for Full-screen Display from 30 Hz VC-1 SDTV IRD
_§5.11.5, PDF pp. 105-105_

| Luminance resolution (horizontal × vertical) | Source Video Aspect Ratio | 4:3 Monitors | 16:9 Monitors |
|---|---|---|---|
| 720 × 480 | 4:3 | × 1 | × 3/4 (see note 1) |
| | 16:9 | × 4/3 (see note 2) | × 1 |
| 640 × 480 | 4:3 | × 9/8 | × 27/32 (see note 1) |
| | 16:9 | × 3/2 | × 9/8 |
| 544 × 480 | 4:3 | × 4/3 | × 1 (see note 1) |
| | 16:9 | × 16/9 (see note 2) | × 4/3 |
| 480 × 480 | 4:3 | × 3/2 | × 9/8 (see note 1) |
| | 16:9 | × 2 (see note 2) | × 3/2 |
| 352 × 480 | 4:3 | × 2 | × 3/2 (see note 1) |
| | 16:9 | × 8/3 (see note 2) | × 2 |
| 352 × 240 | 4:3 | × 2 | × 3/2 (see note 1) |
| | 16:9 | × 8/3 (see note 2) (and vertical up sampling × 2) | × 2 (and vertical up sampling × 2) |

NOTE 1: Up sampling of 4:3 pictures for display on a 16:9 monitor is optional in the IRD, as 16:9 monitors can be switched to operate in 4:3 mode.
NOTE 2: The up sampling with this value is applied to the pixels of the 16:9 picture to be displayed on a 4:3 monitor.

## Table 17 — Resolutions for Full-screen Display from 30 Hz VC-1 HDTV IRD
_§5.12.6, PDF pp. 107-107_

| Luminance resolution (horizontal × vertical) | Source Aspect Ratio | 16:9 Monitors Horizontal up sampling |
|---|---|---|
| 1 920 × 1 080 | 16:9 | × 1 |
| 1 440 × 1 080 | 16:9 | × 4/3 |
| 1 280 × 1 080 | 16:9 | × 3/2 |
| 960 × 1 080 | 16:9 | × 2 |
| 1 280 × 720 | 16:9 | × 1 |
| 960 × 720 | 16:9 | × 4/3 |
| 640 × 720 | 16:9 | × 2 |

## Table 18 — Resolutions for Full-screen Display from MVC Stereo HDTV IRD
_§5.13.1.7, PDF pp. 111-111_

| Luminance resolution (horizontal × vertical) | Source Aspect Ratio | aspect_ratio_idc | 16:9 Monitors Horizontal up sampling |
|---|---|---|---|
| 1 920 × 1 080 | 16:9 | 1 | × 1 |
| 1 440 × 1 080 | 16:9 | 14 | × 4/3 |
| 1 280 × 1 080 | 16:9 | 15 | × 3/2 |
| 960 × 1 080 | 16:9 | 16 | × 2 |
| 1 280 × 720 | 16:9 | 1 | × 1 |
| 960 × 720 | 16:9 | 14 | × 4/3 |
| 640 × 720 | 16:9 | 16 | × 2 |

## Table 18a — HEVC IRD conformance points specified in the present document
_§5.14.6, PDF pp. 119-119_

| HEVC IRD type | Relevant clauses |
|---|---|
| 50 Hz HEVC HDTV 8-bit IRD | 5.14.1 (with constraints set as documented for 50 Hz HEVC HDTV IRDs in 5.14.1.7) |
| | 5.14.2 (with constraints set as documented for HEVC HDTV 8-bit IRDs in 5.14.2.1) |
| 60 Hz HEVC HDTV 8-bit IRD | 5.14.1 (with constraints set as documented for 60 Hz HEVC HDTV IRDs in 5.14.1.7) |
| | 5.14.2 (with constraints set as documented for HEVC HDTV 8-bit IRDs in 5.14.2.1) |
| 50 Hz HEVC HDTV 10-bit IRD | 5.14.1 (with constraints set as documented for 50 Hz HEVC HDTV IRDs in 5.14.1.7) |
| | 5.14.2 (with constraints set as documented for HEVC HDTV 10-bit IRDs in 5.14.2.1) |
| 60 Hz HEVC HDTV 10-bit IRD | 5.14.1 (with constraints set as documented for 60 Hz HEVC HDTV IRDs in 5.14.1.7) |
| | 5.14.2 (with constraints set as documented for HEVC HDTV 10-bit IRDs in 5.14.2.1) |
| HEVC UHDTV IRD | 5.14.1 |
| | 5.14.3 |
| HEVC HDR UHDTV IRD using HLG10 | 5.14.1 |
| | 5.14.4 (with constraints set as documented for HLG10 in 5.14.4.4.2) |
| HEVC HDR UHDTV IRD using PQ10 | 5.14.1 |
| | 5.14.4 (with constraints set as documented for PQ10 in 5.14.4.4.3) |
| HEVC HDR HFR UHDTV IRD using HLG10 | 5.14.1 |
| | 5.14.5 (with constraint set as documented for HLG10) |
| HEVC HDR HFR UHDTV IRD using PQ10 | 5.14.1 |
| | 5.14.5 (with constraints set as documented for PQ10) |
| HEVC HDR UHDTV2 IRD | 5.14.1 |
| | 5.14.6 |

## Table 18b — HEVC UHDTV Bitstream conformance points and capable IRDs
_§5.14.6, PDF pp. 120-120_

Table 18b: HEVC UHDTV Bitstream conformance points specified in the present document and the IRDs capable to decode them (where "yes" means that the IRD can decode the Bitstream and "no" means that the IRD cannot decode the Bitstream)

| UHDTV Bitstream conformance points | HEVC UHDTV IRD | HEVC HDR UHDTV IRD using HLG10 | HEVC HDR UHDTV IRD using PQ10 | HEVC HDR HFR UHDTV IRD using HLG10 | HEVC HDR HFR UHDTV IRD using PQ10 | HEVC HDR UHDTV2 IRD |
|---|---|---|---|---|---|---|
| SDR — Frame Rate up to 60 Hz — Resolution up to 3840×2160 | yes | yes | yes | yes | yes | yes |
| HDR with PQ10 — Frame Rate up to 60 Hz — Resolution up to 3840×2160 | no | no | yes | no | yes | yes |
| HDR with HLG10 — Frame rate up to 60 Hz — Resolution up to 3840×2160 | yes, but as SDR | yes | yes, but as SDR | yes | yes, but as SDR | yes |
| SDR — HFR with single PID — Resolution up to 3840×2160 | no | no | no | yes | yes | no |
| HDR with PQ10 — HFR with single PID — Resolution up to 3840×2160 | no | no | no | no | yes | no |
| HDR with HLG10 — HFR with single PID — Resolution up to 3840×2160 | no | no | no | yes | yes, but as SDR | no |
| SDR — HFR with dual PID and temporal scalability — Resolution up to 3840×2160 | yes, but at half frame rate | yes, but at half frame rate | yes, but at half frame rate | yes | yes | yes, but at half frame rate |
| HDR with PQ10 — HFR with dual PID and temporal scalability — Resolution up to 3840×2160 | no | no | yes, but at half frame rate | no | yes | yes, but at half frame rate |
| HDR with HLG10 — HFR with dual PID and temporal scalability — Resolution up to 3840×2160 | yes, but as SDR and at half frame rate | yes, but at half frame rate | yes, but as SDR and at half frame rate | yes | yes, but as SDR | yes, but at half frame rate |
| SDR — Frame Rate up to 60 Hz — Resolution up to 7680×4320 | no | no | no | no | no | yes |
| HDR with PQ10 — Frame Rate up to 60 Hz — Resolution up to 7680×4320 | no | no | no | no | no | yes |
| HDR with HLG10 — Frame rate up to 60 Hz — Resolution up to 7680×4320 | no | no | no | no | no | yes |

## Table 19 — Progressive and Interlaced Frame Rates for HEVC Bitstreams
_§5.14.1.7, PDF pp. 127-127_

| Output Frame Rate | Interlaced or Progressive | elemental_duration_in_tc_minus1 [temporal_id_max (note 3)] | vui_time_scale | vui_num_units_in_tick | Allowed pic_struct |
|---|---|---|---|---|---|
| 24 000/1 001 | P | 0 | 24 000 | 1 001 | 0,7,8 |
| 24 | P | 0 | 24 | 1 | 0,7,8 |
| 25 | P | 0 | 25 | 1 | 0,7,8 |
| 25 | I (encoded as frames) | 0 | 50 | 1 | 3,4,5,6 |
| 25 | I (encoded as fields) | 0 | 50 | 1 | 9,10,11,12 |
| 30 000/1 001 | P | 0 | 30 000 | 1 001 | 0,7,8 |
| 30 000/1 001 | I (encoded as frames) | 0 | 60 000 | 1 001 | 3,4,5,6 |
| 30 000/1 001 | I (encoded as fields) | 0 | 60 000 | 1 001 | 9,10,11,12 |
| 30 | P | 0 | 30 | 1 | 0,7,8 |
| 50 | P | 0 | 50 | 1 | 0,7,8 |
| 60 000/1 001 | P | 0 | 60 000 | 1 001 | 0,7,8 |
| 60 | P | 0 | 60 | 1 | 0,7,8 |

## Table 20 — Resolutions for Full-screen Display from HEVC HDTV IRD
_§5.14.3.1, PDF pp. 132-132_

| Horizontal | Vertical | Scan (interlace/progressive) | Coded Frame | Aspect_ratio_idc | Horizontal up-sampling | Vertical up-sampling |
|---|---|---|---|---|---|---|
| 1 920 | 1 080 | I and P | 16:9 | 1 | × 1 | × 1 |
| 1 440 | 1 080 | I and P | 16:9 | 14 | × 4/3 | × 1 |
| 1 600 | 900 | P | 16:9 | 1 | × 6/5 | × 6/5 |
| 1 280 | 720 | P | 16:9 | 1 | × 3/2 | × 3/2 |
| 960 | 720 | P | 16:9 | 14 | × 2 | × 3/2 |
| 960 | 540 | P | 16:9 | 1 | × 2 | × 2 |

_Example up-sampling for 1 920 × 1 080 display._

## Table 21 — Resolutions for Full-screen Display from HEVC UHDTV IRD
_§5.14.3.4, PDF pp. 134-134_

| Horizontal | Vertical | Scan (interlace/progressive) | Coded Frame | Aspect_ratio_idc | Horizontal up-sampling | Vertical up-sampling |
|---|---|---|---|---|---|---|
| 3 840 | 2 160 | P | 16x9 | 1 | 1 | 1 |
| 2 880 | 2 160 | P | 16x9 | 14 | × 4/3 | × 1 |
| 3 200 | 1 800 | P | 16x9 | 1 | × 6/5 | × 6/5 |
| 2 560 | 1 440 | P | 16x9 | 1 | × 3/2 | × 3/2 |

_Example up-sampling for 3 840 × 2 160 display._

## Table 21a — Resolutions for Full-screen Display from HEVC HDR UHDTV IRD
_§5.14.4.3, PDF pp. 135-135_

| Horizontal | Vertical | Scan (interlace/progressive) | Coded Frame | Aspect_ratio_idc | Horizontal up-sampling | Vertical up-sampling |
|---|---|---|---|---|---|---|
| 3 840 | 2 160 | P | 16:9 | 1 | × 1 | × 1 |
| 3 200 | 1 800 | P | 16:9 | 1 | × 6/5 | × 6/5 |
| 2 560 | 1 440 | P | 16:9 | 1 | × 3/2 | × 3/2 |
| 1 920 | 1 080 | P | 16:9 | 1 | × 2 | × 2 |
| 1 600 | 900 | P | 16:9 | 1 | × 12/5 | × 12/5 |
| 1 280 | 720 | P | 16:9 | 1 | × 3 | × 3 |
| 960 | 540 | P | 16:9 | 1 | × 4 | × 4 |

_Example up-sampling for 3 840 × 2 160 display._

## Table 21b — Progressive Frame Rates for HEVC HFR UHDTV Bitstreams
_§5.14.5.5.1, PDF pp. 145-145_

| Output Frame Rate (fps) — HEVC UHDTV IRD | Output Frame Rate (fps) — HEVC HDR HFR UHDTV IRD | Stream Type: 0x24 (HEVC bitstream and HEVC temporal video sub-bitstream) elemental_duration_in_tc_minus1 [temporal_id_max](0x24) | Stream Type: 0x25 (HEVC temporal video subset) elemental_duration_in_tc_minus1 [temporal_id_max](0x25) | vui_time_scale | vui_num_units_in_tick | Allowed pic_struct |
|---|---|---|---|---|---|---|
| Not applicable | 100 | 0 | Not applicable | 100 | 1 | 0,7,8 |
| 50 | 100 | 1 | 0 | 100 | 1 | 0,7,8 |
| Not applicable | 120 000/1 001 | 0 | Not applicable | 120 000 | 1 001 | 0,7,8 |
| 60 000/1 001 | 120 000/1 001 | 1 | 0 | 120 000 | 1 001 | 0,7,8 |
| Not applicable | 120 | 0 | Not applicable | 120 | 1 | 0,7,8 |
| 60 | 120 | 1 | 0 | 120 | 1 | 0,7,8 |

NOTE: If the HEVC temporal video subset is either not applicable, not present or not decoded, the HEVC Output Frame Rate is calculated using vui_time_scale, vui_num_units_in_tick and elemental_duration_in_tc_minus1[temporal_id_max](0x24).

## Table 21c — Resolutions for Full-screen Display from HEVC HDR UHDTV2 IRD
_§5.14.6.5, PDF pp. 150-150_

| Horizontal | Vertical | Scan (interlace/progressive) | Coded Frame | Aspect_ratio_idc | Horizontal up-sampling | Vertical up-sampling |
|---|---|---|---|---|---|---|
| 7 680 | 4 320 | P | 16x9 | 1 | × 1 | × 1 |
| 5 120 | 2 880 | P | 16x9 | 1 | × 3/2 | × 3/2 |
| 3 840 | 2 160 | P | 16x9 | 1 | × 2 | × 2 |
| 3 200 | 1 800 | P | 16x9 | 1 | × 12/5 | × 12/5 |
| 2 560 | 1 440 | P | 16x9 | 1 | × 3 | × 3 |
| 1 920 | 1 080 | P | 16x9 | 1 | × 4 | × 4 |
| 1 600 | 900 | P | 16:9 | 1 | × 24/5 | × 24/5 |
| 1 280 | 720 | P | 16:9 | 1 | × 6 | × 6 |
| 960 | 540 | P | 16:9 | 1 | × 8 | × 8 |

_Example up-sampling for 7 680 × 4 320 display._

## Table 22 — drc_decoder_mode_id supported by AC-4
_§6.6.4, PDF pp. 167-167_

| Value of drc_decoder_mode_id | DRC decoder mode | Output level range in LUFS |
|---|---|---|
| 0 | Home Theatre | -31...-27 |
| 1 | Flat panel TV | -26...-17 |
| 2 | Portable - Speakers | -16...0 |
| 3 | Portable - Headphones | -16...0 |

## Table 23 — (E-)AC-3 profiles supported by AC-4
_§6.6.7, PDF pp. 168-168_

| drc_eac3_profile | Profile |
|---|---|
| 0 | None |
| 1 | Film standard |
| 2 | Film light |
| 3 | Music standard |
| 4 | Music light |
| 5 | Speech |

## Table 28 — DTS-UHD BroadcastChunk
_§6.9.3.1, PDF pp. 185-186_

| Syntax | Number of bits | Identifier |
|---|---|---|
| DTSUHD_BCHUNK | 32 | bslbf |
| ByteCount | 8 | uimsbf |
| Version | 3 | uimsbf |
| numLanguages | 5 | uimsbf |
| for (i=0; i ≤ numLanguages; i++) { // LanguageIndex = i | | |
| ISO639_code // Language Table | 24 | bslbf |
| } | | |
| for (i=0; i ≤ numLanguages; i++) { // Language Groups | | |
| b_UserByte | 1 | bslbf |
| reserved_bits | 2 | blsbl |
| numSelectionSets [i] // Preselections per group | 5 | uimsbf |
| for (j = 0; j ≤ numSelectionSets [i]; j++) { // ProgramIndex = j | | |
| AudioDescription // properties of Preselection | 1 | bslbf |
| SpokenSubtitle | 1 | bslbf |
| DialogueEnhancement | 1 | bslbf |
| if (b_UserByte) | | |
| UserByte | 8 | bslbf |
| numComponents | 3 | uimsbf |
| reserved_bits | 2 | bslbf |
| for (k = 0; k ≤ numComponentGroups; k++) { // each Preselection | | |
| StreamID | 3 | uimsbf |
| ComponentGroupID | 5 | uimsbf |
| } // numComponentGroups | | |
| } //numSelectionSets | | |
| } //numLanguages | | |
| CRC16 | 16 | bslbf |

## Table 29 — DTS-UHD Syncwords
_§6.9.7, PDF pp. 188-188_

| Name | Syncword | Description |
|---|---|---|
| DTSUHD_SYNC | 0x40411BF2 | DTS-UHD Sync Frame |
| DTSUHD_NOSYNC | 0x71C442E8 | DTS-UHD Non-sync Frame |
| DTSUHD_BCHUNK | 0x2A3E2523 | DTS-UHD BroadcastChunk |

## §4.1.7 — Program Specific Information (PSI) repetition (hand-transcribed)
_§4.1.7, PDF pp. 49-49 (cites Rec. ITU-T H.222.0 / ISO/IEC 13818-1 §2.4.4)_

The geometry-based extractor targets bit-syntax/value tables; §4.1.7 is prose,
so it is **hand-transcribed** here verbatim (2026-06-11) from the vendored PDF.
This is the authoritative source for the PAT/PMT **100 ms** repetition figure
(distinct from the 0,5 s monitoring ceiling in TR 101 290 §5.2.1 — see
`tr_101_290.md`).

> The Program Association Table (PAT) and Program Map Table (PMT) should be
> repeated with a maximum time interval of 100 ms between repetitions. In
> distribution applications, the maximum time interval between repetitions of
> each of these tables **shall be 100 ms**.

Reading for the dvb-si `SiMux` defaults: PAT/PMT carry **no repetition rate in
TR 101 211** (which covers DVB SI only) and **none in ISO/IEC 13818-1**
(§2.4.1 is general coding structure; the spec's only timing bounds are PCR
100 ms / SCR 700 ms). TS 101 154 §4.1.7 is therefore the tightest authoritative
mandate (a `shall` for distribution), and TR 101 290 §5.2.1 is the looser
monitoring ceiling (0,5 s). The SiMux default cites this clause.
