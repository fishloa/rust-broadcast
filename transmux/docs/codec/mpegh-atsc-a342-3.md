## 4.2.2 Multi-Stream Delivery

The multi-stream-enabled MPEG-H Audio system is capable of handling streams delivered in a hybrid environment (e.g., one stream, containing one complete presentation, delivered over Broadcast and one or more additional streams, containing different languages, delivered over Broadband). The MAE information contained in each stream allows the MPEG-H Audio decoder to correctly merge the streams into one stream containing several sub-streams.

[Figure 4.2](#page-0-0) illustrates an example of such a hybrid delivery scenario, in which, out of several incoming streams, the main stream (stream #0) and the third stream (stream #2) are selected and merged into a single stream, while the second stream (stream #1) is discarded based on information obtained from the systems level.

![](_page_0_Figure_4.jpeg)

**Figure 4.2** Example of switching and merging multiple incoming streams.

#### <span id="page-0-0"></span>4.2.3 Audio/Video Fragment Alignment and Seamless Configuration Change

The MPEG-H Audio system allows for sample-accurate splicing and reconfiguration of the audio stream at Fragment boundaries. For coding efficiency, the audio and video streams usually use different frame rates, however, some applications may require the audio and video streams to be aligned at the end of certain Fragments (e.g., for stream splicing – normally used for commercial insertion). In this case, at least some audio frames usually need to be truncated to a number of audio samples lower than the normal audio frame size.

The MPEG-H Audio system uses a packet structure, called the MPEG-H Audio Stream (MHAS), to transport encoded audio and corresponding metadata. The MHAS packet design guarantees that any MHAS packet payload always is byte-aligned so that the truncation information is easily accessible on the fly and can be easily inserted, removed or modified by, e.g., a stream splicing device.

Another application of the truncation method is changing the configuration of an MPEG-H Audio stream (e.g., a stereo program may be followed by a program with 5.1 channels and additional audio objects). The configuration usually will change at a video frame boundary that is

not aligned with the granules of the audio stream. Using the audio sample truncation method, the configuration change can be aligned to the video frame boundary.

### 4.2.4 Seamless Switching

The MPEG-H Audio system allows for seamless transitions between audio streams by making use of Immediate Play-out Frames (IPFs). Random-access points can be created, for example, by introducing IPFs at arbitrary positions in the audio stream.

IPFs carry additional information of "previous" audio frames and thereby allow seamless transitions between audio streams, e.g., in an adaptive streaming (e.g., DASH) scenario. Having IPFs at segment boundaries allows for glitch-free bitrate adaptation of different audio representations and for on-the-fly reconfiguration of the stream.

#### 4.2.5 Loudness and Dynamic Range Control

The MPEG-H Audio system includes advanced tools for loudness and dynamic range control inherited from MPEG-D DRC. MPEG-D DRC defines a comprehensive and flexible metadata format that includes transmission of loudness metadata according to recommendations ITU-R BS.1770 [5] and ITU-R BS.1771 [9] among others. Further, MPEG-D DRC is compliant to worldwide regulations including those based on ATSC A/85 [5].

The dynamic range control tool offers a rich feature set for accurate adaptation of both complete audio scenes and single elements within audio scenes for different receiving devices and different listening environments. For personalization, the MPEG-H Audio stream can include optional DRC configurations selectable by the user that, e.g., offer improved dialog intelligibility.

In addition to compression provisions, the dynamic range control tool offers broadcastercontrolled ducking of selected audio elements by sending time-varying gain values (e.g., for audio scenes that are combined with voice-overs or Audio Description).

# 4.2.6 Interactive Loudness Control

MPEG-H Audio enables listeners to interactively control and adjust different elements of an audio scene within limits set by broadcasters. To fulfill applicable broadcast regulations and recommendations with respect to loudness consistency, the MPEG-H Audio system includes a tool for automatic compensation of loudness variations due to user interactions.

# **5. MPEG-H AUDIO SPECIFICATION**

# 5.1 Audio Encoding

#### 5.1.1 Baseline Profile

Audio signals shall be encoded into bit streams according to the ISO/IEC 23008-3, MPEG-H Baseline (BL) Profile Level 1, 2, or 3, as defined in [2].

All constraints specified in Sections 5.2, 5.3 and 5.4 shall apply.

## 5.1.2 Low Complexity Profile

Audio signals shall be encoded into bit streams according to the ISO/IEC 23008-3, MPEG-H Low Complexity (LC) Profile Level 1, 2, or 3, as defined in [2].

If the audio signals are encoded according to the BL Profile restrictions specified in ISO/IEC 23008-3, Clause 4.8.2.6 [2], the CompatibleProfileLevelSet() config extension element specified in ISO/IEC 23008-3, Clause 4.8.2.7 [2] shall be present. The mpegh3daProfileLevelIndication and CompatibleSetIndication shall be as set according to Table P.1 in ISO/IEC 23008-3[2].

All constraints specified in Sections 5.2, 5.3 and 5.4 shall apply.

# 5.2 Bit Stream Encapsulation

# 5.2.1 MHAS (MPEG-H Audio Stream) Elementary Stream

Audio data shall be encapsulated into MPEG-H Audio Stream (MHAS) packets according to ISO/IEC 23008-3, Clause 14 [2].

MHAS packets of all types defined in ISO/IEC 23008-3, Clause 14 [2] may be present in an MHAS elementary stream, except for the following packet types, which shall not be present in the stream:

- PACTYP\_CRC16
- PACTYP\_CRC32
- PACTYP\_GLOBAL\_CRC16
- PACTYP\_GLOBAL\_CRC32

The following packet types may be present in an MHAS elementary stream. If they are present, however, they may be ignored by decoders:

- PACTYP\_SYNC
- PACTYP\_SYNCGAP

If Audio Scene Information according to ISO/IEC 23008-3, Clause 15 [2] is present, it always shall be encapsulated in an MHAS PACTYP\_AUDIOSCENEINFO packet [2]. Audio Scene Information shall not be included in the mpegh3daConfig() structure in the MHAS PACTYP\_MPEGH3DACFG packet.

If text labels for Group of Elements, Switch Groups or Presets should be carried within an MPEG-H Audio Stream, they may be encapsulated either as part of the MHAS PACTYP\_AUDIOSCENEINFO packet within an mae\_Description() structure, or alternatively they may be encapsulated within an MHAS PACTYP\_DESCRIPTOR packet carrying an MPEG-H\_3dAudio\_text\_label\_descriptor().

If content identifiers should be carried within an MPEG-H Audio Stream, they may be encapsulated in an MHAS PACTYP\_MARKER packet with the marker\_byte set to "E0".

The immersiveDownmixFlag in the downmixConfig() structure shall be set to "0".

The maximum bitrate shall be limited to 1 200 kbps. This limit applies to the maximum number of decoder-processed core channels (see ISO/IEC 23008-3, Table 12). The total bitrate can be larger and up to 2 400 kbps if there are 32 core channels in the compressed data stream. For bitstreams that fulfill all the complexity restrictions specified in ISO/IEC 23008-3, 4.8.2.5.2, the bitrate of up to 24 decoder-processed core channels shall be limited to 1 540 kbps.

When language is specified, the language tag shall conform to the syntax and semantics defined in IETF BCP 47 [4]. That language tag shall be mapped to the corresponding MAE metadata elements mae\_bsDescriptionLanguage and mae\_contentLanguage as specified in ISO/IEC 23008-3, Clause 15.3 [2].

Note: The IANA language subtag registry can be found here [11].

#### 5.2.2 ISOBMFF Encapsulation

#### 5.2.2.1 MPEG-H Audio Sample Entry

The sample entry "mhm1" shall be used for encapsulation of MHAS packets into ISOBMFF files, according to ISO/IEC 23008-3, Clause 20.6 [2].

The sample entry "mhm2" shall be used in cases of multi-stream or hybrid delivery, i.e., when the MPEG-H Audio Program is split into two or more streams for delivery as described in ISO/IEC 23008-3, Clause 14.6 [2].

If the MHAConfigurationBox() is present, the MPEG-H Profile-Level Indicator mpegh3daProfileLevelIndication in the MHADecoderConfigurationRecord() shall be set to:

- "0x0B", "0x0C", or "0x0D" for MPEG-H Audio LC Profile Level 1, Level 2, or Level 3, respectively.
- "0x10", "0x11", or "0x12" for MPEG-H Audio BL Profile Level 1, Level 2, or Level 3, respectively.

The Profile-Level Indicator in the MHAS PACTYP\_MPEGH3DACFG packet shall be set accordingly.

If the MHAProfileAndLevelCompatibilitySetBox() is present, the parameters carried in the MHAProfileAndLevelCompatibilitySetBox() shall be consistent with the configuration of the audio bitstream. In particular, the MPEG-H 3D audio profile and level compatibility indicator CompatibleSetIndication shall be set to "0x10", "0x11", or "0x12" for MPEG-H 3D audio BL profile Level 1, Level 2, or Level 3, respectively.

#### 5.2.2.2 Random Access Point and Stream Access Point

A File Format sample containing a Random Access Point (RAP), i.e., a RAP into an MPEG-H Audio Stream, is a "sync sample" in the ISOBMFF and shall consist of the following MHAS packets, in the following order:

- PACTYP\_MPEGH3DACFG
- PACTYP\_AUDIOSCENEINFO (if Audio Scene Information is present)
- PACTYP\_BUFFERINFO
- PACTYP\_MPEGH3DAFRAME

Note that additional MHAS packets may be present between the MHAS packets listed above or after the MHAS packet PACTYP\_MPEGH3DAFRAME, with one exception: when present, the PACTYP\_AUDIOSCENEINFO packet shall directly follow the PACTYP\_MPEGH3DACFG packet, as defined in ISO/IEC 23008-3, Clause 14.4 [2].

Additionally, the following constraints shall apply for sync samples:

- The audio data encapsulated in the MHAS packet PACTYP\_MPEGH3DAFRAME shall follow the rules for random access points as defined in ISO/IEC 23008-3, Clause 5.7 [2].
- All rules defined in ISO/IEC 23008-3, Clause 20.6.1 [2] regarding sync samples shall apply.
- The first sample of an ISOBMFF file shall be a RAP. In cases of fragmented ISOBMFF files, the first sample of each Fragment shall be a RAP.
- In case of non-fragmented ISOBMFF files, a RAP shall be signaled by means of the File Format sync sample box "stss", as defined in ISO/IEC 23008-3, Clause 20.2 [2].
- In case of fragmented ISOBMFF files, the sample flags in the Track Run Box ('trun') are used to describe the sync samples. The "sample\_is\_non\_sync\_sample" flag shall be set to "0" for a RAP; it shall be set to "1" for all other samples.

#### 5.2.2.3 Configuration Change

A configuration change takes place in an audio stream when the content setup or the Audio Scene Information changes (e.g., when changes occur in the channel layout, the number of objects etc.), and therefore new PACTYP\_MPEGH3DACFG and PACTYP\_AUDIOSCENEINFO packets are required upon such occurrences. A configuration change usually happens at program boundaries, but it may also occur within a program.

The following constraints apply:

- At each configuration change, the MHASPacketLabel shall be changed to a different value from the MHASPacketLabel in use before the configuration change occurred.
- A configuration change may happen at the beginning of a new ISOBMFF file or Fragment or at any position within the file. In the latter case, the File Format sample that contains a configuration change shall be encoded as a sync sample (RAP) as defined above.
- A sync sample that contains a configuration change and the last sample before such a sync sample may contain a truncation message (PACTYP\_AUDIOTRUNCATION) as defined in ISO/IEC 23008-3, Clause 14.4.13 [2].

The usage of truncation messages enables synchronization between video and audio elementary streams at program boundaries. When used, sample-accurate splicing and reconfiguration of the audio stream are possible. If MHAS packets of type PACTYP\_AUDIOTRUNCATION are present, they shall be used as described in ISO/IEC 23008-3, Clause 14.4.13 [2].

#### 5.2.2.4 Multi-Stream Delivery

In case of multi-stream delivery (as described in Section 4.2) the Audio Program Components of one Audio Program are not carried within one single MHAS elementary stream, but in two or more MHAS elementary streams.

The following constraints apply for ISOBMFF files or fragments using the sample entry "mhm2":

- The Audio Program Components of one Audio Program are carried in one main MHAS elementary stream, and one or more auxiliary MHAS elementary streams.
- The main MHAS stream shall contain at least the Audio Program Components corresponding to the default Audio Presentation, i.e., the Audio Scene Information is present and exactly one preset shall have the mae\_groupPresetID field set to "0", as specified in ISO/IEC 23008-3, Clause 15.3 [2].
- The mae\_isMainStream field in the Audio Scene Information shall be set to "1" in the main MHAS stream, as specified in ISO/IEC 23008-3, Clause 15.3 [2]. This field shall be set to "0" in the auxiliary MHAS streams.
- In each auxiliary MHAS stream the mae\_bsMetaDataElementIDoffset field in the Audio Scene Information shall be set to the index of the first metadata element in the auxiliary MHAS stream minus one, as specified in ISO/IEC 23008-3, Clause 14.6 and Clause 15.3 [2].
- For the main and the auxiliary MHAS stream(s), the MHASPacketLabel shall be set according to ISO/IEC 23008-3, Clause 14.6 [2].
- The main and the auxiliary MHAS stream(s) that carry Audio Program Components of one Audio Program shall be time aligned.
- In each auxiliary MHAS stream, the random access points (RAP) shall be aligned to the RAPs present in the main MHAS stream.

# 5.3 Audio Loudness and DRC Signaling

Loudness metadata shall be embedded within the mpegh3daLoudnessInfoSet() structure as defined in ISO/IEC 23008-3, Clause 6.3 [2]. Such loudness metadata shall include at least the loudness of the content rendered to the default rendering layout as indicated by the referenceLayout field (see ISO/IEC 23008-3, Clause 5.3.2 [2]). More precisely, the mpegh3daLoudnessInfoSet() structure shall

include at least one loudnessInfo() structure with loudnessInfoType set to "0", whose drcSetId and downmixId fields are set to "0" and which includes at least one methodValue field with methodDefinition set to "1" or "2" (see ISO/IEC 23008-3, Clause 6.3.1 [2] and ISO/IEC 23003-4, Clause 7.3 [3]). The indicated loudness value shall be measured according to local loudness regulations (e.g., ATSC A/85 [5]).

DRC metadata shall be embedded in the mpegh3daUniDrcConfig() and uniDrcGain() structures as defined in ISO/IEC 23008-3, Clause 6.3 [2]. For each included DRC set the drcSetTargetLoudnessPresent field as defined in ISO/IEC 23003-4, Clause 7.3 [3] shall be set to "1". The bsDrcSetTargetLoudnessValueUpper and bsDrcSetTargetLoudnessValueLower fields shall be configured to continuously cover the range of target loudness levels between -31 dB and 0 dB.

Loudness compensation information (mae\_LoudnessCompensationData()), as defined in ISO/IEC 23008-3, Clause 15.5 [2], shall be present in the Audio Scene Information if the mae\_allowGainInteractivity field (according to ISO/IEC 23008-3, Clause 15.3 [2]) is set to "1" for at least one group of audio elements.

# 5.4 Audio Emergency Information

The MPEG-H Audio system can insert Audio Emergency Information as an Audio Program Component part of one or more Audio Presentations. If Audio Emergency Information is present than the mae\_contentKind field (according to ISO/IEC 23008-3, Clause 15.3 [2]) of the Audio Program Component carrying the Audio Emergency Information shall be set to "12" ("emergency").

# *Annex A:* Decoder Guidelines

# **A.1 MPEG-H AUDIO DECODER OVERVIEW**

MPEG-H Audio offers the possibility to code channel-based content, object-based content and scene-based content, the latter using Higher Order Ambisonics (HOA) for a sound-field representation. [Figure A.1.1](#page-6-0) gives a brief overview of signal flow in an MPEG-H Audio decoder from bit stream input to loudspeaker or headphone output. As a first step, all transmitted audio signals are decoded by the MPEG-H Audio Core Decoder. Channel-based signals are mapped to the target reproduction loudspeaker layout using the Format Conversion module. Object-based signals are rendered to the target reproduction loudspeaker layout by the Object Renderer. Scenebased content is rendered to the target reproduction loudspeaker layout using associated HOA metadata and an HOA decoder/renderer.

![](_page_6_Figure_6.jpeg)

**Figure A.1.1** MPEG-H Audio Decoder (features, block diagram, signal flow).

# <span id="page-6-0"></span>**A.2 OUTPUT SIGNALS**

# **A.2.1 Loudspeaker Output**

The MPEG-H Audio Decoder is able to render the encoded input signals to loudspeaker output channel signals for any target loudspeaker layout geometry.

The MPEG-H LC Profile Level 3 specified in ISO/IEC 23008-3 [2] limits the maximum number of output channels to 12.

The receiving device obtains the target loudspeaker layout from external information supplied during system setup and passes that information to the MPEG-H Audio decoder during its initialization, as defined in ISO/IEC 23008-3, Clause 17.2 [2].

Examples for target loudspeaker layouts: