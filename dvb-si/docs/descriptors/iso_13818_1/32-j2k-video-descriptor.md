## Table 2-101 — J2K video descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.80, Table 2-101; PDF pp.113-114. Transcribed via BlazeDocs (nested-conditional table)._

|  J2K_video_descriptor() { |  |   |
|  descriptor_tag | 8 | uimsbf  |
|  descriptor_length | 8 | uimsbf  |
|  extended_capability_flag | 1 | bslbf  |
|  profile_and_level | 15 | bslbf  |
|  horizontal_size | 32 | uimsbf  |
|  vertical_size | 32 | uimsbf  |
|  max_bit_rate | 32 | uimsbf  |
|  max_buffer_size | 32 | uimsbf  |
|  DEN_frame_rate | 16 | bslbf  |
|  NUM_frame_rate | 16 | bslbf  |
|  if (extended_capability_flag == '1') { |  |   |
|  stripe_flag | 1 | bslbf  |
|  block_flag | 1 | bslbf  |
|  mdm_flag | 1 | bslbf  |
|  reserved (all bits to be set to '0') | 5 | bslbf  |
|  } else { |  |   |
|  color_specification | 8 | bslbf  |
|  } still_mode | 1 | bslbf  |
|  interlaced_video | 1 | bslbf  |
|  reserved | 6 | bslbf  |
|  if (extended_capability_flag == '1') { |  |   |
|  colour_primaries | 8 | uimsbf  |
|  transfer_characteristics | 8 | uimsbf  |
|  matrix_coefficients | 8 | uimsbf  |
|  video_full_range_flag | 1 | bslbf  |
|  reserved | 7 | bslbf  |
|  if (stripe_flag == '1') { |  |   |
|  strp_max_idx | 8 | uimsbf  |
|  strp_height | 16 | uimsbf  |
|  } if (block_flag == '1') { |  |   |
|  full_horizontal_size | 32 | uimsbf  |
|  full_vertical_size | 32 | uimsbf  |
|  blk_width | 16 | uimsbf  |
|  blk_height | 16 | uimsbf  |
|  max_blk_idx_h | 8 | uimsbf  |
|  max_blk_idx_v | 8 | uimsbf  |
|  blk_idx_h | 8 | uimsbf  |
|  blk_idx_v | 8 | uimsbf  |
|  } if (mdm_flag == '1') { |  |   |
|  X_c0, Y_c0, X_c1, Y_c1, X_c2, Y_c2 | 16x6 | uimsbf  |
|  X_wp | 16 | uimsbf  |
|  Y_wp | 16 | uimsbf  |
|  L_max | 32 | uimsbf  |
|  L_min | 32 | uimsbf  |

Rec. ITU-T H.222.0 (06/2021)

ISO/IEC 13818-1:2021

Table 2-101 – J2K video descriptor

|  Syntax | No. of bits | Mnemonic  |
| --- | --- | --- |
|  MaxCLL | 16 | uimsbf  |
|  MaxFALL | 16 | uimsbf  |
|  } |  |   |
|  } |  |   |
|  for (i=0; i<N; i++) { |  |   |
|  private_data_byte | 8 | hslbf  |
|  } |  |   |
|  } |  |   |

## 2.6.81 Semantics of fields in J2K video descriptor

extended_capability_flag – This 1-bit field indicates that the J2K video stream uses extended color specification (through three bytes defining the chromaticity parameters, as described below), and that it might have one or several of the following capabilities enabled: stripes (through the J2K stripe mode), blocks (through the J2K block mode), or inclusion of mastering display metadata. The exact list of enabled capabilities is set through subsequent flags in the video descriptor (see below).

profile_and_level – This 15-bit field shall correspond to the 15 least significant bits of the 2-bytes Rsize value included in all J2K codestream main headers of this J2K video stream. Rsize values that are defined in Table A.10 of Rec. ITU-T T.800 | ISO/IEC 15444-1 and do set to '0' their most significant bit are allowed.

NOTE – the combination of the extend_capability_flag and the profile_and_level field ensures backward and forward compatibility with legacy devices conforming to previous versions of this Recommendation | International Standard. Having the extended_capability_flag set to '1' leads indeed to a 16-bit value outside the range accepted by previous versions of this Recommendation | International Standard. This way, J2K video streams with extended capabilities can be unequivocally identified by both legacy and new devices.

horizontal_size – This 32-bit field indicates the horizontal size of the frame (for progressive) or field (for interlaced) comprised in each J2K access unit. If J2K block mode is enabled, this frame or field corresponds to a spatial rectangular block of the entire video frame or field. It shall be coded the same as the Xsize parameter found in all J2K codestream main headers of this J2K video stream, as defined in Annex A of Rec. ITU-T T.800 | ISO/IEC 15444-1.

vertical_size – This 32-bit field indicates the vertical size of the frame (for progressive) or field (for interlaced) comprised in each J2K access unit. If J2K block mode is enabled, this frame or field corresponds to a spatial rectangular block of the entire video frame or field. If J2K stripe mode is disabled, it shall be coded the same as the Ysize parameter found in all J2K codestream main headers of this J2K video stream. If J2K stripe mode is enabled, it shall be coded as the sum of the Ysize parameters found in all J2K codestreams composing the frame (for progressive) or a field (for interlaced) comprised in each J2K access unit. Ysize parameters are defined in Annex A of Rec. ITU-T T.800 | ISO/IEC 15444-1.

max_bit_rate – This field may be coded the same as the brat_max_br field specified in Table S.1 and shall not exceed the maximum compressed bit rate value for the profile and level specified in Table S.2. This field shall be set appropriately and signalled when profile_and_level = '000 0011 0000 0111', where no maximum bit rate is specified.

