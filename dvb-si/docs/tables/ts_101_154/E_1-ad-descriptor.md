# Table E.1: AD_descriptor

_Source: specs/etsi_ts_101_154_v02.10.01_av_coding_in_dvb.pdf — AVC/HEVC resolution+frame-rate constraint tables (8-12,19-21), AFD (B.11), AD_descriptor (E.1), DASH player conformance (L.1). Curated constraint/descriptor subset of a 357-page spec; full codec-constraint prose not transcribed._


|  Syntax | value | No. of Bits | Identifier  |
| --- | --- | --- | --- |
|  AD_descriptor { |  |  |   |
|  Reserved | 1111 | 4 | bslbf  |
|  AD_descriptor_length |  | 4 | bslbf  |
|  AD_text_tag | 0x4454474144 | 40 | bslbf  |
|  version_text_tag |  | 8 | bslbf  |
|  AD_fade_byte | 0xXX | 8 | bslbf  |
|  AD_pan_byte | 0xYY | 8 | bslbf  |
|  if (version_text_tag == 0x31) { |  |  |   |
|  Reserved | 0xFFFFFF | 24 | bslbf  |
|  } |  |  |   |
|  if (version_text_tag == 0x32) { |  |  |   |
|  AD_gain_byte_center | 0xUU | 8 | bslbf  |
|  AD_gain_byte_front | 0xVV | 8 | bslbf  |
|  AD_gain_byte surround | 0xWW | 8 | bslbf  |
|  } |  |  |   |
|  Reserved | 0xFFFFFF | 32 | bslbf  |
|  } |  |  |   |

AD_descriptor_length: The number of significant bytes following the length field (i.e. 8 or 11).

AD_text_tag: A string of 5 bytes forming a simple and unambiguous means of distinguishing this from any other PES_private_data. A receiver which fails to recognize this tag should not interpret this audio stream as audio description.

version_text_tag: The AD_text_tag is extended by a single ASCII character version designator (here "1" indicates revision 1). Descriptors with the same AD_text_tag but a higher version number shall be backwards compatible with the present document - the syntax and semantics of the fade and pan fields will be identical but some of the reserved bytes may be used for additional signalling.

AD_fade_byte: Takes values between 0x00 (representing no fade of the main programme sound) and 0xFF (representing a full fade). Over the range 0x00 to 0xFE one lsb represents a step in attenuation of the programme sound of 0,3 dB giving a range of 76,2 dB. The fade value of 0xFF represents no programme sound at all (i.e. mute). The rate of signalling and the expected behaviour of a decoder to changes in fade byte are described below.

AD_pan_byte: Takes values between 0x00 representing a central forward presentation of the audio description and 0xFF, each increment representing a $360/256$ degree step clockwise looking down on the listener (i.e. just over 1,4 degrees, see figure E.2). The rate of signalling and the expected behaviour of a decoder are described below.

AD_gain_byte_center: Represents a signed value in dB. Takes values between 0x7F (representing +76,2 dB boost of the main programme centre) and 0x80 (representing a full fade). Over the range 0x00 to 0x7F one lsb represents a step in boost of the programme centre of 0,6 dB giving a maximum boost of +76,2 dB. Over the range 0x81 to 0x00 one lsb represents a step in attenuation of the programme centre of 0,6 dB giving a maximum attenuation of -76,2 dB. The gain value of 0x80 represents no main centre level at all (i.e. mute). The rate of signalling and the expected behaviour of a decoder to changes in gain byte are described below.

AD_gain_byte_front: As AD_gain_byte_center, applied to left and right front channel.

AD_gain_byte_surround: As AD_gain_byte_center, applied to all surround channels.

The maximum rate of signalling of fade, pan and gain values is determined by the number of audio PES packets per second for that SA stream. For efficiency several Access Units (AUs) of audio are typically encapsulated within one PES packet and the fade and pan values in each AD_descriptor are deemed to apply to each AU encapsulated within, and which commences in, that PES packet. In typical efficient encapsulation fade and pan values are transmitted every $120\mathrm{ms}$ to $200\mathrm{ms}$. This allows the control over the attack and decay of a fade where a particular gap in the narrative permits.

An AD decoder maintains the relative timing between the decoded AD signal and the decoded programme sound signal and between the appropriate fade, pan and gain values and the decoded description signal.



During programmes for which there is no description there is little reason to transmit an SA stream of continual silence; in these cases the bitrate accorded to SA may be reassigned for other purposes. Decoders should therefore be able to respond promptly to the restoration of the SA component at the start of a described programme.

In the case of AD, the streams for programme sound and for AD are distinguished in the PSI by the use of the ISO_639_language descriptor [1]. The audio_type field within the descriptor associated with programme sound is typically assigned the value 0x00 ("undefined") whilst the equivalent descriptor associated with AD has its audio_type field assigned the value 0x03 ("visual impaired commentary"). If a service has AD in several languages the PMT reference to each stream will have the appropriate ISO_639_language_code from Set 2 as defined in ISO 639 [27] and the AD-capable decoder should discriminate between them on the basis of the preferred language chosen in the user settings.

