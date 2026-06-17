# Table 19: Progressive and Interlaced Frame Rates for HEVC Bitstreams and recommended values for signalling

_Source: specs/etsi_ts_101_154_v02.10.01_av_coding_in_dvb.pdf — AVC/HEVC resolution+frame-rate constraint tables (8-12,19-21), AFD (B.11), AD_descriptor (E.1), DASH player conformance (L.1). Curated constraint/descriptor subset of a 357-page spec; full codec-constraint prose not transcribed._


|  Output Frame Rate | Interlaced or Progressive | elemental_duration_in_tc_minus1 [temporal_id_max (note 3)] | vui_time_scale | vui_num_units_in_tick | Allowed pic_struct  |
| --- | --- | --- | --- | --- | --- |
|  24 000/1 001 | P | 0 | 24 000 | 1 001 | 0,7,8  |
|  24 | P | 0 | 24 | 1 | 0,7,8  |
|  25 | P | 0 | 25 | 1 | 0,7,8  |
|  25 | I (encoded as frames) | 0 | 50 | 1 | 3,4,5,6  |
|  25 | I (encoded as fields) | 0 | 50 | 1 | 9,10,11,12  |
|  30 000/1 001 | P | 0 | 30 000 | 1 001 | 0,7,8  |
|  30 000/1 001 | I (encoded as frames) | 0 | 60 000 | 1 001 | 3,4,5,6  |
|  30 000/1 001 | I (encoded as fields) | 0 | 60 000 | 1 001 | 9,10,11,12  |
|  30 | P | 0 | 30 | 1 | 0,7,8  |
|  50 | P | 0 | 50 | 1 | 0,7,8  |
|  60 000/1 001 | P | 0 | 60 000 | 1 001 | 0,7,8  |
|  60 | P | 0 | 60 | 1 | 0,7,8  |

NOTE 2: The interlaced frame rates are only applicable to the luminance resolutions with 1 080 lines.

NOTE 3: Other values of vui_time_scale, vui_num_units_in_tick and elemental_duration_in_tc_minus1[temporal_id_max] may be used, for example where a lower frame rate signal is carried as an HEVC temporal video sub-bitstream of a higher frame rate HEVC bitstream (see clause 5.14.5.5). The note in clause 5.14.5.5.1 explains how to calculate the frame rate using vui_time_scale, vui_num_units_in_tick and elemental_duration_in_tc_minus1[temporal_id_max].

Decoding: 50 Hz HEVC HDTV IRDs shall support decoding and displaying of video with an output frame rate of 25 interlaced or progressive; 50 Hz progressive. Support of other output frame rates is optional.

60 Hz HEVC HDTV IRDs shall support decoding and displaying of video with an output frame rate of 30 000/1 001 interlaced or progressive; 24 000/1 001, 24, 30, 60 000/1 001 or 60 Hz progressive. Support of other output frame rates is optional.

HEVC UHDTV IRDs shall support decoding and displaying of video with the output frame rates supported by 50 Hz HEVC HDTV IRDs and 60 Hz HEVC HDTV IRDs.

The frame rate shall be calculated using the VUI and VUI hrd_parameters() syntax elements vui_time_scale, vui_num_units_in_tick and elemental_duration_in_tc_minus1[temporal_id_max]. The highest TemporalID to be decoded is indicated by temporal_id_max, carried in the HEVC video descriptor.

The frame rates that shall be supported and the recommended values for signalling them are listed in table 19.

NOTE 4: High Dynamic Range and/or High Frame Rates Bitstreams as defined in clauses 5.14.4 and 5.14.5 are not intended to be used with interlaced formats. Therefore, the use of interlaced formats in HEVC UHDTV Bitstreams will complicate any upgrade to HDR and/or HFR.



## 5.14.1.8 Random Access Point

### 5.14.1.8.0 General

**Encoding:**
An HEVC DVB_RAP shall include exactly one Video Parameter Set (that is active), exactly one Sequence Parameter Set (that is active) with VUI, at least one Picture Parameter Set, and optionally a recovery point SEI message which shall be present if the `nal_unit_type` of the HEVC DVB_RAP is equal to TRAIL_R. The VPS, SPS, and PPS that are required for decoding the associated picture shall precede SEI NAL units in this access unit. The recovery point SEI message, when present, shall precede all other SEI NAL units in an HEVC DVB_RAP.

