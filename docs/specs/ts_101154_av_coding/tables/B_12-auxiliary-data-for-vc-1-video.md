# Table B.12: Auxiliary Data for VC-1 video

_Source: specs/etsi_ts_101_154_v02.10.01_av_coding_in_dvb.pdf — AVC/HEVC resolution+frame-rate constraint tables (8-12,19-21), AFD (B.11), AD_descriptor (E.1), DASH player conformance (L.1). Curated constraint/descriptor subset of a 357-page spec; full codec-constraint prose not transcribed._


|  Syntax | No. of Bits | Identifier  |
| --- | --- | --- |
|  user_data() { |  |   |
|  VC1_user_data_start_code | 32 | bslbf  |
|  user_identifier | 32 | bslbf  |
|  user_structure() |  |   |
|  } |  |   |

VC1_user_data_start_code: This 32-bit field shall be set to 0x0000011D to indicate the beginning of a user data structure in the VC-1 elementary stream.

user_identifier: This is a 32 bit code that indicates the contents of the user_structure() as indicated in table B.1.

user_structure(): This is a variable length data structure defined by the value of user_identifier and table B.1.

# B.8a Auxiliary Data and HEVC video

# B.8a.1 Coding

The Auxiliary Data is carried in the data as Supplemental Enhancement Information in HEVC's "User data registered by Recommendation ITU-T T.35 [19] SEI message" syntactic element (see clauses D.2.6 and D.3.6 of Recommendation ITU-T H.265 / ISO/IEC 23008-2 [35]).

Encoding: Support for the encoding of Auxiliary Data is optional.

When the "User data registered by Recommendation ITU-T T.35 [19] SEI message" is present in an HEVC Bitstream, it shall be a prefix SEI message (i.e. nal_unit_type shall be equal to

PREFIX_SEI_NUT).



Decoding: Support for the decoding of Auxiliary Data is optional.

# B.8a.2 Syntax and Semantics

The Auxiliary Data (AFD, bar data, caption data and multi_region_disparity) is carried in the video elementary stream as Supplemental Enhancement Information in HEVC's "User data registered by Recommendation ITU-T T.35 SEI message" syntactic element [19] which shall be the same as for H.264/AVC. See clause B.7.2 for the syntax and semantics.

# B.9 Relationship with Wide Screen Signalling (WSS)

The AFD and bar data provide a super-set of the aspect ratio signalling specified in ETSI EN 300 294 [14]. The mapping of source aspect ratio and active_format to WSS Aspect Ratio is given in table B.13.
