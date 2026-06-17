# Table B.11: Active Format Description for H264/AVC video

_Source: specs/etsi_ts_101_154_v02.10.01_av_coding_in_dvb.pdf — AVC/HEVC resolution+frame-rate constraint tables (8-12,19-21), AFD (B.11), AD_descriptor (E.1), DASH player conformance (L.1). Curated constraint/descriptor subset of a 357-page spec; full codec-constraint prose not transcribed._


|  user_dataregistereditu_t_t35(payloadSize) { | Descriptor | Notes  |
| --- | --- | --- |
|  itu_t_t35_country_code | b(8) | 0xB5  |
|  Itu_t_t35provider_code | u(16) | 0x0031  |
|  user_identifier | f(32) |   |
|  user_structure() |  |   |
|  } |  |   |

itu_t_t35_country_code: This 8 bit field shall have the value 0xB5.

itu_t_t35provider_code: This 16 bit field shall have the value 0x0031.

user_identifier: This is a 32 bit code that indicates the contents of the user_structure() as indicated in table B.1.

NOTE: In MPEG-2, the only discriminator within user_data is this 32-bit value. In the context of H.264/AVC, the value of user_identifier is used in addition to country and provider codes to definitively identify this as Auxiliary Data.

user_structure(): This is a variable length data structure defined by the value of user_identifier and table B.1.

# B.7.3 Auxiliary Data in MVC Stereo HDTV Bitstreams

When present in MVC Stereo HDTV Bitstreams, the active format descriptor, bar data and closed caption data shall be the same for both base and dependent view bitstreams and may be transmitted in the MVC Stereo Base view bitstream.

When present in MVC Stereo HDTV Bitstreams, the multi_region_disparity() data shall be sent in the user_dataregistereditu_t_t35() SEI message, which is contained in MVC scalable nesting SEI message of every MVC Stereo Dependent view component. When present in MVC Stereo HDTV Bitstreams, the multi_region_disparity data shall be present for every MVC Stereo Dependent view component.



# B.8 Auxiliary Data and VC-1 video

# B.8.1 Coding

The Auxiliary Data is carried in the user data of the video elementary stream as defined in SMPTE ST 421 [20]. After each sequence start (and repeat sequence start) the default aspect ratio of the area of interest is that signalled by the sequence header and sequence display extension parameters. When present, after introduction, an AFD or bar data persists until the next sequence start or until another AFD or different bar data is introduced.

Encoding: Support for the encoding of Auxiliary Data is optional.

The Auxiliary Data may be inserted in the video elementary stream as sequence level, entry-point level or frame level user data as specified in SMPTE ST 421 [20]. For example, it could be inserted once per sequence, once per entry-point, or once per frame. It may be changed for each frame. Caption data, when present, shall be inserted once per frame.

After introduction, such an AFD remains in effect until the next sequence start or until a new AFD is introduced.

Decoding: Support for the decoding of Auxiliary Data is optional.

A decoder that supports the decoding of Auxiliary Data shall be capable of decoding it from the sequence level, entry-point level and frame level locations specified in SMPTE ST 421 [20].

# B.8.2 Syntax and Semantics

The Auxiliary Data is carried in the user data of the video elementary stream as defined in SMPTE ST 421 [20]. The syntax is illustrated in table B.12.