The `nal_unit_type` of each VCL NAL unit of an HEVC DVB_RAP picture shall be equal to one of BLA_W_LP, BLA_W_RADL, BLA_N_LP, IDR_W_RADL, IDR_N_LP, CRA_NUT or TRAIL_R that contains only slices with `slice_type` equal to 2 (I slice) (per Recommendation ITU-T H.265 / ISO/IEC 23008-2 [35]).

If a NAL unit contains a HEVC DVB_RAP, the value of `nuh_temporal_id_plus1` shall be equal to 1.

**NOTE 1:** For progressive content coding, it is recommended that HEVC IRAP pictures (IDR, BLA or CRA) are used.

The time interval between HEVC DVB_RAPs may vary between programs and also within a program. The broadcast requirements should set the time interval between HEVC DVB_RAPs as specified in clause 5.14.1.8.1.

All pictures with PTS greater than or equal to PTS(rap) shall be fully reconstructible and displayable, where PTS(rap) represents the Presentation Time Stamp of the picture of the HEVC DVB_RAP. This means that decoders receiving the HEVC DVB_RAP shall not need to utilize data transmitted prior to the HEVC DVB_RAP to decode pictures displayed after the HEVC DVB_RAP.

To improve applications such as channel change, it is recommended that the Presentation Time Stamp of the picture of HEVC DVB_RAP be less than or equal to [DTS(rap) + 0,67 seconds] where DTS(rap) represents the Decoding Time Stamp of the picture of HEVC DVB_RAP.

Packetization of random access points shall comply with the following additional rule:

A transport packet containing the PES header of a HEVC DVB_RAP shall have an adaptation field. The `payload_unit_start_indicator` bit shall be set to "1" in the transport packet header and the `adaptation_field_control` bits shall be set to "11" (as per Recommendation ITU-T H.222.0 / ISO/IEC 13818-1 [1]). In addition, the `random_access_indicator` bit in the adaptation header shall be set to "1". The `elementary_stream_priority_indicator` bit shall also be set to "1" in the same adaptation header if this transport packet contains the first slice start code of the HEVC DVB_RAP access unit (see clauses 4.1.5.1 and 4.1.5.2).

Both the `random_access_indicator` and `elementary_stream_priority_indicator` bits shall be set to "1" in the adaptation field of a transport packet containing the PES packet header of HEVC DVB_RAP if this transport packet also contains the first slice start code of HEVC DVB_RAP picture. Otherwise a transport packet with the `elementary_stream_priority_indicator` bit set to "1" may follow the transport packet with the `random_access_indicator` bit set to "1".

**Decoding:**
HEVC IRDs shall be able to start decoding and displaying an HEVC Bitstream at an HEVC DVB_RAP.

**NOTE 2:** In the case where `elementary_stream_priority_indicator` and `random_access_indicator` are used to identify the HEVC DVB_RAP, it should be noted that a VPS, SPS, PPS and any SEI may exceed one or more transport packet in length.



## 5.14.1.8.1 Time Interval Between Random Access Points

**Encoding:** The encoder shall place HEVC DVB_RAPs in the video elementary stream at least once every 5 s. It is recommended that HEVC DVB_RAPs occur in the video elementary stream on average at least every 2 s. Where rapid channel change times are important or for applications such as PVR it may be appropriate for HEVC DVB_RAPs to occur more frequently, such as every 1 second. The time interval between successive HEVC DVB_RAPs shall be measured as the difference between their respective DTS values.

**NOTE 1:** Decreasing the time interval between HEVC DVB_RAPs may reduce channel hopping time and improve trick modes, but may reduce the efficiency of the video compression.

**NOTE 2:** Having a regular interval between HEVC DVB_RAPs may improve trick mode performance, but may reduce the efficiency of the video compression.

**NOTE 3:** Due to the nature of video encoding, the HEVC DVB_RAP period may not be exactly aligned to whole seconds.

## 5.14.1.9 Scalability

