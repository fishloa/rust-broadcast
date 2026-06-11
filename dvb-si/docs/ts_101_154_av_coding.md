# DVB A001r18 (draft TS 101 154 v2.7.1) — Use of Video and Audio Coding in Broadcast and Broadband Applications

Reference transcribed from the canonical PDF (`specs/dvb_a001r18_draft_ts_101_154_v02.07.01_av_coding.pdf`) by the
geometry-based extractor in `tools/dvb-si-audit/` — field rows aligned to
their bit-widths by page geometry, reproduced verbatim. The PDF in `specs/`
is the authoritative source.

## Contents

- [Table 3 — Values for display_horizontal_size](#table-3-values-for-display_horizontal_size)
- [Table 4 — Resolutions for Full-screen Display from 25 Hz MPEG-2 SDTV IRD](#table-4-resolutions-for-full-screen-display-from-25-hz-mpeg-2-sdtv-ird)
- [Table 5 — Values for display_horizontal_size](#table-5-values-for-display_horizontal_size)
- [Table 6 — Resolutions for Full-screen Display from 30 Hz MPEG-2 SDTV IRD](#table-6-resolutions-for-full-screen-display-from-30-hz-mpeg-2-sdtv-ird)
- [Table 7 — time_scale and num_units_in_tick for Progressive and Interlace](#table-7-time_scale-and-num_units_in_tick-for-progressive-and-interlace)
- [Table 8 — Resolutions for Full-screen Display from 25 Hz H.264/AVC SDTV IRD](#table-8-resolutions-for-full-screen-display-from-25-hz-h264avc-sdtv-ird)
- [Table 9 — Time_scal and num_units_in_tick for Progressive and Interlace](#table-9-time_scal-and-num_units_in_tick-for-progressive-and-interlace)
- [Table 10 — Resolutions for Full-screen Display from 30 Hz H.264/AVC SDTV IRD,](#table-10-resolutions-for-full-screen-display-from-30-hz-h264avc-sdtv-ird)
- [Table 11 — Resolutions for Full-screen Display from H.264/AVC HDTV IRD and SVC HDTV IRD](#table-11-resolutions-for-full-screen-display-from-h264avc-hdtv-ird-and-svc-hdtv-ird)
- [Table 12 — Time_scal and num_units_in_tick for Progressive and Interlace Frame Rates for](#table-12-time_scal-and-num_units_in_tick-for-progressive-and-interlace-frame-rates-for)
- [Table 13 — Time_scal and num_units_in_tick for Progressive and Interlace Frame Rates for](#table-13-time_scal-and-num_units_in_tick-for-progressive-and-interlace-frame-rates-for)
- [Table 14 — Resolutions for Full-screen Display from 25 Hz VC-1 SDTV IRD](#table-14-resolutions-for-full-screen-display-from-25-hz-vc-1-sdtv-ird)
- [Table 15 — Resolutions for Full-screen Display from 25 Hz VC-1 HDTV IRD](#table-15-resolutions-for-full-screen-display-from-25-hz-vc-1-hdtv-ird)
- [Table 16 — Resolutions for Full-screen Display from 30 Hz VC-1 SDTV IRD](#table-16-resolutions-for-full-screen-display-from-30-hz-vc-1-sdtv-ird)
- [Table 17 — Resolutions for Full-screen Display from 30 Hz VC-1 HDTV IRD](#table-17-resolutions-for-full-screen-display-from-30-hz-vc-1-hdtv-ird)
- [Table 18 — Resolutions for Full-screen Display from MVC Stereo HDTV IRD](#table-18-resolutions-for-full-screen-display-from-mvc-stereo-hdtv-ird)
- [Table 18a — HEVC IRD conformance points specified in the present document](#table-18a-hevc-ird-conformance-points-specified-in-the-present-document)
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

| horizontal_size × |  |  |
|---|---|---|
|  | Source aspect ratio | Display_horizontal_size |
| vertical_size |  |  |
| 720 × 576 | 16:9 | 540 |
| 544 × 576 | 16:9 | 408 |
| 480 × 576 | 16:9 | 360 |
| 352 × 576 | 16:9 | 264 |
| 352 × 288 | 16:9 | 264 |

## Table 4 — Resolutions for Full-screen Display from 25 Hz MPEG-2 SDTV IRD
_§5.1.4, PDF pp. 58-58_

| Coded Picture |  | Displayed | Picture |
|---|---|---|---|
|  |  | Horizontal up | sampling |
| Luminance resolution | Aspect Ratio | 4:3 Monitors | 16:9 Monitors |
| (horizontal  vertical) |  |  |  |

## Table 5 — Values for display_horizontal_size
_§5.3.3, PDF pp. 63-63_

| horizontal_size  |  |  |
|---|---|---|
|  | Source aspect ratio | Display_horizontal_size |
| vertical_size |  |  |
| 720  480 | 16:9 | 540 |
| 640  480 | 16:9 | 480 |
| 544  480 | 16:9 | 408 |
| 480  480 | 16:9 | 360 |
| 352  480 | 16:9 | 264 |
| 352  240 | 16:9 | 264 |

## Table 6 — Resolutions for Full-screen Display from 30 Hz MPEG-2 SDTV IRD
_§5.3.5, PDF pp. 65-65_

| Coded Picture |  | Displayed | Picture |
|---|---|---|---|
|  |  | Horizontal up | sampling |
| Luminance resolution | Aspect Ratio | 4:3 Monitors | 16:9 Monitors |
| (horizontal  vertical) |  |  |  |

## Table 7 — time_scale and num_units_in_tick for Progressive and Interlace
_§5.6.2.3, PDF pp. 75-75_

| Frame Rate | Interlaced or | time_scale | Num_units_in_tick |
|---|---|---|---|
|  | Progressive |  |  |
| 25 | P | 50 | 1 |
| 25 | I | 50 | 1 |

## Table 8 — Resolutions for Full-screen Display from 25 Hz H.264/AVC SDTV IRD
_§5.6.3.1, PDF pp. 76-76_

|  |  |  | Displayed | Picture |
|---|---|---|---|---|
| Coded | Picture |  |  |  |
|  |  |  | Horizontal up | sampling |
| Luminance resolution | Source Aspect | Aspect_ratio_idc |  |  |
|  |  |  | 4:3 Monitors | 16:9 Monitors |
| (horizontal × vertical) | Ratio |  |  |  |
| 720 × 576 | 4:3 | 2 | × 1 | × 3/4 (see note 1) |
|  | 16:9 | 4 | × 4/3 (see note 2) | × 1 |
| 544 × 576 | 4:3 | 4 | × 4/3 | × 1 (see note 1) |
|  | 16:9 | 12 | × 16/9 (see note 2) | × 4/3 |
| 480 × 576 | 4:3 | 10 | × 3/2 | × 9/8 (see note 1) |
|  | 16:9 | 6 | × 2 (see note 2) | × 3/2 |
| 352 × 576 | 4:3 | 6 | × 2 | × 3/2 (see note 1) |
|  | 16:9 | 8 | × 8/3 (see note 2) | × 2 |

## Table 9 — Time_scal and num_units_in_tick for Progressive and Interlace
_§5.6.3.3, PDF pp. 77-77_

| Frame Rate | Interlaced or | time_scale | Num_units_in_tick |
|---|---|---|---|
|  | Progressive |  |  |
| 24 000/ 1 001 | P | 48 000 | 1 001 |
| 24 | P | 48 | 1 |
| 30 000/ 1 001 | P | 60 000 | 1 001 |
| 30 | P | 60 | 1 |
| 30 000/ 1 001 | I | 60 000 | 1 001 |
| 30 | I | 60 | 1 |

## Table 10 — Resolutions for Full-screen Display from 30 Hz H.264/AVC SDTV IRD,
_§5.7.1.1, PDF pp. 78-78_

|  |  |  | Displayed | Picture |
|---|---|---|---|---|
| Coded | Picture |  |  |  |
|  |  |  | Horizontal up | sampling |
| Luminance resolution | Source Aspect | aspect_ratio |  |  |
|  |  |  | 4:3 Monitors | 16:9 Monitors |
| (horizontal × vertical) | Ratio | _idc |  |  |
| 720 × 480 | 4:3 | 3 | × 1 | × 3/4 (see note 1) |
|  | 16:9 | 5 | × 4/3 (see note 2) | × 1 |
| 640 × 480 | 4:3 | 1 | × 9/8 | × 27/32 (see note 1) |
|  | 16:9 | 14 | × 3/2 | × 9/8 |
| 544 × 480 | 4:3 | 5 | × 4/3 | × 1 (see note 1) |
|  | 16:9 | 13 | × 16/9 (see note 2) | × 4/3 |
| 480 × 480 | 4:3 | 11 | × 3/2 | × 9/8 (see note 1) |
|  | 16:9 | 7 | × 2 (see note 2) | × 3/2 |
| 352 × 480 | 4:3 | 7 | × 2 | × 3/2 (see note 1) |
|  | 16:9 | 9 | × 8/3 (see note 2) | × 2 |

## Table 11 — Resolutions for Full-screen Display from H.264/AVC HDTV IRD and SVC HDTV IRD
_§5.7.2.2, PDF pp. 80-80_

|  | Coded | Picture |  |
|---|---|---|---|
| Luminance resolution | Source Aspect | aspect_ratio_idc | 16:9 Monitors |
| (horizontal × vertical) | Ratio |  | Horizontal up sampling |
| 1 920 × 1 080 | 16:9 | 1 | × 1 |
| 1 440 × 1 080 | 16:9 | 14 | × 4/3 |
| 1 280 × 1 080 | 16:9 | 15 | × 3/2 |
| 960 × 1 080 | 16:9 | 16 | × 2 |
| 1 280 × 720 | 16:9 | 1 | × 1 |
| 960 × 720 | 16:9 | 14 | × 4/3 |
| 640 × 720 | 16:9 | 16 | × 2 |

## Table 12 — Time_scal and num_units_in_tick for Progressive and Interlace Frame Rates for
_§5.7.2.2, PDF pp. 80-80_

| Frame Rate | Interlaced or Progressive | time_scale | num_units_in_tick |
|---|---|---|---|
| 25 | P | 50 | 1 |
| 25 | I | 50 | 1 |
| 50 | P | 100 | 1 |

## Table 13 — Time_scal and num_units_in_tick for Progressive and Interlace Frame Rates for
_§5.7.3.3, PDF pp. 81-81_

| Frame Rate | Interlaced or Progressive | time_scale | Num_units_in_tick |
|---|---|---|---|
| 24 000/ 1 001 | P | 48 000 | 1 001 |
| 24 | P | 48 | 1 |
| 30 000/ 1 001 | P | 60 000 | 1 001 |
| 30 | P | 60 | 1 |
| 30 000/ 1 001 | I | 60 000 | 1 001 |
| 30 | I | 60 | 1 |
| 60 000/ 1 001 | P | 120 000 | 1 001 |
| 60 | P | 120 | 1 |

## Table 14 — Resolutions for Full-screen Display from 25 Hz VC-1 SDTV IRD
_§5.9.5, PDF pp. 101-101_

|  |  | Displayed | Picture |
|---|---|---|---|
| Coded | Picture |  |  |
|  |  | Horizontal up | sampling |
| Luminance resolution | Source Video Aspect |  |  |
|  |  | 4:3 Monitors | 16:9 Monitors |
| (horizontal × vertical) | Ratio |  |  |
| 720 × 576 | 4:3 | × 1 | × 3/4 (see note 1) |
|  | 16:9 | × 4/3 (see note 2) | × 1 |
| 544 × 576 | 4:3 | × 4/3 | × 1 (see note 1) |
|  | 16:9 | × 16/9 (see note 2) | × 4/3 |
| 480 × 576 | 4:3 | × 3/2 | × 9/8 (see note 1) |
|  | 16:9 | × 2 (see note 2) | × 3/2 |
| 352 × 576 | 4:3 | × 2 | × 3/2 (see note 1) |
|  | 16:9 | × 8/3 (see note 2) | × 2 |

## Table 15 — Resolutions for Full-screen Display from 25 Hz VC-1 HDTV IRD
_§5.10.6, PDF pp. 103-103_

|  | Coded Picture |  |
|---|---|---|
| Luminance resolution | Source Aspect | 16:9 Monitors |
| (horizontal × vertical) | Ratio | Horizontal up sampling |
| 1 920 × 1 080 | 16:9 | × 1 |
| 1 440 × 1 080 | 16:9 | × 4/3 |
| 1 280 × 1 080 | 16:9 | × 3/2 |
| 960 × 1 080 | 16:9 | × 2 |
| 1 280 × 720 | 16:9 | × 1 |
| 960 × 720 | 16:9 | × 4/3 |
| 640 × 720 | 16:9 | × 2 |

## Table 16 — Resolutions for Full-screen Display from 30 Hz VC-1 SDTV IRD
_§5.11.5, PDF pp. 105-105_

|  |  | Displayed | Picture |
|---|---|---|---|
| Coded | Picture |  |  |
|  |  | Horizontal up | sampling |
| Luminance resolution | Source Video Aspect |  |  |
|  |  | 4:3 Monitors | 16:9 Monitors |
| (horizontal × vertical) | Ratio |  |  |
| 720 × 480 | 4:3 | × 1 | × 3/4 (see note 1) |
|  | 16:9 | × 4/3 (see note 2) | × 1 |
| 640 × 480 | 4:3 | × 9/8 | × 27/32 (see note 1) |
|  | 16:9 | × 3/2 | × 9/8 |
| 544 × 480 | 4:3 | × 4/3 | × 1 (see note 1) |
|  | 16:9 | × 16/9 (see note 2) | × 4/3 |
| 480 × 480 | 4:3 | × 3/2 | × 9/8 (see note 1) |
|  | 16:9 | × 2 (see note 2) | × 3/2 |
| 352 × 480 | 4:3 | × 2 | × 3/2 (see note 1) |
|  | 16:9 | × 8/3 (see note 2) | × 2 |

## Table 17 — Resolutions for Full-screen Display from 30 Hz VC-1 HDTV IRD
_§5.12.6, PDF pp. 107-107_

|  | Coded Picture |  |
|---|---|---|
| Luminance resolution | Source Aspect | 16:9 Monitors |
| (horizontal × vertical) | Ratio | Horizontal up sampling |
| 1 920 × 1 080 | 16:9 | × 1 |
| 1 440 × 1 080 | 16:9 | × 4/3 |
| 1 280 × 1 080 | 16:9 | × 3/2 |
| 960 × 1 080 | 16:9 | × 2 |
| 1 280 × 720 | 16:9 | × 1 |
| 960 × 720 | 16:9 | × 4/3 |
| 640 × 720 | 16:9 | × 2 |

## Table 18 — Resolutions for Full-screen Display from MVC Stereo HDTV IRD
_§5.13.1.7, PDF pp. 111-111_

|  | Coded | Picture |  |
|---|---|---|---|
| Luminance resolution | Source Aspect | aspect_ratio_idc | 16:9 Monitors |
| (horizontal × vertical) | Ratio |  | Horizontal up sampling |
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
| 50 Hz HEVC HDTV 8-bit | 5.14.1 (with constraints set as documented for 50 Hz HEVC HDTV IRDs in 5.14.1.7) |
| IRD | 5.14.2 (with constraints set as documented for HEVC HDTV 8-bit IRDs in 5.14.2.1) |
| 60 Hz HEVC HDTV 8-bit | 5.14.1 (with constraints set as documented for 60 Hz HEVC HDTV IRDs in 5.14.1.7) |
| IRD | 5.14.2 (with constraints set as documented for HEVC HDTV 8-bit IRDs in 5.14.2.1) |
| 50 Hz HEVC HDTV 10-bit | 5.14.1 (with constraints set as documented for 50 Hz HEVC HDTV IRDs in 5.14.1.7) |
| IRD | 5.14.2 (with constraints set as documented for HEVC HDTV 10-bit IRDs in 5.14.2.1) |
| 60 Hz HEVC HDTV 10-bit | 5.14.1 (with constraints set as documented for 60 Hz HEVC HDTV IRDs in 5.14.1.7) |
| IRD | 5.14.2 (with constraints set as documented for HEVC HDTV 10-bit IRDs in 5.14.2.1) |
| HEVC UHDTV IRD | 5.14.1 |
|  | 5.14.3 |
| HEVC HDR UHDTV IRD | 5.14.1 |
| using HLG10 | 5.14.4 (with constraints set as documented for HLG10 in 5.14.4.4.2) |
| HEVC HDR UHDTV IRD | 5.14.1 |
| using PQ10 | 5.14.4 (with constraints set as documented for PQ10 in 5.14.4.4.3) |
| HEVC HDR HFR UHDTV | 5.14.1 |
| IRD using HLG10 | 5.14.5 (with constraint set as documented for HLG10) |
| HEVC HDR HFR UHDTV | 5.14.1 |
| IRD using PQ10 | 5.14.5 (with constraints set as documented for PQ10) |
| HEVC HDR UHDTV2 IRD | 5.14.1 |
|  | 5.14.6 |

## Table 19 — Progressive and Interlaced Frame Rates for HEVC Bitstreams
_§5.14.1.7, PDF pp. 127-127_

| 24 000/1 001 | P | 0 | 24 000 | 1 001 | 0,7,8 |
|---|---|---|---|---|---|
| 24 | P | 0 | 24 | 1 | 0,7,8 |
| 25 | P | 0 | 25 | 1 | 0,7,8 |
| 25 | I (encoded as |  |  |  | 3,4,5,6 |
|  | frames) | 0 | 50 | 1 |  |
| 25 | I (encoded as |  |  |  | 9,10,11,12 |
|  | fields) | 0 | 50 | 1 |  |
| 30 000/1 001 | P | 0 | 30 000 | 1 001 | 0,7,8 |
| 30 000/1 001 | I (encoded as |  |  |  | 3,4,5,6 |
|  | frames) | 0 | 60 000 | 1 001 |  |
| 30 000/1 001 | I (encoded as |  |  |  | 9,10,11,12 |
|  | fields) | 0 | 60 000 | 1 001 |  |
| 30 | P | 0 | 30 | 1 | 0,7,8 |
| 50 | P | 0 | 50 | 1 | 0,7,8 |
| 60 000/1 001 | P | 0 | 60 000 | 1 001 | 0,7,8 |
| 60 | P | 0 | 60 | 1 | 0,7,8 |

## Table 20 — Resolutions for Full-screen Display from HEVC HDTV IRD
_§5.14.3.1, PDF pp. 132-132_

| Luminance | resolution | Scan | Aspect | ratio | Example up-sampling |  |
|---|---|---|---|---|---|---|
|  |  | (interlace/ |  |  | for 1 920 x 1 080 | display |
| Horizontal | Vertical | progressive) | Coded | Aspect_ratio_i | Horizontal | Vertical |
|  |  |  | Frame | dc |  |  |
| 1 920 | 1 080 | I and P | 16:9 | 1 | x 1 | x 1 |
| 1 440 | 1 080 | I and P | 16:9 | 14 | x 4/3 | x 1 |
| 1 600 | 900 | P | 16:9 | 1 | x 6/5 | x 6/5 |
| 1 280 | 720 | P | 16:9 | 1 | x 3/2 | x 3/2 |
| 960 | 720 | P | 16:9 | 14 | x 2 | x 3/2 |
| 960 | 540 | P | 16:9 | 1 | x 2 | x 2 |

## Table 21 — Resolutions for Full-screen Display from HEVC UHDTV IRD
_§5.14.3.4, PDF pp. 134-134_

| Luminance | resolution | Scan | Aspect | ratio | Example up-sampling | for |
|---|---|---|---|---|---|---|
|  |  | (interlace/ |  |  | 3 840 x 2 160 | display |
| Horizontal | Vertical | progressive) | Coded | Aspect_ratio_i | Horizontal | Vertical |
|  |  |  | Frame | dc |  |  |
| 3 840 | 2 160 | P | 16x9 | 1 | 1 | 1 |
| 2 880 | 2 160 | P | 16x9 | 14 | x 4/3 | x 1 |
| 3 200 | 1 800 | P | 16x9 | 1 | x 6/5 | x 6/5 |
| 2 560 | 1 440 | P | 16x9 | 1 | x 3/2 | x 3/2 |

## Table 21a — Resolutions for Full-screen Display from HEVC HDR UHDTV IRD
_§5.14.4.3, PDF pp. 135-135_

| Luminance | resolution | Scan | Aspect | ratio | Example up-sampling |  |
|---|---|---|---|---|---|---|
|  |  | (interlace/prog |  |  | for 3 840 x 2 160 | display |
| Horizontal | Vertical | ressive) | Coded | Aspect_ratio_i | Horizontal | Vertical |
|  |  |  | Frame | dc |  |  |
| 3 840 | 2 160 | P | 16:9 | 1 | x 1 | x 1 |
| 3 200 | 1 800 | P | 16:9 | 1 | x 6/5 | x 6/5 |
| 2 560 | 1 440 | P | 16:9 | 1 | x 3/2 | x 3/2 |
| 1 920 | 1 080 | P | 16:9 | 1 | x 2 | x 2 |
| 1 600 | 900 | P | 16:9 | 1 | x 12/5 | x 12/5 |
| 1 280 | 720 | P | 16:9 | 1 | x 3 | x 3 |
| 960 | 540 | P | 16:9 | 1 | x 4 | x 4 |

## Table 21b — Progressive Frame Rates for HEVC HFR UHDTV Bitstreams
_§5.14.5.5.1, PDF pp. 145-145_

|  |  |  |  | Stream Type: 0x24 |  |  |  |  |  |  |  |  | k |  |  |  |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
|  |  |  |  | (H E V C b i t s t r e a m |  | St r e a m T y p e : 0 x 2 5 |  |  |  |  |  |  | c |  |  |  |
|  |  |  |  |  |  |  |  |  |  |  |  |  | it_ |  |  | tc |
| Output Frame |  | Rate (fps) |  | an d H E V C t e m p o ra l |  | ( H E V C t e m p o r a l |  |  | e |  |  |  |  |  |  | u |
|  |  |  |  |  |  |  |  |  | la |  |  |  | n |  |  | r |
|  |  |  |  |  |  |  |  |  |  |  |  |  | i_ |  |  | ts |
|  |  |  |  | v i d e o s u b - |  | v i d e o s u b s e t ) |  |  | c |  |  |  |  |  |  |  |
|  |  |  |  |  |  |  |  |  | s |  |  |  | s |  |  | _ |
|  |  |  |  | bitstream) |  |  |  |  | _ |  |  |  | tin |  |  | c |
|  |  |  |  |  |  |  |  |  | e |  |  |  |  |  |  | ip |
|  |  |  |  | e l e m e n t a l_ |  | e l e m e n t a l_ |  |  | m |  |  |  | u |  |  |  |
|  |  |  |  |  |  |  |  |  | it_ |  |  |  | _ |  |  | d |
|  |  | H E V C H D R |  | d u r a t i o n |  | d u r a t i o n |  |  |  |  |  |  | m |  |  | e |
| HEVC U H DTV |  |  |  |  |  |  |  |  | iu |  |  |  |  |  |  | w |
|  |  |  |  |  |  |  |  |  |  |  |  |  | u |  |  |  |
|  |  | H F R |  | _ in _ t c _ m i n u s 1 |  | _ i n _ t c _ m i n u s 1 |  |  |  |  |  |  | n |  |  | o |
| I R D |  |  |  |  |  |  |  |  | v |  |  |  |  |  |  | llA |
|  |  | U H D T V I R D |  | [ t e m p o r a l _ i d |  | [ t e m p o r a l _ i d |  |  |  |  |  |  | _ |  |  |  |
|  |  |  |  |  |  |  |  |  |  |  |  |  | iu |  |  |  |
|  |  |  |  | _max](0x24) |  | _max](0x25) |  |  |  |  |  |  | v |  |  |  |
| Not applicable |  | 100 |  | 0 |  | Not applicable |  | 100 |  |  |  |  | 1 |  |  | 0,7,8 |
| 50 |  | 100 |  | 1 |  | 0 |  | 100 |  |  |  |  | 1 |  |  | 0,7,8 |
| Not applicable |  | 120 000/1 001 |  | 0 |  | Not applicable |  | 120 | 000 |  |  | 1 | 001 |  |  | 0,7,8 |
| 60 000/1 001 |  | 120 000/1 001 |  | 1 |  | 0 |  | 120 | 000 |  |  | 1 | 001 |  |  | 0,7,8 |
| Not applicable |  | 120 |  | 0 |  | Not applicable |  | 120 |  |  |  |  | 1 |  |  | 0,7,8 |
| 60 |  | 120 |  | 1 |  | 0 |  | 120 |  |  |  |  | 1 |  |  | 0,7,8 |

## Table 21c — Resolutions for Full-screen Display from HEVC HDR UHDTV2 IRD
_§5.14.6.5, PDF pp. 150-150_

| Luminance | resolution | Scan | Aspect | ratio | Example up-sampling | for |
|---|---|---|---|---|---|---|
|  |  | (interlace/ |  |  | 7 680 x 4 320 | display |
| Horizontal | Vertical | progressive) | Coded | Aspect_ratio_i | Horizontal | Vertical |
|  |  |  | Frame | dc |  |  |
| 7 680 | 4 320 | P | 16x9 | 1 | x 1 | x 1 |
| 5 120 | 2 880 | P | 16x9 | 1 | x 3/2 | x 3/2 |
| 3 840 | 2 160 | P | 16x9 | 1 | x 2 | x 2 |
| 3 200 | 1 800 | P | 16x9 | 1 | x 12/5 | x 12/5 |
| 2 560 | 1 440 | P | 16x9 | 1 | x 3 | x 3 |
| 1 920 | 1 080 | P | 16x9 | 1 | x 4 | x 4 |
| 1 600 | 900 | P | 16:9 | 1 | x 24/5 | x 24/5 |
| 1 280 | 720 | P | 16:9 | 1 | x 6 | x 6 |
| 960 | 540 | P | 16:9 | 1 | x 8 | x 8 |

## Table 22 — drc_decoder_mode_id supported by AC-4
_§6.6.4, PDF pp. 167-167_

| Value of | DRC decoder mode | Output level range |
|---|---|---|
| drc_decoder_mode_id |  | in LUFS |
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

| Syntax | Number of | Identifier |
|---|---|---|
|  | bits |  |
| DTSUHD_BCHUNK | 32 | bslbf |
| ByteCount | 8 | uimsbf |
| Version | 3 | uimsbf |
| numLanguages | 5 | uimsbf |
| for (i=0; i ≤ numLanguages; i++) { // LanguageIndex = i |  |  |
| ISO639_code // Language Table | 24 | bslbf |
| } |  |  |
| for (i=0; i ≤ numLanguages; i++) { // Language Groups |  |  |
| b_UserByte | 1 | bslbf |
| reserved_bits | 2 | blsbl |
| Syntax | Number of | Identifier |
|  | bits |  |
| numSelectionSets [i] // Preselections per group | 5 | uimsbf |
| for (j = 0; j ≤ numSelectionSets [i]; j++) { // ProgramIndex = j |  |  |
| AudioDescription // properties of Preselection | 1 | bslbf |
| SpokenSubtitle | 1 | bslbf |
| DialogueEnhancement | 1 | bslbf |
| if (b_UserByte) |  |  |
| UserByte | 8 | bslbf |
| numComponents | 3 | uimsbf |
| reserved_bits | 2 | bslbf |
| for (k = 0; k ≤ numComponentGroups; k++) { // each Preselection |  |  |
| StreamID | 3 | uimsbf |
| ComponentGroupID | 5 | uimsbf |
| } // numComponentGroups |  |  |
| } //numSelectionSets |  |  |
| } //numLanguages |  |  |
| CRC16 | 16 | bslbf |

## Table 29 — DTS-UHD Syncwords
_§6.9.7, PDF pp. 188-307_

| Name | Syncword | Description |  |  |  |  |
|---|---|---|---|---|---|---|
| DTSUHD_SYNC | 0x40411BF2 | DTS-UHD Sync Frame |  |  |  |  |
| DTSUHD_NOSYNC | 0x71C442E8 | DTS-UHD Non-sync Frame |  |  |  |  |
| DTSUHD_BCHUNK | 0x2A3E2523 | DTS-UHD BroadcastChunk |  |  |  |  |
| 1 080 | 1 920 | 16:9 | 25 |  | P | N |
| 1 080 | 1 920 | 16:9 | 23,976; 24; |  | P | N |
|  |  |  | 29,97; 30 |  |  |  |
| 1 080 | 1 920 | 16:9 | 25 |  | I | N |
| 1 080 | 1 920 | 16:9 | 29,97; 30 |  | I | N |
| 720 | 1 280 | 16:9 | 25; 50 |  | P | N |
| 720 | 1 280 | 16:9 | 23,976; 24; |  | P | N |
|  |  |  | 29,97; 30; 59,94; |  |  |  |
|  |  |  | 60 |  |  |  |
| 576 | 720 | 16:9 | 50 |  | P | N |
| 576 | 720 | 4:3, 16:9 | 25 |  | P | Y |
| 576 | 720 | 4:3, 16:9 | 25 |  | I | Y |
| 576 | 544 | 4:3, 16:9 | 25 |  | P | Y |
| 576 | 544 | 4:3, 16:9 | 25 |  | I | Y |
| 576 | 480 | 4:3, 16:9 | 25 |  | P | Y |
| 576 | 480 | 4:3, 16:9 | 25 |  | I | Y |
| 576 | 352 | 4:3, 16:9 | 25 |  | P | Y |
| 576 | 352 | 4:3, 16:9 | 25 |  | I | Y |
| 480 | 720 | 16:9 | 59,94; 60 |  | P | N |
| 480 | 720 | 4:3, 16:9 | 23,976; 24; |  | P | Y |
|  |  |  | 29,97; 30 |  |  |  |
| 480 | 720 | 4:3, 16:9 | 29,97; 30 |  | I | Y |
| 480 | 640 | 4:3 | 23,976; 24; |  | P | Y |
|  |  |  | 29,97; 30 |  |  |  |
| 480 | 640 | 4:3 | 29,97; 30 |  | I | Y |
| 480 | 544 | 4:3, 16:9 | 23,976; 29,97 |  | P | Y |
| 480 | 544 | 4:3, 16:9 | 29,97 |  | I | Y |
| 480 | 480 | 4:3, 16:9 | 23,976; 29,97 |  | P | Y |
| 480 | 480 | 4:3, 16:9 | 29,97 |  | I | Y |
| 480 | 352 | 4:3, 16:9 | 23,976; 29,97 |  | P | Y |
| 480 | 352 | 4:3, 16:9 | 29,97 |  | I | Y |
| 288 | 352 | 4:3, 16:9 | 25 |  | P | Y |
| 240 | 352 | 4:3, 16:9 | 23,976; 29,97 |  | P | Y |
| NOTE: Shaded | "frame_rate_code" values | indicate 30 Hz | bitstreams, clear |  | values 25 Hz bitstreams. |  |
| Vertical | Horizontal size | Aspect ratio | Frame rate |  | Progressive | Decodable by |
| size |  |  | (see note) |  | or Interlaced | H.264/AVC SDTV IRD |
| 1 080 | 1 920, 1 440, | 16:9 | 23,976; 24 |  | P | N |
|  | 1 280, 960 |  | 25 |  | I | N |
|  |  |  |  |  | P | N |
|  |  |  | 29,97; 30 |  | I | N |
| 720 | 1 280, 960, 640 | 16:9 | 25; 50 |  | P | N |
|  |  |  | 23,976; 24; 29,97; |  | P | N |
|  |  |  | 30; 59,94; 60 |  |  |  |
| 576 | 720 | 4:3, 16:9 | 25 |  | P | Y |
|  |  |  |  |  | I | Y |
|  | 544, 480, 352 | 4:3, 16:9 | 25 |  | P | Y |
|  |  |  |  |  | I | Y |
| 480 | 720, 640, 544, | 4:3, 16:9 | 23,976; 24; 29,97; |  | P | Y |
|  | 480, 352 |  | 30 |  |  |  |
|  |  |  | 29,97; 30 |  | I | Y |
| 288 | 352 | 4:3 | 25; 50 |  | P | Y |
|  |  |  | 25 |  | I | Y |
| 240 | 352 | 4:3 | 23,976; 24; 29,97; |  | P | Y |
|  |  |  | 30; 59,94; 60 |  |  |  |
|  |  |  | 29,97; 30 |  | I | Y |
| NOTE: Shaded | "frame_rate_code" | values indicate | 30 Hz bitstreams, | clear | values 25 Hz | bitstreams. |
| Vertical | Horizontal size | Aspect ratio | Frame rate |  | Progressive | Decodable by VC-1 |
| size |  |  | (see note) |  | or Interlaced | SDTV IRD |
| 1 080 | 1 920, 1 440, | 16:9 | 23,976; 24 |  | P | N |
|  | 1 280, 960 |  | 25 |  | I | N |
|  |  |  |  |  | P | N |
|  |  |  | 29,97; 30 |  | I | N |
| 720 | 1 280, 960, 640 | 16:9 | 25; 50 |  | P | N |
|  |  |  | 23,976; 24; 29,97; 30; |  | P | N |
|  |  |  | 59,94; 60 |  |  |  |
| 576 | 720 | 4:3, 16:9 | 25 |  | P | Y |
|  |  |  |  |  | I | Y |
|  | 544, 480, 352 | 4:3, 16:9 | 25 |  | P | Y |
|  |  |  |  |  | I | Y |
| 480 | 720, 640, 544, | 4:3, 16:9 | 23,976; 24; 29,97; 30 |  | P | Y |
|  | 480, 352 |  | 29,97; 30 |  | I | Y |
| 288 | 352 | 4:3 | 25; 50 |  | P | Y |
|  |  |  | 25 |  | I | Y |
| 240 | 352 | 4:3 | 23,976; 24; 29,97; 30; |  | P | Y |
|  |  |  | 59,94; 60 |  |  |  |
|  |  |  | 29,97; 30 |  | I | Y |
| NOTE: Shaded | "frame_rate" values | indicate 30 | Hz bitstreams, clear values |  | 25 Hz bitstreams. |  |
| Vertical size | Horizontal size | Display aspect | Frame rate |  | Progressive or | Decodable by |
|  |  | ratio | (see note) |  | Interlaced | HEVC HDTV IRD |
| 2 160 | 3 840, 2 880 | 16:9 | 25, 50 |  | P | N |
|  |  |  | 24 000/1 001, 24, |  | P | N |
|  |  |  | 30 000/1 001, 30, |  |  |  |
|  |  |  | 60 000/1 001, 60 |  |  |  |
| 1 800 | 3 200 | 16:9 | 25, 50 |  | P | N |
|  |  |  | 24 000/1 001, 24, |  | P | N |
|  |  |  | 30 000/1 001, 30, |  |  |  |
|  |  |  | 60 000/1 001, 60 |  |  |  |
| 1 440 | 2 560 | 16:9 | 25, 50 |  | P | N |
|  |  |  | 24 000/1 001, 24, |  | P | N |
|  |  |  | 30 000/1 001, 30, |  |  |  |
|  |  |  | 60 000/1 001, 60 |  |  |  |
| 1 080 | 1 920, 1 440 | 16:9 | 24 000/1 001, 24 |  | P | Y |
|  |  |  | 25 |  | I | Y |
|  |  |  |  |  | P | Y |
|  |  |  | 50 |  | P | Y |
|  |  |  | 30 000/1 001, 30 |  | I | Y |
|  |  |  |  |  | P | Y |
|  |  |  | 60 000/1 001, 60 |  | P | Y |
| 900 | 1 600 | 16:9 | 25, 50 |  | P | Y |
|  |  |  | 24 000/1 001, 24, |  | P | Y |
|  |  |  | 30 000/1 001, 30, |  |  |  |
|  |  |  | 60 000/1 001, 60 |  |  |  |
| 720 | 1 280, 960 | 16:9 | 25, 50 |  | P | Y |
|  |  |  | 24 000/1 001, 24, |  | P | Y |
|  |  |  | 30 000/1 001, 30, |  |  |  |
|  |  |  | 60 000/1 001, 60 |  |  |  |
| 540 | 960 | 16:9 | 25, 50 |  | P | Y |
|  |  |  | 24 000/1 001, 24, |  | P | Y |
|  |  |  | 30 000/1 001, 30, |  |  |  |
|  |  |  | 60 000/1 001, 60 |  |  |  |
| NOTE: Shaded | frame rate values | indicate bitstreams | that might not be able |  | to be decoded | by 50 Hz HEVC |
| HDTV | IRDs. |  |  |  |  |  |
| user_identifier | user_structure() |  |  |  |  |  |
| 0x47413934 ('GA94') | DVB1_data() |  |  |  |  |  |
| 0x44544731 ('DTG1') | afd_data() |  |  |  |  |  |
| Syntax | No. of Bits | Identifier |  |  |  |  |
| afd_data() { |  |  |  |  |  |  |
| '0' | 1 | bslbf |  |  |  |  |
| active_format_flag | 1 | bslbf |  |  |  |  |
| reserved (set to '00 0001') | 6 | bslbf |  |  |  |  |
| if (active_format_flag == 1) { |  |  |  |  |  |  |
| reserved (set to '1111' ) | 4 | bslbf |  |  |  |  |
| active_format | 4 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| } |  |  |  |  |  |  |
| Syntax | No. of Bits | Identifier |  |  |  |  |
| DVB1_data() { |  |  |  |  |  |  |
| user_data_type_code | 8 | uimsbf |  |  |  |  |
| user_data_type_structure() |  |  |  |  |  |  |
| } |  |  |  |  |  |  |
| user_data_type_code | user_data_type_structure() |  |  |  |  |  |
| 0x00 to 0x02 | Industry Reserved (see code point |  |  |  |  |  |
|  | registry [i.36]) |  |  |  |  |  |
| 0x03 | cc_data() |  |  |  |  |  |
| 0x04 | Industry Reserved (see code point |  |  |  |  |  |
|  | registry [i.36]) |  |  |  |  |  |
| 0x05 | Industry Reserved (see code point |  |  |  |  |  |
|  | registry [i.36]) |  |  |  |  |  |
| 0x06 | bar_data() |  |  |  |  |  |
| 0x07 | multi_region_disparity() |  |  |  |  |  |
| 0x08 | Industry Reserved (see code point |  |  |  |  |  |
|  | registry [i.36]) |  |  |  |  |  |
| 0x09 | ST2094-10_data() |  |  |  |  |  |
| 0x0A to 0xFF | Industry Reserved (see code point |  |  |  |  |  |
|  | registry [i.36]) |  |  |  |  |  |
| Active_format | Aspect ratio of the "area of interest" |  |  |  |  |  |
| 0000 | AFD unknown (see below) |  |  |  |  |  |
| 0001 | Reserved |  |  |  |  |  |
| 0010 | box 16:9 (top) |  |  |  |  |  |
| 0011 | box 14:9 (top) |  |  |  |  |  |
| 0100 | box > 16:9 (centre) |  |  |  |  |  |
| 0101 to 0111 | Reserved |  |  |  |  |  |
| 1000 | Active format is the same as the coded frame |  |  |  |  |  |
| 1001 | 4:3 (centre) |  |  |  |  |  |
| 1010 | 16:9 (centre) |  |  |  |  |  |
| 1011 | 14:9 (centre) |  |  |  |  |  |
| 1100 | Reserved |  |  |  |  |  |
| 1101 | 4:3 (with shoot and protect 14:9 centre) |  |  |  |  |  |
| 1110 | 16:9 (with shoot and protect 14:9 centre) |  |  |  |  |  |
| 1111 | 16:9 (with shoot and protect 4:3 centre) |  |  |  |  |  |
| Active_format |  | Illustration of described | format |  |  |  |
| Value | Description | In 4:3 coded frame | In 16:9 coded frame |  |  |  |
| 0000 to 0001 | reserved |  |  |  |  |  |
| 0101 to 0111 | reserved |  |  |  |  |  |
| 1100 | reserved |  |  |  |  |  |
|  | 4:3 centre) |  |  |  |  |  |
| Active_format | Illustration of described format |  |  |  |  |  |
| Value Description | In 4:3 coded frame In 16:9 coded frame |  |  |  |  |  |
| Syntax | No. of Bits | Identifier |  |  |  |  |
| bar_data() { |  |  |  |  |  |  |
| top_bar_flag | 1 | bslbf |  |  |  |  |
| bottom_bar_flag | 1 | bslbf |  |  |  |  |
| left_bar_flag | 1 | bslbf |  |  |  |  |
| right_bar_flag | 1 | bslbf |  |  |  |  |
| reserved (set to "1111") | 4 | bslbf |  |  |  |  |
| if (top_bar_flag == "1") { |  |  |  |  |  |  |
| marker_bits (set to "11") | 2 | bslbf |  |  |  |  |
| line_number_end_of_top_bar | 14 | uimsbf |  |  |  |  |
| } |  |  |  |  |  |  |
| if (bottom_bar_flag == "1") { |  |  |  |  |  |  |
| marker_bits (set to "11") | 2 | bslbf |  |  |  |  |
| line_number_start_of_bottom_bar | 14 | uimsbf |  |  |  |  |
| } |  |  |  |  |  |  |
| if (left_bar_flag == "1") { |  |  |  |  |  |  |
| marker_bits (set to "11") | 2 | bslbf |  |  |  |  |
| pixel_number_end_of_left_bar | 14 | uimsbf |  |  |  |  |
| } |  |  |  |  |  |  |
| if (right_bar_flag == "1") { |  |  |  |  |  |  |
| marker_bits (set to "11") | 2 | bslbf |  |  |  |  |
| pixel_number_start_of_right_bar | 14 | uimsbf |  |  |  |  |
| } |  |  |  |  |  |  |
| } |  |  |  |  |  |  |
| Video Format | Applicable Standard |  |  |  |  |  |
| 480 Interlaced 4:3 | SMPTE ST 125 [i.8] |  |  |  |  |  |
| 480 Interlaced 16:9 | SMPTE ST 267 [i.10] |  |  |  |  |  |
| 480 Progressive | SMPTE ST 293 [i.12] |  |  |  |  |  |
| 720 Progressive | SMPTE ST 296 [i.13] |  |  |  |  |  |
| 1 080 Interlaced | SMPTE ST 274 [i.11] |  |  |  |  |  |
| 1 080 Progressive | SMPTE ST 274 [i.11] |  |  |  |  |  |
| Syntax | No. of Bits | Identifier |  |  |  |  |
| cc_data() { |  |  |  |  |  |  |
| reserved (set to "1") | 1 | bslbf |  |  |  |  |
| process_cc_data_flag | 1 | bslbf |  |  |  |  |
| zero_bit (set to "0") | 1 | bslbf |  |  |  |  |
| cc_count | 5 | uimsbf |  |  |  |  |
| reserved (set to "1111 1111") | 8 | bslbf |  |  |  |  |
| for ( i=0 ; i < cc_count ; i++ ) { |  |  |  |  |  |  |
| one_bit (set to "1") | 1 |  |  |  |  |  |
| reserved (set to "1111") | 4 |  |  |  |  |  |
| cc_valid | 1 | bslbf |  |  |  |  |
| cc_type | 2 | bslbf |  |  |  |  |
| cc_data_1 | 8 | bslbf |  |  |  |  |
| cc_data_2 | 8 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| marker_bits = "11111111" | 8 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| Syntax | No. of Bits | Identifier |  |  |  |  |
| user_data() { |  |  |  |  |  |  |
| user_data_start_code | 32 | bslbf |  |  |  |  |
| user_identifier | 32 | bslbf |  |  |  |  |
| user_structure() |  |  |  |  |  |  |
| } |  |  |  |  |  |  |
| user_data_registered_itu_t_t35(payloadSize) { | Descriptor | Notes |  |  |  |  |
| itu_t_t35_country_code | b(8) | 0xB5 |  |  |  |  |
| Itu_t_t35_provider_code | u(16) | 0x0031 |  |  |  |  |
| user_identifier | f(32) |  |  |  |  |  |
| user_structure() |  |  |  |  |  |  |
| } |  |  |  |  |  |  |
| Syntax | No. of Bits | Identifier |  |  |  |  |
| user_data() { |  |  |  |  |  |  |
| VC1_user_data_start_code | 32 | bslbf |  |  |  |  |
| user_identifier | 32 | bslbf |  |  |  |  |
| user_structure() |  |  |  |  |  |  |
| } |  |  |  |  |  |  |
| Sequence | Active Format |  | WSS |  |  |  |
| Header | Description |  |  |  |  |  |
| Source aspect | Value | Code | Description |  |  |  |
| ratio |  | (Bits 0-3) |  |  |  |  |
|  | 1001 | 0001 | full format 4:3 |  |  |  |
|  | 1011 | 1000 | box 14:9 Centre |  |  |  |
|  | 0011 | 0100 | box 14:9 Top |  |  |  |
| 4:3 | 1010 | 1101 | box 16:9 Centre |  |  |  |
|  | 0010 | 0010 | box 16:9 Top |  |  |  |
|  | 0100 | 1011 | box > 16:9 Centre |  |  |  |
|  | 1101 | 0111 | full format 4:3 |  |  |  |
|  |  |  | (shoot and protect 14:9 Centre) |  |  |  |
| 16:9 | 1010 | 1110 | full format 16:9 (anamorphic) |  |  |  |
| Syntax | No. of bits | Identifier |  |  |  |  |
| multi_region_disparity() { |  |  |  |  |  |  |
| multi_region_disparity_length | 8 | uimsbf |  |  |  |  |
| number_of_regions = multi_region_disparity_length -1 |  |  |  |  |  |  |
| max_disparity_in_picture | 8 | tcimsbf |  |  |  |  |
| for (i=0; i<number_of_regions, i++) { |  |  |  |  |  |  |
| min_disparity_in_region_i | 8 | tcimsbf |  |  |  |  |
| } |  |  |  |  |  |  |
| } else if (multi_region_disparity_length == 0) { |  |  |  |  |  |  |
| /* there is no disparity information to deliver */ |  |  |  |  |  |  |
| } else { |  |  |  |  |  |  |
| for (i=0;i<N;i++) { |  |  |  |  |  |  |
| reserved_for_future_use | 8 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| } |  |  |  |  |  |  |
| } |  |  |  |  |  |  |
| Value | Meaning of the value |  |  |  |  |  |
| 0 | no disparity information is to be delivered |  |  |  |  |  |
| 1 | Prohibited |  |  |  |  |  |
| 2 | one minimum_disparity_in_region is coded as representing the minimum value in overall |  |  |  |  |  |
|  | picture (see figure B.3) |  |  |  |  |  |
| 3 | two vertical minimum_disparity_in_regions are coded (see figure B.4) |  |  |  |  |  |
| 4 | three vertical minimum_disparity_in_regions are coded (see figure B.5) |  |  |  |  |  |
| 5 | four minimum_disparity_in_regions are coded (see figure B.6) |  |  |  |  |  |
| 6 to 9 | reserved for future use |  |  |  |  |  |
| 10 | nine minimum_disparity_in_regions are coded (see figure B.7) |  |  |  |  |  |
| 11 to 16 | reserved for future use |  |  |  |  |  |
| 17 | sixteen minimum_disparity_in_regions are coded (see figure B.2) |  |  |  |  |  |
| 18 to 255 | reserved for future use |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| dvd_ancillary_data( ) { |  |  |  |  |  |  |
| dynamic_range_control | 8 | bslbf |  |  |  |  |
| dynamic_range_control_on | 1 | bslbf |  |  |  |  |
| reserved (set to "000 0000b") | 7 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| extended ancillary_data( ) { |  |  |  |  |  |  |
| dvd_ancillary_data | 16 | bslbf |  |  |  |  |
| extended_ancillary_data_sync (set to 0xBC) | 8 | bslbf |  |  |  |  |
| bs_info | 8 | bslbf |  |  |  |  |
| ancillary_data_status | 8 | bslbf |  |  |  |  |
| if(advanced_dynamic_range_control_status == 1) |  |  |  |  |  |  |
| advanced_dynamic_range_control | 24 | bslbf |  |  |  |  |
| if(dialog_normalization_status == 1) |  |  |  |  |  |  |
| dialog_normalization | 8 | bslbf |  |  |  |  |
| if(reproduction_level_status == 1) |  |  |  |  |  |  |
| reproduction_level | 8 | bslbf |  |  |  |  |
| if(downmixing_levels_MPEG2_status == 1) |  |  |  |  |  |  |
| downmixing_levels_MPEG2 | 8 | bslbf |  |  |  |  |
| if(audio_coding_mode_and_compression_status == 1) { |  |  |  |  |  |  |
| audio_coding_mode | 8 | bslbf |  |  |  |  |
| Compression | 8 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| if(coarse_grain_timecode_status == 1) |  |  |  |  |  |  |
| coarse_grain_timecode | 16 | bslbf |  |  |  |  |
| if(fine_grain_timecode_status == 1) |  |  |  |  |  |  |
| fine_grain_timecode | 16 | bslbf |  |  |  |  |
| if(scale_factor_CRC_status == 1) |  |  |  |  |  |  |
| scale_factor_CRC | 16 to 32 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| bs_info( ) { |  |  |  |  |  |  |
| mpeg_audio_type | 2 | bslbf |  |  |  |  |
| dolby_surround_mode | 2 | bslbf |  |  |  |  |
| ancillary_data_bytes | 4 | uimsbf |  |  |  |  |
| } |  |  |  |  |  |  |
| mpeg_audio_type | Description |  |  |  |  |  |
| "00" | Reserved |  |  |  |  |  |
| "01" | Only MPEG1 audio data |  |  |  |  |  |
| "10" | MPEG2 audio data |  |  |  |  |  |
| "11" | Reserved |  |  |  |  |  |
| mpeg_audio_type | Description |  |  |  |  |  |
| "00" | Reserved |  |  |  |  |  |
| "01" | MPEG1 part is not Dolby surround encoded |  |  |  |  |  |
| "10" | MPEG1 part is Dolby surround encoded |  |  |  |  |  |
| "11" | Reserved |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| ancillary_data_status( ) { |  |  |  |  |  |  |
| advanced_dynamic_range_control_status | 1 | bslbf |  |  |  |  |
| dialog_normalization_status | 1 | bslbf |  |  |  |  |
| reproduction_level_status | 1 | bslbf |  |  |  |  |
| downmix_levels_MPEG2_status | 1 | bslbf |  |  |  |  |
| scale_factor_CRC_status | 1 | bslbf |  |  |  |  |
| audio_coding_mode_and_compression status | 1 | bslbf |  |  |  |  |
| coarse_grain_timecode_status | 1 | bslbf |  |  |  |  |
| fine_grain_timecode_status | 1 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| advanced_dynamic_range_control( ) { |  |  |  |  |  |  |
| advanced_drc_part_0 | 8 | bslbf |  |  |  |  |
| advanced_drc_part_1 | 8 | bslbf |  |  |  |  |
| advanced_drc_part_2 | 8 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| dialog_normalization( ) { |  |  |  |  |  |  |
| dialog_normalization_on | 2 | bslbf |  |  |  |  |
| dialog_normalization_value | 6 | uimsbf |  |  |  |  |
| } |  |  |  |  |  |  |
| dialog_normalization_on | Description |  |  |  |  |  |
| "00" | dialog_normalization_value is not valid |  |  |  |  |  |
| "01" | reserved |  |  |  |  |  |
| "10" | dialog_normalization_value is valid |  |  |  |  |  |
| "11" | Reserved |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| reproduction_level ( ) { |  |  |  |  |  |  |
| Surround_reproduction_level | 1 | bslbf |  |  |  |  |
| production_roomtype | 2 | bslbf |  |  |  |  |
| reproduction_level_value | 5 | uimsbf |  |  |  |  |
| } |  |  |  |  |  |  |
| surround_reproduction_level | Description |  |  |  |  |  |
| "0" | The surround channels have the correct level for reproduction |  |  |  |  |  |
| "1" | The surround channels should be attenuated by 3 dB during reproduction |  |  |  |  |  |
| production_roomtype | Description |  |  |  |  |  |
| "00" | not indicated |  |  |  |  |  |
| "01" | large room |  |  |  |  |  |
| "10" | small room |  |  |  |  |  |
| "11" | reserved |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| downmixing_levels_MPEG2 ( ) { |  |  |  |  |  |  |
| center_mix_level_on | 1 | bslbf |  |  |  |  |
| center_mix_level_value | 3 | bslbf |  |  |  |  |
| Surround_mix_level_on | 1 | bslbf |  |  |  |  |
| Surround_mix_level_value | 3 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| mix_level_value | Multiplication factor |  |  |  |  |  |
| "000" | 1,000 (0,0 dB) |  |  |  |  |  |
| "001" | 0,841 (-1,5 dB) |  |  |  |  |  |
| "010" | 0,707 (-3,0 dB) |  |  |  |  |  |
| "011" | 0,596 (-4,5 dB) |  |  |  |  |  |
| "100" | 0,500 (-6,0 dB) |  |  |  |  |  |
| "101" | 0,422 (-7,5 dB) |  |  |  |  |  |
| "110" | 0,355 (-9,0 dB) |  |  |  |  |  |
| "111" | 0,000 (- dB) |  |  |  |  |  |
| Syntax | No. of bits | Mnemonic |  |  |  |  |
| audio_coding_mode ( ) { |  |  |  |  |  |  |
| MPEG2_extension_stream_present | 1 | bslbf |  |  |  |  |
| MPEG2_center | 2 | bslbf |  |  |  |  |
| MPEG2_surround | 2 | bslbf |  |  |  |  |
| MPEG2_lfeon | 1 | bslbf |  |  |  |  |
| MPEG2_copyright_ident_present | 1 | bslbf |  |  |  |  |
| compression_on | 1 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| coarse_grain_timecode ( ) { |  |  |  |  |  |  |
| coarse_grain_timecode_on | 2 | bslbf |  |  |  |  |
| coarse_grain_timecode_value | 14 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| fine_grain_timecode ( ) { |  |  |  |  |  |  |
| fine_grain_timecode_on | 2 | bslbf |  |  |  |  |
| fine_grain_timecode_value | 14 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| announcement_switching_data( ) { |  |  |  |  |  |  |
| announcement_switching_data_sync | 8 | bslbf |  |  |  |  |
| data_field_length | 8 | bslbf |  |  |  |  |
| announcement_switching_flag_field_1 | 16 | bslbf |  |  |  |  |
| announcement_switching_flag_field_2 | 16 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| Bit flag | Description |  |  |  |  |  |
| b (see note) | Emergency alarm |  |  |  |  |  |
| 0 |  |  |  |  |  |  |
| b | Road Traffic flash |  |  |  |  |  |
| 1 |  |  |  |  |  |  |
| b | Public Transport flash |  |  |  |  |  |
| 2 |  |  |  |  |  |  |
| b | Warning message |  |  |  |  |  |
| 3 |  |  |  |  |  |  |
| b | News flash |  |  |  |  |  |
| 4 |  |  |  |  |  |  |
| b | Weather flash |  |  |  |  |  |
| 5 |  |  |  |  |  |  |
| b | Event announcement |  |  |  |  |  |
| 6 |  |  |  |  |  |  |
| b | Personal call |  |  |  |  |  |
| 7 |  |  |  |  |  |  |
| b to b | Reserved for future use |  |  |  |  |  |
| 8 15 |  |  |  |  |  |  |
| NOTE: This | bit is transmitted last. |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| scale_factor_error_check_data( ) { |  |  |  |  |  |  |
| scale_factor_error_check data_sync | 8 | bslbf |  |  |  |  |
| data_field_length | 8 | bslbf |  |  |  |  |
| scale factor CRC | 32 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| UECP_data( ) { |  |  |  |  |  |  |
| UECP_data_sync | 8 | bslbf |  |  |  |  |
| data_field_length | 8 | bslbf |  |  |  |  |
| for (i=0; i<N; i++){ |  |  |  |  |  |  |
| UECP_data_byte | 8 | uimsbf |  |  |  |  |
| } |  |  |  |  |  |  |
| } |  |  |  |  |  |  |
| Site Address | Terminal Address | DVB consumer receiver |  |  |  |  |
| 0 | 0 | All |  |  |  |  |
|  | 0 | Stereo |  |  |  |  |
|  | 1 | Dual Channel, ch. A |  |  |  |  |
| 1008 | 2 | Dual Channel, ch. B |  |  |  |  |
|  | 3 | Single Channel (Mono) |  |  |  |  |
|  | 4 to 63 | Not yet assigned |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| MPEG4 ancillary_data( ) { |  |  |  |  |  |  |
| ancillary_data_sync | 8 | bslbf |  |  |  |  |
| bs_info | 8 | bslbf |  |  |  |  |
| ancillary_data_status | 8 | bslbf |  |  |  |  |
| If (downmixing_levels_MPEG4_status == 1) |  |  |  |  |  |  |
| downmixing_levels_MPEG4 | 8 | bslbf |  |  |  |  |
| If (audio_coding_mode_and_compression_status == 1) { |  |  |  |  |  |  |
| audio_coding_mode | 8 | bslbf |  |  |  |  |
| Compression_value | 8 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| if(coarse_grain_timecode_status == 1) |  |  |  |  |  |  |
| coarse_grain_timecode | 16 | bslbf |  |  |  |  |
| if(fine_grain_timecode_status == 1) |  |  |  |  |  |  |
| fine_grain_timecode | 16 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| bs_info( ) { |  |  |  |  |  |  |
| mpeg_audio_type | 2 | bslbf |  |  |  |  |
| dolby_surround_mode | 2 | bslbf |  |  |  |  |
| drc_presentation_mode | 2 | bslbf |  |  |  |  |
| reserved, set to "00" | 2 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| mpeg_audio_type | Description |  |  |  |  |  |
| "00" | Reserved |  |  |  |  |  |
| "01" | Reserved |  |  |  |  |  |
| "10" | Reserved |  |  |  |  |  |
| "11" | MPEG4 Audio data |  |  |  |  |  |
| dolby_surround_mode | Description |  |  |  |  |  |
| "00" | Dolby surround mode not indicated |  |  |  |  |  |
| "01" | 2-ch audio part is not Dolby surround encoded |  |  |  |  |  |
| "10" | 2-ch audio part is Dolby surround encoded |  |  |  |  |  |
| "11" | Reserved |  |  |  |  |  |
| drc_presentation_mode | Description |  |  |  |  |  |
| "00" | DRC presentation mode not indicated |  |  |  |  |  |
| "01" | DRC presentation mode 1 |  |  |  |  |  |
| "10" | DRC presentation mode 2 |  |  |  |  |  |
| "11" | Reserved |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| ancillary_data_status( ) { |  |  |  |  |  |  |
| Reserved, set to "0" | 1 | bslbf |  |  |  |  |
| Reserved, set to "0" | 1 | bslbf |  |  |  |  |
| Reserved, set to "0" | 1 | bslbf |  |  |  |  |
| downmixing_levels_MPEG4_status | 1 | bslbf |  |  |  |  |
| Reserved, set to "0" | 1 | bslbf |  |  |  |  |
| audio_coding_mode_and_compression status | 1 | bslbf |  |  |  |  |
| coarse_grain_timecode_status | 1 | bslbf |  |  |  |  |
| fine_grain_timecode_status | 1 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| downmixing_levels_MPEG4 ( ) { |  |  |  |  |  |  |
| center_mix_level_on | 1 | bslbf |  |  |  |  |
| center_mix_level_value | 3 | bslbf |  |  |  |  |
| surround_mix_level_on | 1 | bslbf |  |  |  |  |
| surround_mix_level_value | 3 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| mix_level_value | Multiplication factor |  |  |  |  |  |
| "000" | 1,000 (0,0 dB) |  |  |  |  |  |
| "001" | 0,841 (-1,5 dB) |  |  |  |  |  |
| "010" | 0,707 (-3,0 dB) |  |  |  |  |  |
| "011" | 0,596 (-4,5 dB) |  |  |  |  |  |
| "100" | 0,500 (-6,0 dB) |  |  |  |  |  |
| "101" | 0,422 (-7,5 dB) |  |  |  |  |  |
| "110" | 0,355 (-9,0 dB) |  |  |  |  |  |
| "111" | 0,000 (- dB) |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| audio_coding_mode ( ) { |  |  |  |  |  |  |
| reserved, set to "000 0000" | 7 | bslbf |  |  |  |  |
| compression_on | 1 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| Data field | Default value |  |  |  |  |  |
| dolby_surround_mode | "00" |  |  |  |  |  |
| drc_presentation_mode | "00" |  |  |  |  |  |
| center_mix_level_value | "010" |  |  |  |  |  |
| surround_mix_level_value | "010" |  |  |  |  |  |
| compression_on | "0" |  |  |  |  |  |
| compression_value | "0000 0000" |  |  |  |  |  |
| coarse_grain_timecode | "00 00000000000000" |  |  |  |  |  |
| fine_grain_timecode | "00 00000000000000" |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| announcement_switching_data( ) { |  |  |  |  |  |  |
| announcement_switching_data_sync | 8 | bslbf |  |  |  |  |
| data_field_length | 8 | bslbf |  |  |  |  |
| announcement_switching_flag_field_1 | 16 | bslbf |  |  |  |  |
| announcement_switching_flag_field_2 | 16 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
|  |  | Playback corresponding | to a | Playback | corresponding | to a |
|  |  | target level | of -31 dB | target | level of -23 dB |  |
|  | Channels of | 5.1 | 2.0 | 5.1 | 2.0 | 1.0 |
|  | playback system |  |  |  |  |  |
| data_field_tag | Description |  |  |  |  |  |
| 0x00 | Reserved |  |  |  |  |  |
| 0x01 | Announcement switching data field |  |  |  |  |  |
| 0x02 | AU_information data field |  |  |  |  |  |
| 0x03 | PVR_assist_information data field |  |  |  |  |  |
| 0x04 | Void |  |  |  |  |  |
| 0x05 to 0x9F | Reserved for future use |  |  |  |  |  |
| 0xA0 to 0xFF | User defined |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| announcement_switching_data( ) { |  |  |  |  |  |  |
| data_field_tag | 8 | uimsbf |  |  |  |  |
| data_field_length | 8 | uimsbf |  |  |  |  |
| announcement_switching_flag_field | 16 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| AU_information () { |  |  |  |  |  |  |
| data_field_tag | 8 | uimsbf |  |  |  |  |
| data_field_length | 8 | uimsbf |  |  |  |  |
| AU_coding_format | 4 | uimsbf |  |  |  |  |
| AU_coding_type_information | 4 | bslbf |  |  |  |  |
| AU_ref_pic_idc | 2 | uimsbf |  |  |  |  |
| AU_pic_struct | 2 | bslbf |  |  |  |  |
| AU_PTS_present_flag | 1 | bslbf |  |  |  |  |
| AU_profile_info_present_flag | 1 | bslbf |  |  |  |  |
| AU_stream_info_present_flag | 1 | bslbf |  |  |  |  |
| AU_trick_mode_info_present_flag | 1 | bslbf |  |  |  |  |
| if (AU_PTS_present_flag == "1") { |  |  |  |  |  |  |
| AU_PTS_32 | 32 | uimsbf |  |  |  |  |
| } |  |  |  |  |  |  |
| if (AU_stream_info_present_flag == "1") { |  |  |  |  |  |  |
| Reserved | 4 | "0000" |  |  |  |  |
| AU_frame_rate_code | 4 | uismbf |  |  |  |  |
| } |  |  |  |  |  |  |
| if (AU_profile_info_present_flag == "1") { |  |  |  |  |  |  |
| AU_profile | 8 | uismbf |  |  |  |  |
| AU_constraint_set0_flag | 1 | bslbf |  |  |  |  |
| AU_constraint_set1_flag | 1 | bslbf |  |  |  |  |
| AU_constraint_set2_flag | 1 | bslbf |  |  |  |  |
| AU_AVC_compatible_flags | 5 | bslbf |  |  |  |  |
| AU_level | 8 | uismbf |  |  |  |  |
| } |  |  |  |  |  |  |
| if (AU_trick_mode_info_present_flag == "1") { |  |  |  |  |  |  |
| AU_max_I_picture_size | 12 | uismbf |  |  |  |  |
| AU_nominal_I_period | 8 | uismbf |  |  |  |  |
| AU_max_I_period | 8 | uismbf |  |  |  |  |
| Reserved | 4 | "0000" |  |  |  |  |
| } |  |  |  |  |  |  |
| if (data_parsed < data_field_length) { |  |  |  |  |  |  |
| AU_Pulldown_info_present_flag | 1 | bslbf |  |  |  |  |
| AU_reserved_zero | 6 | '000000' |  |  |  |  |
| AU_flags_extension_1 | 1 | bslbf |  |  |  |  |
| if (AU_Pulldown_info_present_flag == '1') { |  |  |  |  |  |  |
| AU_reserved_zero | 4 | '0000' |  |  |  |  |
| AU_Pulldown_info | 4 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| if (AU_flags_extension_1 == '1') { |  |  |  |  |  |  |
| AU_reserved | 8 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| } |  |  |  |  |  |  |
| for(i=0; i<n; i++) { |  |  |  |  |  |  |
| AU_reserved_byte | 8 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| } |  |  |  |  |  |  |
| Value | Stream Type |  |  |  |  |  |
| 0 | Undefined |  |  |  |  |  |
| 1 | Recommendation ITU-T H.262 [2] / ISO/IEC 13818-2 [2] Video or |  |  |  |  |  |
|  | ISO/IEC 11172-1 [8] constrained parameter video stream |  |  |  |  |  |
| 2 | H.264/AVC video stream as defined in Recommendation ITU-T H.264 / |  |  |  |  |  |
|  | ISO/IEC 14496-10 [16] Video |  |  |  |  |  |
| 3 | VC-1 video stream as defined in SMPTE ST 421 [20] |  |  |  |  |  |
| 4 | HEVC video stream as defined in ISO/IEC 23008-2 [35] Video |  |  |  |  |  |
| 5-0xF | Reserved |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| AU_IDR_slice_present_flag | 1 | bslbf |  |  |  |  |
| AU_I_slice_present_flag | 1 | bslbf |  |  |  |  |
| AU_P_slice_present_flag | 1 | bslbf |  |  |  |  |
| AU_B_slice_present_flag | 1 | bslbf |  |  |  |  |
| Value | AU_coding_type_information |  |  |  |  |  |
| 0 | Undefined |  |  |  |  |  |
| 1 | I |  |  |  |  |  |
| 2 | P |  |  |  |  |  |
| 3 | B |  |  |  |  |  |
| 4 to 0xF | Reserved |  |  |  |  |  |
| Syntax | No. of Bits | Mnemonic |  |  |  |  |
| Reserved_0 | 1 | bslbf |  |  |  |  |
| AU_I_slice_present_flag | 1 | bslbf |  |  |  |  |
| AU_P_slice_present_flag | 1 | bslbf |  |  |  |  |
| AU_B_slice_present_flag | 1 | bslbf |  |  |  |  |
| AU_frame_rate_code | Corresponding Frame Rate (Hz) |  |  |  |  |  |
| 0 | Forbidden |  |  |  |  |  |
| 1 | 23,976 |  |  |  |  |  |
| 2 | 24 |  |  |  |  |  |
| 3 | 25 |  |  |  |  |  |
| 4 | 29,97 |  |  |  |  |  |
| 5 | 30 |  |  |  |  |  |
| 6 | 50 |  |  |  |  |  |
| 7 | 59,94 |  |  |  |  |  |
| 8 | 60 |  |  |  |  |  |
| 9 to 0xF | Reserved |  |  |  |  |  |
| AU_pic_struct default | AU_Pulldown_info value |  |  |  |  |  |
| 00 | 0 |  |  |  |  |  |
| 01 | 1 |  |  |  |  |  |
| 10 | 2 |  |  |  |  |  |
|  |  | Number of pictures | H.264/AVC | HEVC VCL |  |  |
| IRD | Frame rate (Hz) |  |  |  |  |  |
|  |  | per second | VCL max size | max size |  |  |
|  | 24 or 24 000 / 1 001 | 24 | 10 Mbits | Not applicable |  |  |
| 25 Hz or 30 Hz H.264/AVC SDTV | 25 | 25 | 10 Mbits | Not applicable |  |  |
|  | 30 or 30 000 / 1 001 | 30 | 10 Mbits | Not applicable |  |  |
|  | 24 or 24 000 / 1 001 | 24 | 25 Mbits | Not applicable |  |  |
|  | 25 | 25 | 25 Mbits | Not applicable |  |  |
| 25 Hz or 30 Hz HDTV | 30 or 30 000 / 1 001 | 30 | 25 Mbits | Not applicable |  |  |
|  | 50 | 50 | 25 Mbits | Not applicable |  |  |
|  | 60 or 60 000 / 1 001 | 60 | 25 Mbits | Not applicable |  |  |
|  | 24 or 24 000 / 1 001 | 24 | 62,5 Mbits | 20 Mbits |  |  |
|  | 25 | 25 | 62,5 Mbits | 20 Mbits |  |  |
| 50 Hz or 60 Hz HDTV | 30 or 30 000 / 1 001 | 30 | 62,5 Mbits | 20 Mbits |  |  |
|  | 50 | 50 | 62,5 Mbits | 20 Mbits |  |  |
|  | 60 or 60 000 / 1 001 | 60 | 62,5 Mbits | 20 Mbits |  |  |
|  | 24 or 24 000 / 1 001 | 24 | Not applicable | 40 Mbits |  |  |
|  | 25 | 25 | Not applicable | 40 Mbits |  |  |
| UHDTV | 30 or 30 000 / 1 001 | 30 | Not applicable | 40 Mbits |  |  |
|  | 50 | 50 | Not applicable | 40 Mbits |  |  |
|  | 60 or 60 000 / 1 001 | 60 | Not applicable | 40 Mbits |  |  |
| Syntax | No. bits | Mnemonic |  |  |  |  |
| PVR_assist_information( ) { |  |  |  |  |  |  |
| data_field_tag | 8 | uimsbf |  |  |  |  |
| data_field_length | 8 | uimsbf |  |  |  |  |
| if (data_field_length > 0) { |  |  |  |  |  |  |
| PVR_assist_tier_pic_num | 3 | uimsbf |  |  |  |  |
| PVR_assist_block_trick_mode_present_flag | 1 | bslbf |  |  |  |  |
| PVR_assist_pic_struct_present_flag | 1 | bslbf |  |  |  |  |
| PVR_assist_tier_next_pic_in_tier_present_flag | 1 | bslbf |  |  |  |  |
| PVR_assist_substream_info_present_flag | 1 | bslbf |  |  |  |  |
| PVR_assist_extension_present_flag | 1 | bslbf |  |  |  |  |
| if (PVR_assist_block_trick_mode_present_flag == "1") { |  |  |  |  |  |  |
| PVR_assist_pause_disable_flag | 1 | bslbf |  |  |  |  |
| PVR_assist_fwd_slow_motion_disable_flag | 1 | bslbf |  |  |  |  |
| PVR_assist_fast_fwd_disable_flag | 1 | bslbf |  |  |  |  |
| PVR_assist_rewind_disable_flag | 1 | bslbf |  |  |  |  |
| PVR_assist_reserved_0 | 4 | "0000" |  |  |  |  |
| } |  |  |  |  |  |  |
| if (PVR_assist_pic_struct_present_flag == "1") { |  |  |  |  |  |  |
| PVR_assist_pic_struct | 4 | uimsbf |  |  |  |  |
| PVR_assist_reserved_0 | 4 | "0000" |  |  |  |  |
| } |  |  |  |  |  |  |
| if (PVR_assist_tier_next_pic_in_tier_present_flag == "1") { |  |  |  |  |  |  |
| PVR_assist_tier_next_pic_in_tier | 7 | uimsbf |  |  |  |  |
| PVR_assist_reserved_0 | 1 | "0" |  |  |  |  |
| } |  |  |  |  |  |  |
| if (PVR_assist_substream_info_present_flag == "1") { |  |  |  |  |  |  |
| for ( i = 0; i < 4; i++) { |  |  |  |  |  |  |
| PVR_assist_substream_flag_i | 1 | bslbf |  |  |  |  |
| } |  |  |  |  |  |  |
| PVR_assist_substream_speed_info_present_flag | 1 | bslbf |  |  |  |  |
| PVR_assist_substream_1x_decodable _flag | 1 | bslbf |  |  |  |  |
| PVR_assist_reserved_0 | 2 | "00" |  |  |  |  |
| if (PVR_assist_substream_speed_info_present_flag == "1") { |  |  |  |  |  |  |
| for ( i = 0; i < 4; i++) { |  |  |  |  |  |  |
| PVR_assist_substream_speed_idx_i | 4 | uimsbf |  |  |  |  |
| } |  |  |  |  |  |  |
| } |  |  |  |  |  |  |
| } |  |  |  |  |  |  |
| if (PVR_assist_extension_present_flag == "1") { |  |  |  |  |  |  |
| PVR_assist_segmentation_info_present_flag | 1 | bslbf |  |  |  |  |
| PVR_assist_tier_m_cumulative_frames_present_flag | 1 | bslbf |  |  |  |  |
| Syntax | No. bits | Mnemonic |  |  |  |  |
| PVR_assist_tier_n_mmco_present_flag | 1 | bslbf |  |  |  |  |
| PVR_assist_temporal_id_info_present_flag | 1 | bslbf |  |  |  |  |
| PVR_assist_reserved_0 | 4 | "0000" |  |  |  |  |
| if (PVR_assist_segmentation_info_present_flag == "1") { |  |  |  |  |  |  |
| PVR_assist_seg_id | 8 | uimsbf |  |  |  |  |
| PVR_assist_prg_id | 16 | uimsbf |  |  |  |  |
| PVR_assist_seg_start_flag | 1 | bslbf |  |  |  |  |
| PVR_assist_seg_end_flag | 1 | bslbf |  |  |  |  |
| PVR_assist_prg_start_flag | 1 | bslbf |  |  |  |  |
| PVR_assist_prg_stop_flag | 1 | bslbf |  |  |  |  |
| PVR_assist_scene_change_flag | 1 | bslbf |  |  |  |  |
| PVR_assist_reserved_0 | 3 | "000" |  |  |  |  |
| } |  |  |  |  |  |  |
| if (PVR_assist_tier_m_cumulative_frames_present_flag == "1") { |  |  |  |  |  |  |
| PVR_assist_tier_m | 3 | uimsbf |  |  |  |  |
| PVR_assist_tier_m_cumulative_frames | 5 | uimsbf |  |  |  |  |
| } |  |  |  |  |  |  |
| if (PVR_assist_tier_n_mmco_present_flag == "1") { |  |  |  |  |  |  |
| PVR_assist_tier_n_mmco | 3 | uimsbf |  |  |  |  |
| PVR_assist_reserved_0 | 5 | "00000" |  |  |  |  |
| } |  |  |  |  |  |  |
| if (PVR_assist_temporal_id_info_present_flag == "1") { |  |  |  |  |  |  |
| PVR_assist_max_temporal_id | 3 | uimsbf |  |  |  |  |
| PVR_assist_reserved_0 | 5 | "00000" |  |  |  |  |
| } |  |  |  |  |  |  |
| } |  |  |  |  |  |  |
| for (i=0; i<n; i++) { |  |  |  |  |  |  |
| PVR_assist_reserved_byte | 8 | uimsbf |  |  |  |  |
| } |  |  |  |  |  |  |
| } |  |  |  |  |  |  |
| } |  |  |  |  |  |  |
| Index | Trick Mode Speed |  |  |  |  |  |
| 0 | No defined sub-stream |  |  |  |  |  |
| 1 | 1,25 |  |  |  |  |  |
| 2 | 1,5 |  |  |  |  |  |
| 3 | 2,0 |  |  |  |  |  |
| 4 | 2,5 |  |  |  |  |  |
| 5 | 3,0 |  |  |  |  |  |
| 6 | 4,0 |  |  |  |  |  |
| 7 | 5,0 |  |  |  |  |  |
| 8 | 6,0 |  |  |  |  |  |
| 9 | 8,0 |  |  |  |  |  |
| 10 | 10,0 |  |  |  |  |  |
| 11 | 12,0 |  |  |  |  |  |
| 12 | 16,0 |  |  |  |  |  |
| 13 | 20,0 |  |  |  |  |  |
| 14 | 24,0 |  |  |  |  |  |
| 15 | 30,0 |  |  |  |  |  |
| Syntax | value | No. of Bits | Identifier |  |  |  |
| AD_descriptor { |  |  |  |  |  |  |
| Reserved | 1111 | 4 | bslbf |  |  |  |
| AD_descriptor_length |  | 4 | bslbf |  |  |  |
| AD_text_tag | 0x4454474144 | 40 | bslbf |  |  |  |
| version_text_tag |  | 8 | bslbf |  |  |  |
| AD_fade_byte | 0xXX | 8 | bslbf |  |  |  |
| AD_pan_byte | 0xYY | 8 | bslbf |  |  |  |
| if (version_text_tag == 0x31) { |  |  |  |  |  |  |
| Reserved | 0xFFFFFF | 24 | bslbf |  |  |  |
| } |  |  |  |  |  |  |
| if (version_text_tag == 0x32) { |  |  |  |  |  |  |
| AD_gain_byte center | 0xUU | 8 | bslbf |  |  |  |
| AD_gain_byte front | 0xVV | 8 | bslbf |  |  |  |
| AD_gain_byte surround | 0xWW | 8 | bslbf |  |  |  |
| } |  |  |  |  |  |  |
| Reserved | 0xFFFFFFFF | 32 | bslbf |  |  |  |
| } |  |  |  |  |  |  |
| MCA |  | MCA |  |  |  |  |
|  |  |  | LF |  |  |  |
| Syntax | Value | No. of Bits | Identifier |  |  |  |
| DE_control_data { |  |  |  |  |  |  |
| Reserved | 1111 | 4 | bslbf |  |  |  |
| DE_control_data_length |  | 4 | uimsbf |  |  |  |
| DE_text_tag | 0x4143414445 | 40 | bslbf |  |  |  |
| DE_version_text_tag |  | 8 | bslbf |  |  |  |
| if (DE_version_text_tag == 0x31) { |  |  |  |  |  |  |
| DE_SAOC-DE_in_band |  | 1 | bslbf |  |  |  |
| DE_mode |  | 4 | uimsbf |  |  |  |
| DE_loudness_compensation_info |  | 1 | bslbf |  |  |  |
| Reserved | 11 | 2 | bslbf |  |  |  |
| DE_max_attenuation_dialogue |  | 8 | uimsbf |  |  |  |
| DE_max_attenuation_background |  | 8 | uimsbf |  |  |  |
| if (DE_loudness_compensation_info == 1) { |  |  |  |  |  |  |
| DE_loudness_diff_dialogue |  | 8 | uimsbf |  |  |  |
| DE_loudness_diff_background |  | 8 | uimsbf |  |  |  |
| } |  |  |  |  |  |  |
| } |  |  |  |  |  |  |
| for (i=0; i<N; i++) { |  |  |  |  |  |  |
| fill_byte | 0xFF | 8 | bslbf |  |  |  |
| } |  |  |  |  |  |  |
| } |  |  |  |  |  |  |
| Syntax | Value |  |  |  |  |  |
| Stereo service with dialogue on left and right channel | 0 |  |  |  |  |  |
| Multichannel service with dialogue on center channel | 1 |  |  |  |  |  |
| Multichannel service with dialogue on the left, center | 2 |  |  |  |  |  |
| and right channels |  |  |  |  |  |  |
| Mono service | 3 |  |  |  |  |  |
| Reserved | 4-15 |  |  |  |  |  |
| Syntax | Value |  |  |  |  |  |
| No loudness compensation info present. | 0 |  |  |  |  |  |
| Loudness compensation info is present. Reference for loudness | 1 |  |  |  |  |  |
| compensation info are the dialogue-only and background-only signals. |  |  |  |  |  |  |
| m input | 0,25 | 0,5 | 1 | 2 | 4 |  |
| G |  |  |  |  |  |  |
| Background attenuation in dB | - | - | 0 | 6 dB | 12 dB |  |
| Dialogue attenuation in dB | 12 dB | 6 dB | 0 | - | - |  |
| NOTE: dB values are rounded | to integer | values | for those | examples. |  |  |
| Percentage of | Maximum Trick Play Speed |  |  |  |  |  |
| Discardable Pictures | Achievable by Dropping Pictures |  |  |  |  |  |
| 16 % (1/6 of the pictures) | 1,2x |  |  |  |  |  |
| 20 % (1/5 of the pictures) | 1,25x |  |  |  |  |  |
| 25 % (1/4 of the pictures) | 1,33x |  |  |  |  |  |
| 33 % (1/3 of the pictures) | 1,5x |  |  |  |  |  |
| 50 % (1/2 of the pictures) | 2x |  |  |  |  |  |
| 66 % (2/3 of the pictures) | 3x |  |  |  |  |  |
| 75 % (3/4 of the pictures) | 4x |  |  |  |  |  |
| IRD Class | Output | Frame rate | Frame compatible |  |  |  |
|  | resolution/Format |  | arrangement type |  |  |  |
| 25 Hz | 720p | 50 Hz | Top-and-Bottom, |  |  |  |
|  |  |  | Side-by-Side |  |  |  |
| 25 Hz | 1 080i | 25 Hz | Side-by-Side |  |  |  |
| 30 Hz | 720p | 59,94/60 Hz | Top-and-Bottom, |  |  |  |
|  |  |  | Side-by-Side |  |  |  |
| 30 Hz | 1 080i | 29,97/30 Hz | Side-by-Side |  |  |  |
| 30 Hz | 1 080p | 23,98/24 Hz | Top-and-Bottom, |  |  |  |
|  |  |  | Side-by-Side |  |  |  |
| IRD Class | Output | Frame rate | Frame compatible |  |  |  |
|  | resolution/Format |  | arrangement type |  |  |  |
| 50 Hz | 1 080p | 50 Hz | Top-and-Bottom |  |  |  |
| 60 Hz | 1 080p | 59,94/60 Hz | Top-and-Bottom |  |  |  |
| 1 920 x 1 080p |  |  |  |  |  |  |
|  | 0 | 0 | 540 | 0 |  |  |
| Top-and-Bottom |  |  |  |  |  |  |
|  |  | English | M&E + D1 |  |  |  |
|  |  | English + AD | M&E + D1 + AD |  |  |  |
|  |  | German | M&E + D2 |  |  |  |
| AP 1 | 5.1 M&E + D1 (EN) + D2 (DE) + AD (EN) + TeamRadio |  |  |  |  |  |
|  |  | M&E + Team Radio | M&E + |  |  |  |
|  |  |  | TeamRadio |  |  |  |
|  |  | M&E Only | M&E |  |  |  |
|  |  | English | M&E + D1 |  |  |  |
|  |  | English + AD | M&E + D1 + AD |  |  |  |
| AP 2 | 7.1.4 M&E + D1 (EN) + D2 (DE) + AD (EN) |  |  |  |  |  |
|  |  | German | M&E + D2 |  |  |  |
|  |  | M&E Only | M&E |  |  |  |
|  |  | English | M&E + D1 |  |  |  |
|  |  | English + AD | M&E + D1 + AD |  |  |  |
| AP 3 | O(15).1 M&E + D1 (EN) + D2 (DE) + AD (EN) |  |  |  |  |  |
|  |  | German | M&E + D2 |  |  |  |
|  |  | M&E Only | M&E |  |  |  |
|  |  | English | M&E + D1 |  |  |  |
|  |  | English + AD | M&E + D1 + AD |  |  |  |
| AP 4 | HOA(6) M&E + D1 (EN) + D2 (DE) + AD (EN) |  |  |  |  |  |
|  |  | German | M&E + D2 |  |  |  |
|  |  | M&E Only | M&E |  |  |  |
| NOTE: Audio | Programme examples 2, 3 and 4 are used only to illustrate | the different immersive | formats |  |  |  |
| supported | by NGA systems. |  |  |  |  |  |
| Name | Codec | Frame | Colorimetry | Resolutions | Frame rates |  |
|  |  | formats |  | (notes 1 and 2) | (notes 1 and 3) |  |
| Player conformance point | Additional player conformance points required |  |  |  |  |  |
| avc_hd_50_level40 |  |  |  |  |  |  |
| avc_hd_60_level40 |  |  |  |  |  |  |
| avc_hd_50 | avc_hd_50_level40 (note 1) |  |  |  |  |  |
| avc_hd_60 | avc_hd_60_level40 (note 1) |  |  |  |  |  |
| hevc_hd_50_8 |  |  |  |  |  |  |
| hevc_hd_60_8 |  |  |  |  |  |  |
| hevc_hd_50_10 | hevc_hd_50_8 (note 1) |  |  |  |  |  |
| hevc_hd_60_10 | hevc_hd_60_8 (note 1) |  |  |  |  |  |
| hevc_uhd | hevc_hd_50_8, hevc_hd_60_8, hevc_hd_50_10 and |  |  |  |  |  |
|  | hevc_hd_60_10 (note 1) |  |  |  |  |  |
| hevc_uhd_hlg10 | hevc_uhd (note 2) |  |  |  |  |  |
| hevc_uhd_pq10 | hevc_uhd (note 2) |  |  |  |  |  |
| hevc_uhd_hfr_hlg10 | hevc_uhd_hlg10 (note 1) |  |  |  |  |  |
| hevc_uhd_hfr_pq10 | hevc_uhd_pq10 (note 1) |  |  |  |  |  |
| hevc_uhd2_hdr | hevc_uhd_hlg10, hevc_uhd_pq10 (note 1) |  |  |  |  |  |
| DASH player conformance point | Related broadcast IRD |  |  |  |  |  |
| avc_hd_50_level40 | 25 Hz H.264/AVC HDTV IRD |  |  |  |  |  |
| avc_hd_60_level40 | 30 Hz H.264/AVC HDTV IRD |  |  |  |  |  |
| avc_hd_50 | 50 Hz H.264/AVC HDTV IRD |  |  |  |  |  |
| avc_hd_60 | 60 Hz H.264/AVC HDTV IRD |  |  |  |  |  |
| hevc_hd_50_8 | 50 Hz HEVC HDTV 8-bit IRD |  |  |  |  |  |
| hevc_hd_60_8 | 60 Hz HEVC HDTV 8-bit IRD |  |  |  |  |  |
| hevc_hd_50_10 | 50 Hz HEVC HDTV 10-bit IRD |  |  |  |  |  |
| hevc_hd_60_10 | 60 Hz HEVC HDTV 10-bit IRD |  |  |  |  |  |
| hevc_uhd | HEVC UHDTV IRD |  |  |  |  |  |
| hevc_uhd_hlg10 | HEVC HDR UHDTV IRD using HLG10 |  |  |  |  |  |
| hevc_uhd_pq10 | HEVC HDR UHDTV IRD using PQ10 |  |  |  |  |  |
| hevc_uhd_hfr_hlg10 | HEVC HDR HFR UHDTV IRD using HLG10 |  |  |  |  |  |
| hevc_uhd_hfr_pq10 | HEVC HDR HFR UHDTV IRD using PQ10 |  |  |  |  |  |
| hevc_uhd2_hdr | HEVC HDR UHDTV2 IRD |  |  |  |  |  |
| Coded picture luminance resolution | aspect_ratio_idc |  |  |  |  |  |
| 720 x 576, 704 x 576, 352 x 288 | 2, 4 |  |  |  |  |  |
| 544 x 576 | 4, 12 |  |  |  |  |  |
| Date | Version | Information about changes |  |  |  |  |
|  |  | Extended HE AAC to include HE AAC v2 |  |  |  |  |
| June 2005 | 1.7.1 |  |  |  |  |  |
|  |  | - MPEG-4 HE AAC and HE AAC v2 audio coding (Annex H) |  |  |  |  |
|  |  | Added optional support for frame-compatible 3D video and other enhancements |  |  |  |  |
| June 2011 | 1.10.1 |  |  |  |  |  |
|  |  | - Added annex H on Frame Compatible Plano-Stereoscopic 3DTV |  |  |  |  |
| Date | Version | Information about changes |  |  |  |  |
|  |  | Document history |  |  |  |  |
| Edition 1 | January 1996 | Publication as ETSI ETR 154 |  |  |  |  |
| Edition 2 | October 1996 | Publication as ETSI ETR 154 |  |  |  |  |
| Edition 3 | September 1997 | Publication as ETSI ETR 154 |  |  |  |  |
| V1.4.1 | July 2000 | Publication as ETSI TR 101 154 |  |  |  |  |
| V1.5.1 | May 2004 | Publication |  |  |  |  |
| V1.6.1 | January 2005 | Publication |  |  |  |  |
| V1.7.1 | June 2005 | Publication |  |  |  |  |
| V1.8.1 | July 2007 | Publication |  |  |  |  |
| V1.9.1 | September 2009 | Publication |  |  |  |  |
| V1.10.1 | June 2011 | Publication |  |  |  |  |
| V1.11.1 | November 2012 | Publication |  |  |  |  |
| V2.1.1 | March 2015 | Publication |  |  |  |  |
| V2.2.1 | June 2015 | Publication |  |  |  |  |
| V2.3.1 | February 2017 | Publication |  |  |  |  |
| V2.4.1 | February 2018 | Publication |  |  |  |  |
| V2.5.1 | January 2019 | Publication |  |  |  |  |
| V2.6.1 | September 2019 | Publication |  |  |  |  |

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