max_buffer_size – This field shall not exceed the Maximum buffer size value for the profile and level specified in Table S.2. When profile_and_level = '000 0011 0000 0111', the max_buffer_size shall be set appropriately and shall not exceed (max_bit_rate/1.60E5), where max_bit_rate is expressed in bit/s.

DEN_frame_rate – This field shall be coded the same as frat_denominator field specified in Table S.1 (see Annex S).

NUM_frame_rate – This field shall be coded the same as frat_numerator field specified in Table S.1 (see Annex S).

NOTE – J2K frame rate is derived from the DEN_frame_rate and NUM_frame_rate values. Table 2-102 lists examples of typical broadcast frame rates with associated values of DEN_frame_rate and NUM_frame_rate.

Rec. ITU-T H.222.0 (06/2021)

ISO/IEC 13818-1:2021

Table 2-102 – Example frame rates based on DEN_frame_rate and NUM_frame_rate values

|  DEN_frame_rate | NUM_frame_rate | Frame rate ratio (decimal representation) | Frame rate  |
| --- | --- | --- | --- |
|  0000 0000 0000 0000 |  |  | Forbidden  |
|  0000 0011 1110 1001 | 0101 1101 1100 0000 | 24 000 / 1001 | 23.976  |
|  0000 0000 0000 0001 | 0000 0000 0001 1000 | 24 / 1 | 24.0  |
|  0000 0000 0000 0001 | 0000 0000 0001 1001 | 25 / 1 | 25.0  |
|  0000 0011 1110 1001 | 0111 0101 0011 0000 | 30 000 / 1001 | 29.97  |
|  0000 0000 0000 0001 | 0000 0000 0001 1110 | 30 / 1 | 30.0  |
|  0000 0000 0000 0001 | 0000 0000 0011 0010 | 50 / 1 | 50.0  |
|  0000 0011 1110 1001 | 1110 1010 0110 0000 | 60 000 / 1001 | 59.94  |
|  0000 0000 0000 0001 | 0000 0000 0011 1100 | 60 / 1 | 60.00  |

stripe_flag – This 1-bit field is included only if the extended_capability_flag is set to '1'. It indicates whether the J2K video stream has J2K stripe mode enabled. When this flag is set to '1' the J2K access unit elementary stream header (see Table S.1) shall not include the syntax element j2k_tcod, shall include the syntax element j2k_strp, and the corresponding J2K access unit shall be made of a succession of J2K stripes. When this flag is set to '0', the J2K access unit elementary stream header shall include the syntax element j2k_tcod, shall not include the syntax element j2k_strp, and the corresponding J2K access unit shall be made of one J2K codestream in case of progressive content and two J2K codestreams in case of interlaced content.

block_flag – This 1-bit field is included only if the extended_capability_flag is set to '1'. When set to '1', it indicates that the J2K video stream has J2K block mode enabled, meaning that this J2K video stream actually corresponds to a spatial rectangular block of the full video stream. Subdivision of each frame into rectangular independent blocks is further defined in Section S.3. When set to '0', then the associated J2K video stream shall not have J2K block mode enabled.

mdm_flag – This 1-bit field is included only if the extended_capability_flag is set to '1'. When set to '1', it indicates that the J2K video descriptor contains the characteristics of the Mastering Display Metadata, as described in SMPTE ST 2086:2014 (see below corresponding fields). When set to '0', then the J2K video descriptor shall not contain the characteristics of the Mastering Display Metadata.

color_specification – This 8-bit field is included only if the extended_capability_flag is set to '0' and corresponds to the legacy color specification method. It shall be coded the same as the bcol_colcr 8-bit field of the j2k_bcol box as specified in Table S.1 (see Annex S).

still_mode – This 1-bit field, when set to '1', indicates that the J2K video stream may include J2K still pictures. When set to '0', then the associated J2K video stream shall not contain J2K still pictures.

interlaced_video – This 1-bit field indicates whether the J2K video stream contains interlaced video. When this flag is set to '1' the J2K access unit elementary stream header (see Table S.1) shall include the syntax elements brat_auf2, fiel_box_code, fiel_fic and fiel_fio. When this flag is set to '0', these syntax elements shall not be present in the J2K access unit elementary stream header.

color_primaries, transfer_characteristics, matrix_coefficients, video_full_range_flag – These four fields (three 8-bit fields and one 1-bit field) are included only if the extended_capability_flag is set to '1' and correspond to a color specification method allowing a broader set of color code points than the legacy method (see color_specification field above). These fields shall be coded according to the semantics with the same name defined in Rec. ITU-T H.273 | ISO/IEC 23001-8.

strp_max_idx – This 8-bit field is included only if J2K stripe mode is enabled. It shall be in the range 0x01 .. 0xff and indicates the maximum value of the stripe index. It corresponds to the number of stripes in the block/field/frame, minus one. Value 0x00 is forbidden as a minimum of 2 stripes is required (otherwise J2K stripe mode shall be disabled).

strp_height – This 16-bit field is included only if J2K stripe mode is enabled. It indicates the default vertical size of a stripe. Depending on the vertical_size field value, the last stripe might have a different height, as detailed in S.4.

full_horizontal_size – This 32-bit field is included only if J2K block mode is enabled. It indicates the horizontal size of the entire video frame of this J2K video stream.

full_vertical_size – This 32-bit field is included only if J2K block mode is enabled. It indicates the vertical size of the entire video frame of this J2K video stream.

blk_width – This 16-bit field is included only if J2K block mode is enabled. It indicates the default width of a J2K block. Depending on the full_horizontal_size field value, the last block of a row might have a different width, as detailed in S.3.

Rec. ITU-T H.222.0 (06/2021)
