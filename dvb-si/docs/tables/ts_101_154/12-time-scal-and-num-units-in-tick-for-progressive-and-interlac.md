# Table 12: Time_scal and num_units_in_tick for Progressive and Interlace Frame Rates for 25 Hz H.264/AVC HDTV, 50 Hz H.264/AVC HDTV, 25 Hz SVC HDTV, 50 Hz SVC HDTV and 25 Hz MVC Stereo HDTV

_Source: specs/etsi_ts_101_154_v02.10.01_av_coding_in_dvb.pdf — AVC/HEVC resolution+frame-rate constraint tables (8-12,19-21), AFD (B.11), AD_descriptor (E.1), DASH player conformance (L.1). Curated constraint/descriptor subset of a 357-page spec; full codec-constraint prose not transcribed._


|  Frame Rate | Interlaced or Progressive | time_scale | num_units_in_tick  |
| --- | --- | --- | --- |
|  25 | P | 50 | 1  |
|  25 | I | 50 | 1  |
|  50 | P | 100 | 1  |

Decoding: 25 Hz H.264/AVC HDTV IRDs shall support decoding and displaying video with a frame rate of 25 Hz interlaced or progressive, or 50 Hz progressive within the constraints of High Profile at Level 4. Support of other frame rates is optional.

### 5.7.2.3 Backwards Compatibility

Decoding: 25 Hz H.264/AVC HDTV IRDs shall be capable of decoding any bitstream that a 25 Hz H.264/AVC SDTV IRD is required to decode and resulting in the same displayed pictures as the 25 Hz H.264/AVC SDTV IRD, as described in clause 5.6.2.

## 5.7.3 30 Hz H.264/AVC HDTV IRD and Bitstream

### 5.7.3.0 General

This clause specifies the 30 Hz H.264/AVC HDTV IRD and Bitstream. All specifications in clauses 5.5 and 5.7.1 shall apply. The specification in the remainder of this clause only applies to the 30 Hz H.264/AVC HDTV IRD and Bitstream.



NOTE 1: Setting frame_field_info_present_flag to "1" indicates the presence of pic_struct to determine if the picture should be displayed as a frame or one or more fields. Possible values for pic_struct are defined in table D-2 of Recommendation ITU-T H.265 / ISO/IEC 23008-2 [35]. The pic_struct values 1 and 2 are not allowed in bitstreams since these values do not carry field relationship information which may be needed by the IRD to avoid field parity loss in presence of transmission errors. This implies that non-paired fields are to be avoided in HEVC Bitstreams, and that HEVC IRD may not be able to display correctly HEVC Bitstreams containing non-paired fields.

NOTE 2: In the context of HEVC, paired fields are two fields that are in consecutive access units in decoding order as two coded fields of opposite parity of the same frame, regardless their display order.

Decoding: HEVC IRDs shall support all values defined in pic_struct including all modes requiring field and frame repetition except pic_struct values 1 and 2. The HEVC IRDs need not make use of any other syntax elements (except pic_struct) in the picture timing SEI message, if these elements are present.

An IRD may utilize the values of duplicate_flag to identify and extract for display the original lower programme frame rate out of a received and decoded higher frame rate.

## 5.14.1.6.2 Recovery Point SEI Message

Encoding: The recovery point SEI message shall not be present in access units that do not contain an HEVC DVB_RAP. When present, the recovery_poc_cnt shall be set to 0, exact_match_flag shall be set to 1, and broken_link_flag shall be set to 0.

## 5.14.1.7 Frame rate

Encoding: The frame rate for progressive material shall be 24 000/1 001, 24, 25, 30 000/1 001, 30, 50, 60 000/1 001 or 60 Hz for all allowed luminance resolutions.

The frame rates for interlaced material shall be 25 and 30 000/1 001 Hz.

These two frame rates for interlaced material are only applicable to luminance resolutions with 1 080 lines.

The Decoding Time Stamps of access units in HEVC Bitstreams shall be inserted at a constant rate, such as every 1/50 s for 50 Hz HEVC Bitstreams or every 1/60 s for 60 Hz HEVC Bitstreams.

NOTE 1: 24 and 24 000/1 001 Hz content is carried within a 60 and 60 000/1 001 Hz bitstream respectively, using 3:2 pull-down (pic_struct values 7 and 8) - see clause 5.14.5.5.2. In which case the HEVC temporal video subset is not present and the DTS interval will be at multiples of 60 and 60 000/1 001 Hz.

The frame rate shall be indicated in the VUI by setting vui_time_scale, vui_num_units_in_tick syntax elements and, if HEVC Temporal sub-layers are present, by setting elemental_duration_in_tc_minus1[temporal_id_max] in the hrd_parameters(), where temporal_id_max is signalled in the HEVC video descriptor (as per clause 4.1.8.19a).

Table 19 lists the frame rates that shall be supported and the recommended values for signalling them.