### 5.14.1.9.0 General

HEVC Temporal sub-layers are components of a single bitstream, analogous to the tiers described in annex D of the present document, and signalled using the **nuh_temporal_id_plus1** in the NAL unit header. That is, each HEVC Temporal sub-layer represents a set of pictures that are only dependent upon pictures of an equivalent or lower numbered sub-layer. HEVC Temporal sub-layers can be beneficially used to assist trick modes.

Extensions might be added in future versions of the present document. It is expected that such extensions would use additional transport stream PIDs to allow such services to be introduced in a backwards compatible manner.

**Decoding:** HEVC IRDs shall skip over data structures which are currently "reserved" (as per Recommendation ITU-T H.265 / ISO/IEC 23008-2 [35]), or which correspond to functions not implemented by the HEVC IRD.

### 5.14.1.9.1 Temporal sub-layers

**Encoding:** HEVC Bitstreams and HEVC temporal video sub-bitstreams shall be carried on a single Transport Stream PID with **stream_type** equal to 0x24. Only HEVC Bitstreams and HEVC temporal video sub-bitstreams:

- obeying the limits associated with level 4.1 for HDTV HEVC Bitstreams shall be carried within this PID;
- obeying the limits associated with level 5.1 for UHDTV HEVC Bitstreams or UHDTV HDR HEVC Bitstreams shall be carried within this PID.

The Decoding Time Stamps of access units in HEVC Bitstreams and HEVC temporal video sub-bitstreams carried within this PID shall be inserted at a constant rate.

All Access Units of HEVC Bitstreams and HEVC temporal video sub-bitstreams carried within this PID shall have values of TemporalId lower than or equal to 4.

**Decoding:** HEVC HDTV IRDs shall decode HEVC Bitstreams with **stream_type** equal to 0x24, obeying the limits associated with level 4.1.

HEVC UHDTV IRDs shall decode HEVC Bitstreams and HEVC temporal video sub-bitstreams with stream_type equal to 0x24, obeying the limits associated with level 5.1.

HEVC IRDs shall decode HEVC Bitstreams and HEVC temporal video sub-bitstreams with **sps_max_sub_layers_minus1** greater than 0.



NOTE: HEVC IRDs are not required to use the temporal substream extraction process described in Recommendation ITU-T H.265 / ISO/IEC 23008-2 [35], clause 10 to decode and display the HEVC Bitstreams in the Transport Stream PID with stream_type equal to 0x24.

## 5.14.1.9.2 Layer Sets

Encoding: HEVC Bitstreams and HEVC video sub-bitstreams shall set the NAL unit header nuh_layer_id equal to 0 for Transport Streams with stream_type equal to 0x24.

Decoding: HEVC IRDs may ignore all NAL units with values of nuh_layer_id not equal to 0.

NOTE 1: It is possible that future versions of the present document may allow non-zero values, but that these streams should still be decodable by HEVC IRDs compliant with the present document.

NOTE 2: It is expected that NAL units with values of nuh_layer_id other than 0 would occur on a different PID (or separate but associate bitstream carriage) and so would not be expected to be seen by a decoder compliant to the present document.

NOTE 3: HEVC Layer sets may be used in future versions of the present document to support for example, high dynamic range, wide colour gamut, 3D or higher spatial resolution extensions.

## 5.14.1.10 HEVC Seamless splicing

Seamless splicing of HEVC video may be accomplished by conforming to the constraints of ANSI/SCTE 172 [i.20], "Constraints on AVC Video Coding for Digital Program Insertion".

NOTE: While ANSI/SCTE 172 is currently drafted with AVC in mind, the constraints are fully applicable to HEVC and DVB expects that SCTE will produce an updated document which explicitly includes HEVC.

## 5.14.2 HEVC HDTV IRDs and Bitstreams

## 5.14.2.0 General

This clause specifies the HEVC HDTV IRDs and Bitstreams. All specifications in clause 5.14.1 shall apply. The specification in the remainder of this clause only applies to the HEVC HDTV IRDs and Bitstreams.

Two HEVC HDTV IRDs are defined in the present document: HEVC HDTV 10-bit IRD and HEVC HDTV 8-bit IRD with the capabilities defined in the definitions.

