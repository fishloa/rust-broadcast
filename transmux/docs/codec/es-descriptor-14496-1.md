### 7.2.6.5 ES Descriptor

### 7.2.6.5.1 Syntax

```
class ES Descriptor extends BaseDescriptor : bit(8) tag=ES DescrTag {
  bit(16) ES ID:
  bit(1) streamDependenceFlag;
  bit(1) URL_Flag;
  bit(1) OCRstreamFlag;
  bit(5) streamPriority;
  if (streamDependenceFlag)
    bit(16) dependsOn_ES_ID;
  if (URL_Flag) {
    bit(8) URLlength;
    bit(8) URLstring[URLlength];
  if (OCRstreamFlag)
    bit(16) OCR ES Id;
  DecoderConfigDescriptor decConfigDescr;
  if (ODProfileLevelIndication==0x01)
                                           //no SL extension.
  {
    SLConfigDescriptor slConfigDescr;
  }
  else
                                         // SL extension is possible.
  {
    SLConfigDescriptor slConfigDescr;
  IPI_DescrPointer ipiPtr[0 .. 1];
  IP_IdentificationDataSet ipIDS[0 .. 255];
  IPMP_DescriptorPointer ipmpDescrPtr[0 .. 255];
  LanguageDescriptor langDescr[0 .. 255];
  QoS_Descriptor gosDescr[0 .. 1];
  RegistrationDescriptor regDescr[0 .. 1];
  ExtensionDescriptor extDescr[0 .. 255];
}
```

### 7.2.6.5.2 Semantics

The ES\_Descriptor conveys all information related to a particular elementary stream and has three major parts.

The first part consists of the ES\_ID which is a unique reference to the elementary stream within its name scope (see 7.2.7.2.4), a mechanism to describe dependencies of elementary streams within the scope of the parent object descriptor and an optional URL string. Dependencies and usage of URLs are specified in 7.2.7.

The second part consists of the component descriptors which convey the parameters and requirements of the elementary stream.

The third part is a set of optional extension descriptors that support the inclusion of future extensions as well as the transport of private data in a backward compatible way.

ES\_ID - This syntax element provides a unique label for each elementary stream within its name scope. The values 0 and 0xFFFF are reserved.

streamDependenceFlag - If set to one indicates that a dependson ES ID will follow.

URL\_Flag - if set to 1 indicates that a URLstring will follow.

OCRstreamFlag - indicates that an OCR\_ES\_ID syntax element will follow.

# ISO/IEC 14496-1:2010(E)

streamPriority - indicates a relative measure for the priority of this elementary stream. An elementary stream with a higher streamPriority is more important than one with a lower streamPriority. The absolute values of streamPriority are not normatively defined.

depends on ES ID - is the ES ID of another elementary stream on which this elementary stream depends. The stream with depends on ES ID shall also be associated to the same object descriptor as the current ES\_Descriptor.

URLlength - the length of the subsequent URLstring in bytes.

URLstring[] - contains a UTF-8 (ISO/IEC 10646-1) encoded URL that shall point to the location of an SLpacketized stream by name. The parameters of the SL-packetized stream that is retrieved from the URL are fully specified in this ES\_Descriptor. See also 7.2.7.3.3. Permissible URLs may be constrained by profile and levels as well as by specific delivery layers.

OCR\_ES\_ID - indicates the ES\_ID of the elementary stream within the name scope (see 7.2.7.2.4) from which the time base for this elementary stream is derived. Circular references between elementary streams are not permitted.

decConfigDescr - is a DecoderConfigDescriptor as specified in 7.2.6.6.

slConfigDescr - is an SLConfigDescriptor as specified in 7.2.6.8. If ODProfileLevelIndication is different from 0x01, it may be an extension of SLConfigDescriptor (i.e. and extended class) as defined in 7.2.6.8.

ipiPtr[] - an array of zero or one IPI\_DescrPointer as specified in 7.2.6.12.

ipIDS[] - an array of zero or more IP IdentificationDataSet as specified in 7.2.6.9.

Each ES Descriptor shall have either one IPI DescrPointer or zero uр to 255 IP\_IdentificationDataSet elements. This allows to unambiguously associate an IP Identification to each elementary stream.

ipmpDescrPtr[] - an array of IPMP\_DescriptorPointer, as defined in 7.2.6.13, that points to the IPMP\_Descriptors related to the elementary stream described by this ES\_Descriptor. The array shall have any number of zero up to 255 elements.

langDescr[] - an array of zero or one LanguageDescriptor structures as specified in 7.2.6.18.6. It indicates the language attributed to this elementary stream.

NOTE — Multichannel audio streams may be treated as one elementary stream with one ES Descriptor by ISO/IEC 14496. In that case different languages present in different channels of the multichannel stream are not identifyable with a LanguageDescriptor.

