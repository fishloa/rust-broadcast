---
source: DASH-IF Interoperability Points v4.3 (15 Nov 2018), private/specs/dashif_iop_v4.3_guidelines.pdf
clauses: "10 DASH-IF UHD Extensions (10.1-10.4)"
pdf_pages: "209-225"
printed_pages: "188-204"
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch10-uhd-extensions.md
verified_against_render: true
notes_original_wording: true
---

# IOP v4.3 clause 10 - UHD extensions (notes)

Own-words notes; exact text in the private file. "p." = PDF page. SEI payload syntax (Dolby DM_data()/CM_data()) is in annexes E/F and is not covered.

## 10.2 HEVC 4k (pp.210-215)

- Identifier: `http://dashif.org/guidelines/dash-if-uhd#hevc-4k` (spelled `dashif-uhd#hevc-4k` once in 10.2.3.1, p.212-213). Builds on IOP Main and 6.2.3; adds up to 4k at 60 fps, codec restricted to HEVC Main10 Level 5.1.
- Picture format limits: max 3840x2160, max 60000/1000 fps, progressive source, YCbCr 4:2:0, 8 or 10 bit, BT.709 primaries only. Within an AdaptationSet Representations may differ only in bitrate, spatial resolution, frame rate. `aspect_ratio_idc`, `cpb_cnt_minus1`, `bit_rate_scale`, `bit_rate_value_minus1`, `cpb_size_scale`, `cpb_size_value_minus1` must not change in a track; `pic_width_in_luma_samples` / `pic_height_in_luma_samples` should not change in a stream (clients should handle resolution switching, tested with DASH-IF vectors).
- Bitstream: Main10 Main tier L5.1 (`general_level_idc` <= 153, `general_tier_flag` 0, `general_profile_idc` 2); `vui_parameters_present_flag` 1, `sps_extension_flag` 0, `fixed_pic_rate_general_flag` 1, `general_interlaced_source_flag` 0; square pixels (`aspect_ratio_idc` 1); Main/Main10-compliant streams are recommended to set `general_profile_compatibility_flag[1]`; BT.709 signalled as colour_primaries = transfer_characteristics = matrix_coeffs = 1; SEI per H.265 Annex D allowed. Receivers: decode and display such bitstreams, no further processing (SEI out of scope).
- HDR dynamic metadata (10.2.2.4): signalled by a Supplemental descriptor with scheme `http://dashif.org/metadata/hdr`; value `TS103433` (only scheme here) means SL-HDR Information SEI (ETSI TS 103 433-1 A.2.2) with every SAP of type 1 or 2, and optionally Mastering Display Colour Volume SEI (also at every type 1/2 SAP, used per A.3).
- DASH mapping: the `@profiles` identifier may be set on the AdaptationSet when elementary stream, MPD and file-format rules hold. `@codecs` must not exceed and should be `hvc1.2.4.L153.B0` or `hev1.2.4.L153.B0`. File format: `mvhd`, `tkhd`, `mdhd` durations 0; `tkhd` width/height 16.16 on a square-pixel grid; `vmhd` version 0, graphicsmode 0, opcolor 0,0,0; `stsd` visual sample entry with VPS, maximum cropped width/height, decoder configuration record; `stsc`/`stco` entry_count 0, `stsz`/`stz2` sizes and counts 0 (sizes live in `trun`; moov has no samples); any `sidx` has timescale equal to `mdhd` and reference_ID = track_ID; for `hev1` the `first_sample_flags` states sync (non-sync flag 0/1 accordingly, depends_on 2 for an I frame); Colour Information Box should be present and give the transfer characteristics; sample timing obeys the frame rates.
- AdaptationSet constraints (same as 3.2.10): `@codecs` on the set = maximum capability; `@profiles` optional; `@maxWidth`/`@maxHeight` required (source format, may exceed coded sizes); `@width`/`@height` per Representation matching the sample description maximum; `@minWidth`/`@minHeight` should be absent; `@maxFrameRate` optional; `@frameRate` per Representation. Extras: colour space may be signalled with Essential/Supplemental `urn:mpeg:mpegB:cicp:MatrixCoefficients` (ISO/IEC 23001-8 Table 4), primaries and transfer with `urn:mpeg:mpegB:cicp:ColourPrimaries` / `TransferCharacteristics`; values must match VUI and the signalling is at AdaptationSet level.
- Compatibility: aims to match DVB-DASH ETSI TS 103 285 media profile and 3GPP TS 26.116 5.6 HEVC UHD operation point, but BT.709 only (no BT.2020).

