# VVC / H.266 — SPS config + `vvcC` box for the transmux IR

Sources:
- **ITU-T H.266 (2026-01) = ISO/IEC 23090-3** (VVC codec), vendored at
  `specs/itu_t_h266_vvc.pdf` — SPS + `profile_tier_level` syntax (fields below).
- **`vvcC` box** = ISO/IEC **14496-15:2022 §11**. The paid ISO edition is not
  vendored; the exact `vvcC` byte layout is transcribed from the **FFmpeg
  reference writer** (`libavformat/vvc.c`) and byte-verified against a real
  ffmpeg-muxed box — see **`vvcC-14496-15.md`** (authoritative layout doc).

Carriage:
- **MPEG-2 TS:** `stream_type` **0x33** (VVC, ISO/IEC 13818-1 amendment).
- **ISO-BMFF / mp4:** `vvc1` (parameter sets in `vvcC`) / `vvi1` (in-band) sample
  entry + `vvcC` box.

VVC NAL header (H.266 §7.3.1.2): 2 bytes — `forbidden_zero_bit(1)`
`nuh_reserved_zero_bit(1)` `nuh_layer_id(6)` | `nal_unit_type(5)`
`nuh_temporal_id_plus1(3)`. Key `nal_unit_type`: 14=DCI, **15=VPS, 16=SPS,
17=PPS**, 7=IDR_W_RADL, 8=IDR_N_LP, 9=CRA.

## seq_parameter_set_rbsp() — H.266 §7.3.2.4 (subset for config)

| field | descriptor | note |
|---|---|---|
| `sps_seq_parameter_set_id` | u(4) | |
| `sps_video_parameter_set_id` | u(4) | |
| `sps_max_sublayers_minus1` | u(3) | |
| `sps_chroma_format_idc` | u(2) | 0=mono,1=4:2:0,2=4:2:2,3=4:4:4 |
| `sps_log2_ctu_size_minus5` | u(2) | |
| `sps_ptl_dpb_hrd_params_present_flag` | u(1) | |
| `if (…present) profile_tier_level(1, sps_max_sublayers_minus1)` | | see below |
| `sps_gdr_enabled_flag` | u(1) | |
| `sps_ref_pic_resampling_enabled_flag` | u(1) | |
| … | | |
| `sps_pic_width_max_in_luma_samples` | ue(v) | **coded width** |
| `sps_pic_height_max_in_luma_samples` | ue(v) | **coded height** |
| … | | |
| `sps_bitdepth_minus8` | ue(v) | luma/chroma bit depth − 8 |

## profile_tier_level(profileTierPresentFlag, MaxNumSubLayersMinus1) — H.266 §7.3.3.1

| field | descriptor |
|---|---|
| `general_profile_idc` | u(7) |
| `general_tier_flag` | u(1) |
| `general_level_idc` | u(8) |
| `ptl_frame_only_constraint_flag` | u(1) |
| `ptl_multilayer_enabled_flag` | u(1) |
| `general_constraints_info()` | |
| `ptl_sublayer_level_present_flag[i]` … | |
| `ptl_num_sub_profiles` | u(8) |
| `general_sub_profile_idc[i]` | u(32) each |

## VvcDecoderConfigurationRecord (`vvcC` body — hvcC-analogous, 14496-15:2022 §11.2)

Structure mirrors `hvcC` (see `avcC-hvcC-14496-15.md`):
- a leading config prefix (LengthSizeMinusOne for the NAL length field, ptl_present)
  + a **VvcPTLRecord** carrying the `general_profile_idc` / `general_tier_flag` /
  `general_level_idc` / sub-profiles above, plus `chroma_format_idc`,
  `bit_depth_minus8`, and picture width/height (from the SPS);
- then a **NAL-unit array** section: per array `(array_completeness, NAL_unit_type,
  numNalus, [ (nalUnitLength u16, nalUnit) ])` — carrying the DCI/VPS/SPS/PPS NAL
  units, exactly the `hvcC` array layout.

## Derived config
- **width/height** = `sps_pic_width_max_in_luma_samples` / `sps_pic_height_max_in_luma_samples`.
- **profile/tier/level** from `profile_tier_level()`.
- **rfc6381 codec string**: `vvc1.<general_profile_idc>.<...>.L<general_level_idc>.<constraints>`.
