# Table B.13: Support for WSS

_Source: specs/etsi_ts_101_154_v02.10.01_av_coding_in_dvb.pdf — AVC/HEVC resolution+frame-rate constraint tables (8-12,19-21), AFD (B.11), AD_descriptor (E.1), DASH player conformance (L.1). Curated constraint/descriptor subset of a 357-page spec; full codec-constraint prose not transcribed._


|  Sequence Header | Active Format Description | WSS  |   |
| --- | --- | --- | --- |
|  Source aspect ratio | Value | Code (Bits 0-3) | Description  |
|   | 1001 | 0001 | full format 4:3  |
|   | 1011 | 1000 | box 14:9 Centre  |
|   | 0011 | 0100 | box 14:9 Top  |
|  4:3 | 1010 | 1101 | box 16:9 Centre  |
|   | 0010 | 0010 | box 16:9 Top  |
|   | 0100 | 1011 | box > 16:9 Centre  |
|   | 1101 | 0111 | full format 4:3 (shoot and protect 14:9 Centre)  |
|  16:9 | 1010 | 1110 | full format 16:9 (anamorphic)  |

As all-digital systems are constructed, there may remain legacy (or even regulatory) requirements to provide WSS support at some IRD outputs. It is recommended that transmission systems make use of SMPTE ST 2016-1:2009 [23] for signalling AFD and bar data in the incoming video, and that IRDs provide support for this on digital outputs.

Encoding: Incoming aspect ratio signalling (whether originating via WSS or AFD) should be placed in the video elementary stream per the present document. If desired, the encoder may also carry equivalent WSS data per ETSI EN 300 294 [14] in a separate PID.

Decoding: IRDs shall pass AFD and bar data values to their digital video outputs. Such values may be translated, per table B.13 into analogue WSS waveforms for appropriate placement on analogue outputs.

# B.10 Aspect Ratio Ranges

The labels 4:3, 14:9, 16:9 and  $&gt;16:9$  used in the AFD shall correspond to the aspect ratio ranges specified in ETSI EN 300 294 [14] (note that the corresponding active lines specified in ETSI EN 300 294 [14] do not, in general, apply).

# B.11 Multi Region Disparity

# B.11.0 Introduction

This clause describes how to convey depth information in the form of disparity values so as to enable the overlay of additional information (graphics, menus, etc.) such that a depth violation between the plano-stereoscopic video and graphics is avoided.


# Annex E (normative): Supplementary Audio Services

## Overview

Supplementary Audio (SA) services provide an additional audio soundtrack that provides an additional feature or function over and above that provided by the main audio stream. The SA stream may be provided using one of two schemes:

- "Broadcast mix": pre-mixed by the broadcaster and offered as an alternative audio stream.
- "Receiver mixed": mixed in the receiver under the control of signalling provided by the broadcaster plus some limited control of the user.

This annex only deals with receiver-mixed SA services. Further information on the DVB-SI signalling for receiver-mixed SA services can be found in clause 2 of ETSI EN 300 468 [.32].

Examples of SA services include audio description for the visually impaired, audio for the hearing impaired ("Clean Audio") and a director's commentary. The language used in this annex is mainly in terms of an audio description service although it is equally applicable to all SA applications.

Audio Description (AD) delivers a description of the scene. It is intended to aid understanding and enjoyment particularly, but not exclusively, for viewers who have visual impairments.

Clean Audio refers to audio providing improved intelligibility. It is targeted for viewers with hearing impairments, but can as well serve as improvement for listening in noisy environments like airplanes.

Loud sound effects or music could make the added supplementary audio hard to discern so an important requirement is to adjust, on a passage-by-passage basis, the relative level of programme sound in the mix which the SA user hears. The programme maker is best able to determine the level under controlled conditions when authoring the SA information to modulate the level of programme sound in the SA-capable receiver so suitable SA information is thus transmitted within the SA stream.

Individual SA users will have different aural acuity, describers (of AD) will have different styles of delivery (voice pitch and timbre), several voices may be used to describe one programme and there are, in practice, differences in audio signal level for different home receivers. An essential requirement is for the user to be able to adjust the volume of the SA signal to suit his/her condition.

The ability to optionally mix one or more supplementary additional audio channels with the main programme sound can have other applications, including multi-language commentaries, use for interactivity, and educational purposes.

## Syntax and semantics

SA control information is coded in PES_private_data within the PES encapsulation of the coded SA component in accordance with Recommendation ITU-T H.222.0 / ISO/IEC 13818-1 [1].