gosDescr[] - an array of zero or one OoS Descriptor as specified in 7.2.6.15.

extDescr[] - an array of ExtensionDescriptor structures as specified in 7.2.6.16.

#### <span id="page-1-0"></span>7.2.6.6 DecoderConfigDescriptor

#### 7.2.6.6.1 **Syntax**

```
class DecoderConfigDescriptor extends BaseDescriptor : bit(8)
tag=DecoderConfigDescrTag {
  bit(8) objectTypeIndication;
  bit(6) streamType;
  bit(1) upStream;
  const bit(1) reserved=1;
```

```
bit(24) bufferSizeDB;
bit(32) maxBitrate;
bit(32) avgBitrate;
DecoderSpecificInfo decSpecificInfo[0 .. 1];
profileLevelIndicationIndexDescriptor profileLevelIndicationIndexDescr
[0..255];
}
```

#### 7.2.6.6.2 Semantics

The <code>DecoderConfigDescriptor</code> provides information about the decoder type and the required decoder resources needed for the associated elementary stream. This is needed at the receiving terminal to determine whether it is able to decode the elementary stream. A stream type identifies the category of the stream while the optional decoder specific information descriptor contains stream specific information for the set up of the decoder in a stream specific format that is opaque to this layer.

 ${\tt ObjectTypeIndication} \ - \ an \ indication \ of \ the \ object \ or \ scene \ description \ type \ that \ needs \ to \ be \ supported \ by \ the \ decoder \ for \ this \ elementary \ stream \ as \ per \ Table \ 5.$ 

<span id="page-2-0"></span>Table 5 — objectTypeIndication Values

| Value     | ObjectTypeIndication Description                                   |
|-----------|--------------------------------------------------------------------|
| 0x00      | Forbidden                                                          |
| 0x01      | Systems ISO/IEC 14496-1 a                                          |
| 0x02      | Systems ISO/IEC 14496-1 b                                          |
| 0x03      | Interaction Stream                                                 |
| 0x04      | Systems ISO/IEC 14496-1 Extended BIFS Configuration <sup>c</sup>   |
| 0x05      | Systems ISO/IEC 14496-1 AFX <sup>d</sup>                           |
| 0x06      | Font Data Stream                                                   |
| 0x07      | Synthesized Texture Stream                                         |
| 0x08      | Streaming Text Stream                                              |
| 0x09-0x1F | reserved for ISO use                                               |
| 0x20      | Visual ISO/IEC 14496-2 <sup>e</sup>                                |
| 0x21      | Visual ITU-T Recommendation H.264   ISO/IEC 14496-10 f             |
| 0x22      | Parameter Sets for ITU-T Recommendation H.264   ISO/IEC 14496-10 f |
| 0x23-0x3F | reserved for ISO use                                               |
| 0x40      | Audio ISO/IEC 14496-3 <sup>9</sup>                                 |
| 0x41-0x5F | reserved for ISO use                                               |
| 0x60      | Visual ISO/IEC 13818-2 Simple Profile                              |
| 0x61      | Visual ISO/IEC 13818-2 Main Profile                                |
| 0x62      | Visual ISO/IEC 13818-2 SNR Profile                                 |
| 0x63      | Visual ISO/IEC 13818-2 Spatial Profile                             |
| 0x64      | Visual ISO/IEC 13818-2 High Profile                                |
| 0x65      | Visual ISO/IEC 13818-2 422 Profile                                 |
| 0x66      | Audio ISO/IEC 13818-7 Main Profile                                 |
| 0x67      | Audio ISO/IEC 13818-7 LowComplexity Profile                        |
| 0x68      | Audio ISO/IEC 13818-7 Scaleable Sampling Rate Profile              |
| 0x69      | Audio ISO/IEC 13818-3                                              |
| 0x6A      | Visual ISO/IEC 11172-2                                             |
| 0x6B      | Audio ISO/IEC 11172-3                                              |
| 0x6C      | Visual ISO/IEC 10918-1                                             |
| 0x6D      | reserved for registration authority i                              |

| Value       | ObjectTypeIndication Description      |
|-------------|---------------------------------------|
| 0x6E        | Visual ISO/IEC 15444-1                |
| 0x6F - 0x9F | reserved for ISO use                  |
| 0xA0 - 0xBF | reserved for registration authority i |
| 0xC0 - 0xE0 | user private                          |
| 0xE1        | reserved for registration authority i |
| 0xE2 - 0xFE | user private                          |
| 0xFF        | no object type specified h            |

a This type is used for all 14496-1 streams unless specifically indicated to the contrary. Scene Description scenes, which are identified with StreamType=0x03, using this object type value shall use the BIFSConfig specified in ISO/IEC 14496-11.

When the objectTypeIndication value is 0x6C (Visual ISO/IEC 10918-1, which is JPEG) the stream may contain one or more Access Units, where one Access Unit is defined to be a complete JPEG (as defined in Visual ISO/IEC 10918-1). Note, that timing and other Access Unit and packetization information is to be carried in the transport layer such as the MPEG-4 Sync Layer.

When the objectTypeIndication value is 0x6E (Visual ISO/IEC 15444-1, which is JPEG 2000) the stream may contain one or more Access Units, where one Access Unit is defined to be a complete JPEG 2000 (as defined in Visual ISO/IEC 15444-1). Note, that timing and other Access Unit and packetization information is to be carried in the transport layer such as the MPEG-4 Sync Layer.

NOTE The format defined in ISO/IEC 15444-3 is preferred for the storage of JPEG 2000 sequences in file format of the ISO/IEC 14496-12 family, including MP4.

streamType – conveys the type of this elementary stream as per Table 6.

This object type shall be used, with StreamType=0x03, for Scene Description streams that use the BIFSv2Config specified in ISO/IEC 14496-11. Its use with other StreamTypes is reserved.

c This object type shall be used, with StreamType=0x03, for Scene Description streams that use the BIFSConfigEx specified in 7.2.6.7 of this specification. Its use with other StreamTypes is reserved.

d This object type shall be used, with StreamType=0x03, for Scene Description streams that use the AFXConfig specified in 7.2.6.7 of this specification. Its use with other StreamTypes is reserved.

e Includes associated Amendment(s) and Corrigendum(a). The actual object types are defined in ISO/IEC 14496-2 and are conveyed in the DecoderSpecificInfo as specified in ISO/IEC 14496-2, Annex K.

f Includes associated Amendment(s) and Corrigendum(a). The actual object types are defined in ITU-T Recommendation H.264 | ISO/IEC 14496-10 and are conveyed in the DecoderSpecificInfo as specified in this amendment, I.2.

Includes associated Amendment(s) and Corrigendum(a). The actual object types are defined in ISO/IEC 14496-3 and are conveyed in the DecoderSpecificInfo as specified in ISO/IEC 14496-3 subpart 1 subclause 6.2.1.

h Streams with this value with a StreamType indicating a systems stream (values 1,2,3, 6, 7, 8, 9) shall be treated as if the ObjectTypeIndication had been set to 0x01.