In the case of Clean Audio, the streams for programme sound and for Clean Audio are distinguished in the PSI by the use of the ISO_639_language descriptor [1]. The audio_type field within the ISO_639_language descriptor associated with main programme sound is typically assigned the value 0x00 ("undefined") whilst the equivalent descriptor associated with Clean Audio has its audio_type field assigned the value 0x02 ("hearing impaired").

# E.3 Coding for Audio Description SA services

AD content is voice-only and is conveyed as a mono signal coded in accordance with ISO/IEC 11172-3 [9] or ISO/IEC 14496-3 [17] or ETSI TS 102 366 [12] or ETSI TS 103 190-1 [43]. The coding scheme used for the main audio service determines the coding scheme used for the description service (i.e. they shall use the same coding standard) and the sampling rate shall be the same for both services.

The principles of processing in a SA decoder in the case of AD when main audio is stereo are shown diagrammatically in figure E.1.

![img-0.jpeg](img-0.jpeg)
Figure E.1: Functionality of AD decoder processing

The level by which the main programme sound should be attenuated during a description passage is signalled in PES_private_data within the PES encapsulation of the coded SA component (as specified in Recommendation ITU-T H.222.0 / ISO/IEC 13818-1 [1]).

Encoding: Support for the encoding of AD is optional.

Decoding: Support for the decoding of AD is optional.

The signalled fade value is an unsigned byte value, 0x00 representing 0 dB, each increment representing a nominal 0,3 dB, 0xFE representing approximately -76,2 dB whilst the fade value 0xFF represents completely mute programme sound.

The signalled gain values for centre, front (L/R) and surround of the main programme represent a signed byte value, with 0x00 representing 0 dB, 0x7F representing +76,2 dB boost, 0x81 representing -76,2 dB and 0x80 complete mute. This allows a gain of -76,2 to +76,2 in steps of nominal 0,6 dB.



![img-0.jpeg](img-0.jpeg)
Figure K.1: Example Broadcast Audio Preselections in an Audio Programme (AP 1)

For automatic selection the Preselection information contains language, accessibility and role attributes. Similarly "text labels" can be used for displaying the Preselections availability on the TV screen and allow manual selection.

# K.3 Carriage of NGA

The simplest method for carrying the Audio Programme Components is to carry all components in a single elementary stream (linked to a single PID, i.e. single-stream delivery case). In this case all components are carried in the transport stream together with the signaling information of the available Audio Preselections. This example emphasizes one of the main differences of the NGA systems compared to the legacy systems, one PID can contain much more than one complete audio main. In legacy systems the multi-language functionality can be achieved using supplementary streams ("broadcast-mix" or "receiver-mix"). For NGA systems this is achieved in a much more bitrate efficient way using only one stream (linked to one PID) containing the independent components instead of complete mains.

In some applications the broadcaster might decide to embed some of the Audio Programme Components in individual elementary streams (separate elementary streams with separate PIDs, i.e. multi-stream delivery case). This method is used with non NGA CODECs in the case of Audio Description, and a secondary language. In these use cases, the additional language or the Audio Description are placed on separate PIDs and all streams are multiplexed into the same transport stream for distribution.



# Annex L (normative): Video codec profiles for DVB DASH

## L.1 Introduction

This annex specifies DVB bitstream and decoder requirements for video content delivered using MPEG DASH.

DASH content is offered in the form of a DASH Media Presentation Description (MPD) that can target a range of device capabilities, with MPEG DASH players able to select those parts of the presentation that they can present and ignoring those that they cannot.

This model for interoperability calls for a different structure to the specification of content conformance points and player capabilities than is used for broadcast.

Clause L.2 defines the decoder requirements in terms of "player conformance points" that other specifications may reference. The player conformance points are specified by referencing the broadcast IRDs but with constraints, relaxations and extensions to account for the differing requirements of MPEG DASH delivery compared to use in a broadcast MPEG-2 Transport Stream.

Clause L.3 defines the content requirements, describing how content can be offered that is interoperable with one or more player conformance points and covering constraints on the use of each supported video codec.

This annex focuses on H.264/AVC and HEVC bitstream requirements that are the foundation of interoperability between encoders and decoders. It does not cover how bitstreams should be used within an MPEG DASH media presentation, nor does it cover requirements relating to bitstream switching, ISO BMFF system layer constraints or playback of MPEG DASH media presentations themselves. Those aspects, as they relate to DVB DASH, are covered in the DVB DASH specification ETSI TS 103 285 [i.34]. For these reasons, the decoder requirements specified in this annex are necessary but not sufficient for a decoder to be suitable for playback of DVB DASH content according to ETSI TS 103 285 [i.34] and reference shall also be made to that specification.

## L.2 H.264/AVC and HEVC player conformance points

### L.2.1 Summary of player conformance points

Table L.1 specifies the principal requirements of codec profile, colorimetry, resolution and frame rates that MPEG DASH players shall support for each of the defined player conformance points.
