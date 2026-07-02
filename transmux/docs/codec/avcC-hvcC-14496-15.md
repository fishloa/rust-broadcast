# avcC / hvcC decoder-configuration records — clean reference (#425 follow-up)

**Source situation (transparent):** ISO/IEC 14496-15 has no *freely-downloadable
text-layer* edition — the 2012 (AVC-only) copy is text (grounds `avcC`), but every
2017/2019 (HEVC) copy is image-only scanned. The `hvcC` layout here is triple-grounded:
1. **ISO §8.3.3.1 text**, recovered by **marker OCR** of the scanned 2017 4th edition
   (the syntax block is reproduced verbatim under "hvcC — ISO §8.3.3.1" below);
2. **ffmpeg's reference muxer** (`movenc.c` `mov_write_avcc_tag`/`mov_write_hvcc_tag`);
3. **byte-exact oracle** from real ffmpeg muxes (`fixtures/mp4/h264_high.mp4`,
   `fixtures/mp4/hevc_main.mp4`), which pins the layout empirically.
All three agree field-by-field (#394 value-verified). `avcC` (§5.3.3.1) is text-grounded
on the 2012 edition (`specs/iso_iec_14496-15_2012_avc_format_TEXTLAYER.pdf`).

## avcC — AVCDecoderConfigurationRecord (14496-15 §5.3.3.1)

Oracle (`h264_high.mp4`): `01 64 00 0d ff e1 0019 6764000dacd9… 01 0006 68eb…`

```
aligned(8) class AVCDecoderConfigurationRecord {
    unsigned int(8)  configurationVersion = 1;
    unsigned int(8)  AVCProfileIndication;        // = SPS profile_idc
    unsigned int(8)  profile_compatibility;       // = SPS constraint byte
    unsigned int(8)  AVCLevelIndication;          // = SPS level_idc
    bit(6)           reserved = '111111'b;
    unsigned int(2)  lengthSizeMinusOne;          // 3 → 4-byte NAL length
    bit(3)           reserved = '111'b;
    unsigned int(5)  numOfSequenceParameterSets;
    for (i=0;i<numOfSequenceParameterSets;i++){ unsigned int(16) spsLength; bit(8) sps[spsLength]; }
    unsigned int(8)  numOfPictureParameterSets;
    for (i=0;i<numOfPictureParameterSets;i++){ unsigned int(16) ppsLength; bit(8) pps[ppsLength]; }
    // High-profile (100,110,122,244) trailer, if present:
    //   bit(6) reserved='111111'b; unsigned int(2) chroma_format;
    //   bit(5) reserved; unsigned int(3) bit_depth_luma_minus8;
    //   bit(5) reserved; unsigned int(3) bit_depth_chroma_minus8;
    //   unsigned int(8) numOfSequenceParameterSetExt; { u(16) len; sps_ext[len]; }
}
```

## hvcC — HEVCDecoderConfigurationRecord (14496-15 §8.3.3.1)

Oracle (`hevc_main.mp4`): `01 01 60000000 90000000000000 3c f000 fc fd f8 f8 0000 0f 03 …arrays…`

```
aligned(8) class HEVCDecoderConfigurationRecord {
    unsigned int(8)  configurationVersion = 1;
    unsigned int(2)  general_profile_space;
    unsigned int(1)  general_tier_flag;
    unsigned int(5)  general_profile_idc;
    unsigned int(32) general_profile_compatibility_flags;
    unsigned int(48) general_constraint_indicator_flags;
    unsigned int(8)  general_level_idc;
    bit(4)  reserved='1111'b; unsigned int(12) min_spatial_segmentation_idc;
    bit(6)  reserved='111111'b; unsigned int(2)  parallelismType;
    bit(6)  reserved='111111'b; unsigned int(2)  chromaFormat;
    bit(5)  reserved='11111'b;  unsigned int(3)  bitDepthLumaMinus8;
    bit(5)  reserved='11111'b;  unsigned int(3)  bitDepthChromaMinus8;
    unsigned int(16) avgFrameRate;
    unsigned int(2)  constantFrameRate; unsigned int(3) numTemporalLayers;
    unsigned int(1)  temporalIdNested;  unsigned int(2) lengthSizeMinusOne;
    unsigned int(8)  numOfArrays;
    for (j=0;j<numOfArrays;j++){
        unsigned int(1) array_completeness; bit(1) reserved=0; unsigned int(6) NAL_unit_type;
        unsigned int(16) numNalus;
        for (i=0;i<numNalus;i++){ unsigned int(16) nalUnitLength; bit(8) nalUnit[nalUnitLength]; }
    }
}
```
These types already exist in `transmux::{AVCConfigurationBox, HEVCConfigurationBox}`;
the value-verification gate parses each oracle box and byte-exact round-trips it.

## hvcC — ISO §8.3.3.1 (OCR-verified from the scanned 2017 4th ed)

Recovered verbatim via marker OCR; confirmed identical to the ffmpeg `hvcC` oracle
(`hevc_main.mp4`: `01 01 60000000 900000000000 3c f000 fc fd f8 f8 0000 0f 03 …`).

```
aligned(8) class HEVCDecoderConfigurationRecord {
    unsigned int(8)  configurationVersion = 1;
    unsigned int(2)  general_profile_space;
    unsigned int(1)  general_tier_flag;
    unsigned int(5)  general_profile_idc;
    unsigned int(32) general_profile_compatibility_flags;
    unsigned int(48) general_constraint_indicator_flags;
    unsigned int(8)  general_level_idc;
    bit(4)  reserved = '1111'b;   unsigned int(12) min_spatial_segmentation_idc;
    bit(6)  reserved = '111111'b; unsigned int(2)  parallelismType;
    bit(6)  reserved = '111111'b; unsigned int(2)  chroma_format_idc;
    bit(5)  reserved = '11111'b;  unsigned int(3)  bit_depth_luma_minus8;
    bit(5)  reserved = '11111'b;  unsigned int(3)  bit_depth_chroma_minus8;
    unsigned int(16) avgFrameRate;
    unsigned int(2)  constantFrameRate;
    unsigned int(3)  numTemporalLayers;
    unsigned int(1)  temporalIdNested;
    unsigned int(2)  lengthSizeMinusOne;   // 0/1/3 → 1/2/4-byte NALUnitLength
    unsigned int(8)  numOfArrays;
    for (j=0; j < numOfArrays; j++) {
        unsigned int(1)  array_completeness;
        bit(1) reserved = 0;
        unsigned int(6)  NAL_unit_type;
        unsigned int(16) numNalus;
        for (i=0; i < numNalus; i++) {
            unsigned int(16) nalUnitLength;
            bit(8*nalUnitLength) nalUnit;
        }
    }
}
```