i The latest entries registered can be found at [http://www.mp4ra.org/object.html.](http://www.mp4ra.org/object.html)

| streamType value | Stream type description                       |
|------------------|-----------------------------------------------|
| 0x00             | Forbidden                                     |
| 0x01             | ObjectDescriptorStream (see 7.2.5)            |
| 0x02             | ClockReferenceStream (see 7.3.2.5)            |
| 0x03             | SceneDescriptionStream (see ISO/IEC 14496-11) |
| 0x04             | VisualStream                                  |
| 0x05             | AudioStream                                   |
| 0x06             | MPEG7Stream                                   |
| 0x07             | IPMPStream (see 7.2.3.2)                      |
| 0x08             | ObjectContentInfoStream (see 7.2.4.2)         |
| 0x09             | MPEGJStream                                   |
| 0x0A             | Interaction Stream                            |
| 0x0B             | IPMPToolStream (see [ISO/IEC 14496-13])       |
| 0x0C - 0x1F      | reserved for ISO use                          |
| 0x20 - 0x3F      | user private                                  |

**Table 6 — streamType Values**

upStream – indicates that this stream is used for upstream information.

bufferSizeDB – is the size of the decoding buffer for this elementary stream in byte.

maxBitrate – is the maximum bitrate in bits per second of this elementary stream in any time window of one second duration.

avgBitrate – is the average bitrate in bits per second of this elementary stream. For streams with variable bitrate this value shall be set to zero.

decSpecificInfo[] – an array of zero or one decoder specific information classes as specified in [7.2.6.7.](#page-4-0)

ProfileLevelIndicationIndexDescr [0..255] – an array of unique identifiers for a set of profile and level indications as carried in the ExtensionProfileLevelDescr defined in 7.2.6.19.

## <span id="page-4-0"></span>**7.2.6.7 DecoderSpecificInfo**

# **7.2.6.7.1 Syntax**

```
abstract class DecoderSpecificInfo extends BaseDescriptor : bit(8) 
tag=DecSpecificInfoTag 
{ 
 // empty. To be filled by classes extending this class. 
}
```

### **7.2.6.7.2 Semantics**

The decoder specific information constitutes an opaque container with information for a specific media decoder. The existence and semantics of decoder specific information depends on the values of DecoderConfigDescriptor.streamType and DecoderConfigDescriptor.objectTypeIndication.

For values of DecoderConfigDescriptor.objectTypeIndication that refer to streams complying with ISO/IEC 14496-2 the syntax and semantics of decoder specific information are defined in Annex K of that part.

# **ISO/IEC 14496-1:2010(E)**

For values of DecoderConfigDescriptor.objectTypeIndication that refer to streams complying with ISO/IEC 14496-3 the syntax and semantics of decoder specific information are defined in subpart 1, subclause 1.6 of that part.

For values of DecoderConfigDescriptor.objectTypeIndication that refer to scene description streams the semantics of decoder specific information is defined in ISO/IEC 14496-11.

For values of DecoderConfigDescriptor.objectTypeIndication that refer to streams complying with ISO/IEC 13818-7 the decoder specific information consists of an "adif\_header()" and an access unit is a "raw\_data\_block()" as defined in ISO/IEC 13818-7.

For values of DecoderConfigDescriptor.objectTypeIndication that refer to streams complying with ISO/IEC 11172-3 or ISO/IEC 13818-3 the decoder specific information is empty since all necessary data is contained in the bitstream frames itself. The access units in this case are the "frame()" bitstream element as is defined in ISO/IEC 11172-3.

For values of DecoderConfigDescriptor.objectTypeIndication that refer to streams complying with ISO/IEC 10918-1, the decoder specific information is:

```
class JPEG_DecoderConfig extends DecoderSpecificInfo : bit(8) 
tag=DecSpecificInfoTag { 
 int(16) headerLength; 
 int(16) Xdensity; 
 int(16) Ydensity; 
 int(8) numComponents; 
}
```

### with

headerLength –indicates the number of bytes to skip from the beginning of the stream to find the first pixel of the image.

Xdensity and Ydensity – specify the pixel aspect ratio.

numComponents – indicates whether the image has Y component only or is Y, Cr, Cb. It shall be equal to 1 or 3.

For values of DecoderConfigDescriptor.objectTypeIndication that refer to interaction streams, the decoder specific information is:

```
class UIConfig extends DecoderSpecificInfo : bit(8) tag=DecSpecificInfoTag { 
 bit(8) deviceNamelength; 
 bit(8) deviceName[deviceNamelength]; 
 bit(8) devSpecInfo[sizeOfInstance – deviceNamelength - 1]; 
}
```

# with

deviceNameLength –indicates the number of bytes in the deviceName field

deviceName –indicates the name of the class of device, which allows the terminal to invoke the appropriate interaction decoder.

devSpecInfo –is a opaque container with information for a device specific handler.

For values of DecoderConfigDescriptor.objectTypeIndication that refers to extended BIFS configuration (0x04), the decoder specific information is:

#### 7.3.2.3 **SL Packet Header Configuration**

#### 7.3.2.3.1 **Syntax**

```
class SLConfigDescriptor extends BaseDescriptor : bit(8) tag=SLConfigDescrTag {
  bit(8) predefined;
  if (predefined==0) {
    bit(1) useAccessUnitStartFlag;
    bit(1) useAccessUnitEndFlag;
    bit(1) useRandomAccessPointFlag;
    bit(1) hasRandomAccessUnitsOnlyFlag;
    bit(1) usePaddingFlag;
    bit(1) useTimeStampsFlag;
    bit(1) useIdleFlag:
    bit(1) durationFlag;
    bit(32) timeStampResolution;
    bit(32) OCRResolution;
    bit(8) timeStampLength; // must be ≤ 64
    bit(8) OCRLength: // must be \leq 64
    bit(8) AU Length;
                          // must be ≤ 32
    bit(8) instantBitrateLength;
    bit(4) degradationPriorityLength;
    bit(5) AU segNumLength; // must be \leq 16
    bit(5) packetSeqNumLength; // must be ≤ 16
    bit(2) reserved=0b11;
  }
  if (durationFlag) {
    bit(32) timeScale;
    bit(16) accessUnitDuration;
    bit(16) compositionUnitDuration;
  if (!useTimeStampsFlag) {
    bit(timeStampLength) startDecodingTimeStamp;
    bit(timeStampLength) startCompositionTimeStamp;
}
class ExtendedSLConfigDescriptor extends SLConfigDescriptor : bit(8)
tag=ExtSLConfigDescrTag {
  SLExtensionDescriptor slextDescr[1..255];
```

#### 7.3.2.3.2 **Semantics**

The SL packet header may be configured according to the needs of each individual elementary stream. Parameters that can be selected include the presence, resolution and accuracy of time stamps and clock references. This flexibility allows, for example, a low bitrate elementary stream to incur very little overhead on SL packet headers.

For each elementary stream the configuration is conveyed in an SLConfigDescriptor, which is part of the associated ES Descriptor within an object descriptor.

The configurable parameters in the SL packet header can be divided in two classes: those that apply to each SL packet (e.g. OCR, sequenceNumber) and those that are strictly related to access units (e.g. time stamps, accessUnitLength, instantBitrate, degradationPriority).

predefined - allows to default the values from a set of predefined parameter sets as detailed below.