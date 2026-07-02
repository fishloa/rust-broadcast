 DTSSpecificBox() // 'ddts' box 
}
```

For DTS\_SampleEntry(), the following values inherited from AudioSampleEntry are set as follows:

**codingname** is according to table E-1.

**channelcount** is set to the number of decodable output channels in basic playback, as described in the 'ddts' configuration box. Additional channel count as a result of future feature enhancements are defined in a box following the 'ddts' box, where ReservedBox() is the placeholder.

**samplesize** is always set to 16.

**samplerate** is set according to DTSSamplingFrequency of either:

- 48 000 for original sampling frequencies of 24 000 Hz, 48 000 Hz, 96 000 Hz or 192 000 Hz;
- 44 100 for original sampling frequencies of 22 050 Hz, 44 100 Hz, 88 200 Hz or 176 400 Hz;
- 32 000 for original sampling frequencies of 16 000 Hz, 32 000 Hz, 64 000 Hz or 128 000 Hz.

#### E.2.2.3 DTSSpecificBox

#### E.2.2.3.1 Syntax of DTSSpecificBox

The syntax and semantics of the DTSSpecificBox ('ddts') are shown below:

```
class DTSSpecificBox extends Box ('ddts'){ 
 unsigned int(32) size; 
 unsigned char type[4] = 'ddts'; 
 unsigned int(32) DTSSamplingFrequency; 
 unsigned int(32) maxBitrate; 
 unsigned int(32) avgBitrate; 
 unsigned char pcmSampleDepth; // value is 16 or 24 bits 
 bit(2) FrameDuration; // 0 = 512, 1 = 1024, 2 = 2048, 3 = 4096 
 bit(5) StreamConstruction; // Table E-2 
 bit(1) CoreLFEPresent; // 0 = none; 1 = LFE exists 
 bit(6) CoreLayout; // Table E-3 
 bit(14) CoreSize; 
 bit(1) StereoDownmix // 0 = none; 1 = embedded downmix present 
 bit(3) RepresentationType; // Table E-4 
 bit(16) ChannelLayout; // Table E-5 
 bit(1) MultiAssetFlag // 0 = single asset, 1 = multiple asset 
 bit(1) LBRDurationMod // 0 = ignore, 1 = Special LBR duration modifier 
 bit(1) ReservedBoxPresent // 0 = NoReservedBox, 1 = NoReservedBox present 
 bit(5) Reserved // Reserved bits are set to 0 
 ReservedBox() // optional, for future expansion 
};
```

#### E.2.2.3.2 Semantics
**MultiAssetFlag:** This flag is set if the stream contains more than one asset. This also implies that a DTS extension substream is present. Multiple asset streams use the 'dtsh' coding type. When multiple assets exist, the remaining parameters in the DTSSpecificBox only reflect the coding parameters of the first asset.

**LBRDurationMod:** This flag indicates a special case of the LBR coding bandwidth, resulting in 1/3 or 2/3 band limiting. The result of this is the LBR frame duration is 50 % larger than indicated in FrameDuration. For example, when this flag is set to 1, the FrameDuration is 6 144 samples instead of 4 096 samples.

**Reserved:** These bits are reserved for future definition. ISO media files created according to this version of specification will have these bits set to 0.

#### E.2.2.3.3 ReservedBox

The reserved box is optional and serves as a placeholder for future expansion. Additional private boxes may follow the 'ddts' box in the DTS\_SampleEntry(). Playback devices not equipped to support these additional extensions depend on the 'ddts' box for basic playback capability.

## E.3 Storage of DTS-HD Elementary Streams

One DTS-HD audio frame constitutes one sample. Samples shall be stored in the 'mdat' box in the same order in which they are intended to be played back.

## E.4 Restrictions on DTS Formats

The following conditions shall remain constant in a core substream for seamless playback:

- Duration of Synchronized Frame
- Sampling Frequency
- Audio Channel Arrangement
- Low Frequency Effects flag
- Extension assignment as indicated in StreamConstruction.

The following conditions shall remain constant in an Extension substream for seamless playback:

- Duration of Synchronized Frame
- Sampling Frequency
- Audio Channel Arrangement including LFE
- Embedded stereo flag
- Extensions assignment indicated in **StreamConstruction**

## E.5 Implementation of DTS Sample Entry

The information needed to derive the elements of the DTS Sample Entry box and boxes contained within it, may be extracted from the respective elementary stream. DTS has tools available to implementers that will analyse DTS elementary streams and extract the information necessary to populate these parameters. DTS document #9302J81100, describes the function calls and return structures. To obtain this tool and additional documentation, please direct all document requests to DTS Licensing at [LicensingAdministration@dts.com.](mailto:LicensingAdministration@dts.com)

# Annex F (normative): Application of DTS formats to MPEG-2 Streams

### F.1 Overview of Annex F

This annex specifies how DTS and DTS-HD audio is applied in MPEG-2 systems and provides additional information and references to the usage of DTS and DTS-HD elementary streams in DVB broadcast applications. While the use of DTS formats in DVB broadcast is optional, if they are used, the present document should be followed.

This Annex is informative in that it is not required to use DTS or DTS-HD audio in an MPEG-2 system. However, if the audio formats specified in the present document are implemented in MPEG-2 systems, this annex is to be followed.

Additional information pertaining to DTS formats in other MPEG-2 TS environments may be available at [www.dts.com](http://www.dts.com/).

## F.2 Buffering Model

The DTS buffering model is designed in accordance with ISO/IEC 13818-1 [6]. Refer to the derivation of BSn for audio elementary streams.

- For DTS core streams, the main audio buffer size (BSn) has a fixed value of 9 088 bytes, with a drain rate (Rxn) of 2 Mbps. The fixed value above (9 088 bytes) was calculated from a double buffer (2 × 4 096 bytes) plus jitter (384 bytes) + packet bursts (512 bytes).
- For DTS-HD Lossless formats, the value of BSn has a fixed value of 66 432 bytes, with an Rxn value of 32 Mbps.
- For all other DTS-HD formats, the value of BSn has a fixed value of 17 814 bytes, with an Rxn value of 8 Mbps.

## F.3 Signalling

#### F.3.1 PSI Signalling in the PMT

#### F.3.1.1 Overview of PSI Signalling for DTS and DTS-HD

Two related generations of DTS formats exist, the original DTS core format and the expanded DTS-HD format. As a result of this second generation of DTS formats, a new DTS-HD audio descriptor was created to accommodate the expanded feature set. This new structures can accommodate core only formats as well as extension only and core + extension combinations. If an MPEG-2 system supports DTS-HD, all DTS formats broadcast in that system may use the DTS-HD signalling as described in clause G.3 in ETSI EN 300 468 [1].

#### F.3.1.2 Stream Type

In DVB systems, and systems that follow DVB convention, DTS and DTS-HD elementary streams are signalled as private\_stream\_1 and therefore use a stream\_type = 0x06, consistent with ETSI TS 101 154 [2], clause 4.1.6.1 and in accordance with Recommendation ITU-T H.222.0/ISO/IEC 13818-1 [6].

In systems that follow ATSC convention, such as SCTE, DTS and DTS-HD have been assigned a value in the ATSC registry, therefore stream\_type is to be set to 0x88.