## 10.3 HEVC HDR PQ10 (pp.216-220)

- Identifier: `http://dashif.org/guidelines/dash-if-uhd#hevc-hdr-pq10`. Same as 4k except BT.2020 (non-constant luminance) primaries and SMPTE ST 2084 transfer: colour_primaries 9, transfer_characteristics 16, matrix_coeffs 9; Main10 Main tier L5.1; equals BT.2100 PQ, 10-bit narrow range. Does not require 60 fps or 4k; one cannot make one asset conformant to both 4k and PQ10 but both can be offered as two AdaptationSets in one MPD. Receivers decode and display, no extra processing.
- Dynamic metadata schemes under scheme `http://dashif.org/metadata/hdr`:

| `@value` | Requirement |
|---|---|
| `SMPTE2094-10` | DM_data() in an ITU-T T.35 SEI message; exactly one such SEI per access unit (the text says "ST2094-40" in that sentence, an apparent slip, p.218) |
| `SMPTE2094-40` | ST 2094-40 SEI (CTA-861-G) via T.35; exactly one per SAP of type 1 or 2 |
| `TS103433` | as 10.2.2.4.2 |

- DASH mapping: same `@codecs` limit (`hvc1.2.4.L153.B0` / `hev1.2.4.L153.B0`), file-format rules of 10.2.3.3, AdaptationSet rules of 10.2.3.4. Expected interoperable with DECE CFF v2.2 HDR10 (more constraints there) and the UHD Forum Phase A PQ10 package.

## 10.4 Dual-stream (Dolby Vision) (pp.220-225)

- Identifier `http://dashif.org/guidelines/dash-if-uhd#dvduallayer`, set on the Enhancement Layer. Two interlocked streams: Base Layer (BL, conforms fully to IOP Main, UHD 4k or UHD HDR10 and plays alone) and Enhancement Layer (EL), combined by an ETSI CCM operation into a 12-bit BT.2020/ST 2084 HDR output (Fig. 26: MPD -> BL decoder + EL decoder -> combination -> display).
- EL rules: HEVC Main10 Main tier up to L5.1; same frame rate, fragment durations and presentation times as BL; IDR in BL implies IDR in EL at the same time; EL DPB supports at least as many frames as the BL DPB; BL and EL tracks are paired 1:1. SPS: `bit_depth_luma_minus8` 2, `aspect_ratio_idc` 1, `general_interlaced_source_flag` 0.
- Required EL SEI: composing-metadata SEI (CM_data()), display-management SEI (DM_data()), mastering display colour volume SEI (valid primaries, white point, max/min luminance). Carried in T.35 SEI (prefix SEI NAL type 39, payloadType 4): country code 0xB5, provider code 0x0031, user identifier 0x47413934 ("GA94"), `user_data_type_code` 0x08 = CM_data(), 0x09 = DM_data(), 0x00-0x07 and 0x0A-0xFF reserved (Tables 31-32). CM_data per access unit with ETSI profile 1 (`ccm_profile` 1, `ccm_level` 0, `BL_bit_depth_minus8` 2, `EL_bit_depth_minus8` 2, `hdr_bit_depth_minus8` 2 or 4); DM_data per access unit with `app_identifier` 1, `app_version` 1, exactly one block of level 1, at most 16 of level 2, 0 or 1 of level 5.
- MPD: conforms to IOP HEVC Main plus the above. The EL Representation uses `@dependencyId` pointing at the BL Representation's `@id`; `@codecs` are per layer (BL e.g. `hvc1.1.0.L120.00`, EL e.g. `dvhe.dtr.uhd30`; syntax in annex D). Clients play BL alone or BL+EL. Sample MPD: annex G (text calls it "Annex E"). The printed text has a highlighted placeholder "[TBD Reference on integration, 12]" in 10.4.2.1 (p.222).