NOTE: An additional HEVC HDR HDTV Bitstream conformance point is specified in clause 5.14.3, but without any corresponding IRD conformance point.

## 5.14.2.1 Profile, tier and level

Encoding: In addition to the provisions set forth in Recommendation ITU-T H.265 / ISO/IEC 23008-2 [35], the following restrictions shall apply for the fields in the sequence parameter set:

bit_depth_luma_minus8 = 0 or 2
bit_depth_chroma_minus8 = bit_depth_luma_minus8
vui_parameters_present_flag = 1
sps_extension_present_flag = 0

In addition to the provisions set forth in Recommendation ITU-T H.265 / ISO/IEC 23008-2 [35], the following restrictions shall apply for the fields in the profile_tier_level syntax structure in the sequence parameter set:

general_tier_flag = 0
general_profile_idc = 1 (Main profile) or 2 (Main 10 profile)



HEVC HDTV Bitstreams shall obey the limits in Recommendation ITU-T H.265 / ISO/IEC 23008-2 [35], table A.1 and table A.2 associated with Level 4.1.

general_level_idc shall be less than or equal to 123 (level 4.1), unless the HEVC Bitstream is a HEVC temporal video sub-bitstream. In this case, sps_max_sub_layers_minus1 shall be greater than "0", sub_layer_level_present_flag[i] where 'i' is equal to temporal_id_max carried within the HEVC Video Descriptor shall be equal to "1", and sub_layer_level_idc[i] where 'i' is equal to temporal_id_max carried within the HEVC Video Descriptor shall be less than or equal to "123" (level 4.1).

It is recommended that bitstreams which are compliant with the Main profile set general_profile_compatibility_flag[1] to 1.

As specified in annex A of Recommendation ITU-T H.265 / ISO/IEC 23008-2 [35], the value of bit_depth_luma_minus8 and bit_depth_chroma_minus8 shall be equal to 0 when general_profile_idc is equal to 1 or general_profile_compatibility_flag[1] is equal to 1.

NOTE 1: In the Main 10 Profile and the Main Profile, chroma_format_idc is equal to '1'.

Decoding: HEVC HDTV 10-bit IRDs shall support the decoding of HEVC HDTV Bitstreams. HEVC HDTV 8-bit IRDs shall support the decoding of HEVC HDTV Bitstreams within the constraints of the definition.

If temporal extensions are added in future versions of the present document, general_level_idc may be greater than 123 (level 4.1). When sps_max_sub_layers_minus1 is greater than "0", IRDs may ignore general_level_idc and shall make use of the sub_layer_level_idc[i] syntax element, where 'i' is equal to temporal_id_max carried within the HEVC Video Descriptor, to determine whether a bitstream or sub-bitstream can be decoded.

The HEVC HDTV IRD may ignore sequence parameter set extensions signalled by sps_extension_present_flag set to "1".

NOTE 2: HEVC HDTV IRDs are not required to decode and display correctly HEVC Bitstreams or HEVC temporal video sub-bitstreams that do not obey the constraints and limits associated with the Main or Main 10 Profile, Main Tier, Level 4.1.

## 5.14.2.2 Luminance resolution

Encoding: HEVC HDTV encoders shall, as a minimum, represent video with the luminance resolutions shown in table 20, where luminance resolution is to be understood as the video resolution after conformance cropping. Pictures may be down-scaled and encoded at less than full size using the reciprocal of the scaling ratios shown in the table. Additional luminance resolutions may be supported, but they shall be square pixel formats indicated by aspect_ratio_idc equal to "1".

Where non 16:9 sources are re-formatted and encoded within a 16:9 frame, AFD/bar data defined in clause B.3 and default display window defined in clause 5.14.1.5.6 should be included within the bitstream to assist the IRD in displaying the content.

Decoding: HEVC HDTV IRDs shall be capable of decoding pictures with luminance resolutions shown in table 20, where luminance resolution is to be understood as the video resolution after conformance cropping. HEVC IRDs shall be able to reconstruct the image size to allow the decoded pictures to be displayed at full-screen size.
