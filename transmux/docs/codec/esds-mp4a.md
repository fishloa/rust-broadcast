The reference type 'cdsc' (content describes) is the way within an MP4 file that description streams (such as MPEG-7) are linked to the content they describe; when the file is streamed or hinted, these track references are used to form an ObjectDescriptor describing the content and the description, or the DescriptionDescriptionDescriptor as appropriate.

## **5.3 Track Header Box**

The track header box documents the track duration. If the duration of a track cannot be determined, then the duration is set to all 1s (32-bit maxint): this is the case when an Elementary Stream Descriptor contains a ES\_URL, since the media content is outside the MP4 file and its partitioning into samples is not known. The track header flags track\_in\_movie and track\_in\_preview are not used in MP4 and shall be set to the default value of 1 in all files.

## **5.4 Handler Reference Types**

The following additional values for handler-type, in the Handler Reference Box ('hdlr') of the ISO Base Media File Format, are defined:

| 'odsm' | ObjectDescriptorStream  |
|--------|-------------------------|
| 'crsm' | ClockReferenceStream    |
| 'sdsm' | SceneDescriptionStream  |
| 'm7sm' | MPEG7Stream             |
| 'ocsm' | ObjectContentInfoStream |
| 'ipsm' | IPMP Stream             |
| 'mjsm' | MPEG-J Stream           |

## **5.5 MPEG-4 Media Header Boxes**

ISO/IEC 14496 streams other than visual and audio currently use an empty MPEG-4 Media Header Box, as defined here. There is a set of reserved types for media headers specific to these ISO/IEC 14496 stream types.

#### **5.5.1 Syntax**

```
aligned(8) class Mpeg4MediaHeaderBox extends NullMediaHeaderBox( flags ) { };
```

## **5.5.2 Semantics**

```
version - is an integer that specifies the version of this box. 
flags - is a 24-bit integer with flags (currently all zero).
```

The following box types are reserved as potential Media Header box types, but are currently unused:

```
ObjectDescriptorStream 'odhd' 
ClockReferenceStream 'crhd' 
SceneDescriptionStream 'sdhd' 
MPEG7Stream 'm7hd' 
ObjectContentInfoStream 'ochd' 
IPMP Stream 'iphd' 
MPEG-J Stream 'mjhd'
```

## **5.6 Sample Description Boxes**

**Box Types: 'mp4v', 'mp4a', 'mp4s' Container: Sample Table Box ('stbl')** 

**Mandatory: Yes** 

**Quantity: Exactly one**

For visual streams, a VisualSampleEntry is used; for audio streams, an AudioSampleEntry. For all other MPEG-4 streams, a MpegSampleEntry is used. Hint tracks use an entry format specific to their protocol, with an appropriate name.

For all the MPEG-4 streams, the data field stores an ES\_Descriptor with all its contents. Multiple entries in the table imply the occurrence of ES\_DescriptorUpdate commands. In case an ES\_Descriptor references the stream through an ES URL (thus outside the scope of the MP4 file as described in this document) only one entry in this table is allowed, i.e. the occurrence of ES\_DescriptorUpdate commands is not supported. The ES\_Descriptor as stored within the file format is constrained by the rules set in 3.1

For hint tracks, the sample description contains appropriate declarative data for the protocol being used, and the format of the hint track. The definition of the sample description is specific to the streaming protocol. However, note the discussion of FlexMux above, and the need for a Stream Map table, and MuxCode mode format definitions.

For visual streams, Annex K subclause 3.1 of the video specification requires that configuration information (e.g. the video sequence header) be carried in the decoder configuration structure, and not in stream. Since MP4 is a systems structure, it should be noted that that means that these headers (video object sequence, and so on) shall be in the ES\_descriptor in the sample description, and not in the media samples themselves.

#### **5.6.1 Syntax**

```
aligned(8) class ESDBox 
 extends FullBox('esds', version = 0, 0) { 
 ES_Descriptor ES; 
} 
 // Visual Streams 
class MP4VisualSampleEntry() extends VisualSampleEntry ('mp4v'){ 
 ESDBox ES; 
} 
 // Audio Streams 
class MP4AudioSampleEntry() extends AudioSampleEntry ('mp4a'){ 
 ESDBox ES; 
} 
 // all other Mpeg stream types 
class MpegSampleEntry() extends SampleEntry ('mp4s'){ 
 ESDBox ES; 
} 
aligned(8) class SampleDescriptionBox (unsigned int(32) handler_type) 
 extends FullBox('stsd', 0, 0){ 
 int i ; 
 unsigned int(32) entry_count; 
 for (i = 0 ; i < entry_count ; i++){ 
 switch (handler_type){ 
 case 'soun': // AudioStream 
 AudioSampleEntry(); 
 break; 
 case 'vide': // VisualStream 
 VisualSampleEntry(); 
 break; 
 case 'hint': // Hint track 
 HintSampleEntbry(); 
 break; 
 default : 
 MpegSampleEntry(); 
 break; 
 } 
 }
```

#### **5.6.2 Semantics**

Entry\_count — is an integer that gives the number of entries in the following table.

SampleEntry — is the appropriate sample entry.

width in the VisualSampleEntry is the maximum visual width of the stream described by this sample description, in pixels, as described in ISO/IEC 14496-2, 6.2.3, video\_object\_layer\_width in the visual headers; it is repeated here for the convenience of tools;

height in the VisualSampleEntry is the maximum visual height of the stream described by this sample description, in pixels, as described in ISO/IEC 14496-2, 6.2.3, video\_object\_layer\_height in the visual headers; it is repeated here for the convenience of tools;

compressorname in the sample entries shall be set to 0

ES — is the ES Descriptor for this stream.

## **5.7 Degradation Priority Values**

In the Degradation Priority Box, the maximum size of a degradation priority in the SL header is 15 bits; this is smaller than the field size of 16 bits. The most-significant bit is reserved as zero.

## **6 Template fields used**

In the section "Data Types and Fields" of the ISO Base Media File Format, the concept of "template" fields is defined. This specification derives from the base, and it is required that any derived specification state explicitly which template fields are used. This format uses no template fields.

When a file is created as a pure MPEG-4 file, those fields shall be set to their default values. If a file is multi-purpose and also complies with other specifications, then those fields may have non-default values as required by those other specifications.

When a file is read as an MPEG-4 file, the values in the template fields shall be ignored.

# **Annex A** (informative)

# **Patent statements**

The International Organization for Standardization and the International Electrotechnical Commission (IEC) draw attention to the fact that it is claimed that compliance with this part of ISO/IEC 14496 may involve the use of patents.

ISO and IEC take no position concerning the evidence, validity and scope of these patent rights.

The holders of these patent rights have assured the ISO and IEC that they are willing to negotiate licences under reasonable and non-discriminatory terms and conditions with applicants throughout the world. In this respect, the statements of the holders of these patents right are registered with ISO and IEC. Information may be obtained from the companies listed below.

Attention is drawn to the possibility that some of the elements of this part of ISO/IEC 14496 may be the subject of patent rights other than those identified in this annex. ISO and IEC shall not be held responsible for identifying any or all such patent rights.

|    | Company                                  |
|----|------------------------------------------|
| 1. | Apple                                    |
| 2. | IBM                                      |
| 3. | Matsushita Electric Industrial Co., Ltd. |
| 4. | Mitsubishi Electric                      |