---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Clause 5.3.7 - Common attributes and elements"
pdf_pages: "71-78"
printed_pages: "63-70"
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION.md
---

<a id="c5-3-7"></a>

### 5.3.7 Common attributes and elements

<a id="c5-3-7-1"></a>

#### 5.3.7.1 Overview

The elements AdaptationSet, Representation and SubRepresentation have assigned common attributes and elements. The attributes and elements listed in Table 14 of subclause [5.3.7.2](#c5-3-7-2) may be present in all three elements.

The semantics of the common attributes and elements are provided in Table 14 in subclause [5.3.7.2](#c5-3-7-2), the syntax is provided in subclause [5.3.7.3](#c5-3-7-3).

The 'Use' column in Table 14 shall be interpreted that an attribute marked with 'M' shall be available for a Representation, i.e. it shall either be present in the Representation element, or if not, it shall be in the containing AdaptationSet element. An attribute marked with 'O' may be absent in both.

<!--PB printed 63-->

<a id="c5-3-7-2"></a>

#### 5.3.7.2 Semantics

Table 14 — Common Adaptation Set, Representation and Sub-Representation attributes and elements

| Element or Attribute Name | Use (see subclause [5.3.7.1](#c5-3-7-1)) | Description |
|---|---|---|
| ***Common attributes and elements*** |  |  |
| `@profiles` | O | specifies the profiles which the associated Representation(s) conform to of the list of Media Presentation profiles as described in 8. The value shall be a subset of the respective value in any higher level of the document hierarchy (Representation, Adaptation Set, MPD).<br><br>If not present, the value is inferred to be the same as in the next higher level of the document hierarchy. For example, if the value is not present for a Representation, then `@profiles` at the Adaptation Set level is valid for the Representation.<br><br>The same syntax as defined in 5.3.1.2 shall be used. |
| `@width` | O | specifies the horizontal visual presentation size of the video media type on a grid determined by the `@sar` attribute.<br><br>In the absence of `@sar` width and height are specified as if the value of `@sar` were "1:1"<br><br>NOTE The visual presentation size of the video is equal to the number of horizontal and vertical samples used for presentation after encoded samples are cropped in response to encoded cropping parameters, “overscan” signalling, or “pan/scan” display parameters, e.g. SEI messages.<br><br>If not present on any level, the value is unknown. |
| `@height` | O | specifies the vertical visual presentation size of the video media type, on a grid determined by the `@sar` attribute.<br><br>If not present on any level, the value is unknown. |
| `@sar` | O | specifies the sample aspect ratio of the video media component type, in the form of a string consisting of two integers separated by ':', e.g. "10:11". The first number specifies the horizontal size of the encoded video pixels (samples) in arbitrary units. The second number specifies the vertical size of the encoded video pixels (samples) in the same units as the horizontal size.<br><br>If not present on any level, the value is unknown. |
| <!--PB printed 64--> `@frameRate` | O | specifies the output frame rate (or in the case of  interlaced, half the output field rate) of the video media type in the Representation. If the frame or field rate is varying, the value is the average frame or half the average field rate field rate over the entire duration of the Representation.<br><br>The value is coded as a string, either containing two integers separated by a "/", ("F/D"), or a single integer "F". The frame rate is the division F/D, or F, respectively, per second (i.e. the default value of D is "1").<br><br>If not present on any level, the value is unknown. |
| `@audioSamplingRate` | O | Either a single decimal integer value specifying the sampling rate or a whitespace-separated pair of decimal integer values specifying the minimum and maximum sampling rate of the audio media component type. The values are in samples per second.<br><br>If not present on any level, the value is unknown. |
| `@mimeType` | M | specifies the MIME type of the concatenation of the Initialization Segment, if present, and all consecutive Media Segments in the Representation. |
| `@segmentProfiles` | O | specifies the profiles of Segments that are essential to process the Representation. The detailed semantics depend on the value of the @mimeType attribute.<br><br>The contents of this attribute shall conform to either the pro-simple or pro-fancy productions of 4.5 of IETF RFC 6381:2011, without the enclosing DQUOTE characters, i.e. including only the unencodedv or encodedv elements respectively. As profile identifier, the brand identifier for the Segment as defined in 6 shall be used.<br><br>4CC may contain characters that must be escaped in XML. 4CC strings shall be escaped according to W3C Extensible Markup Language (XML):2008, Section 2.4.<br><br>If not present on any level, the value may be deducted from the value of the @profiles attribute. |
| <!--PB printed 65--> `@codecs` | O | specifies the codecs present within the Representation. The codec parameters shall also include the profile and level information where applicable.<br><br>For segment formats defined in this document, this element shall be present and the contents of this attribute shall conform to either the simp-list or fancy-list productions of IETF RFC 6381:2011, subclause [3.2](03-terms.md#c3-2), without the enclosing DQUOTE characters. The codec identifier for the Representation's media format, mapped into the name space for codecs as specified in IETF RFC 6381:2011, subclause [3.3](03-terms.md#c3-3), shall be used. |
| `@containerProfiles` | O | specifies the container profiles of Representations that are essential to process it. The detailed semantics depend on the value of the @mimeType attribute.<br><br>The contents of this attribute shall conform to either the pro-simple or pro-fancy productions ofIETF RFC 6381:2011, subclause [4.5](04-overview.md#c4-5), without the enclosing DQUOTE characters, i.e. including only the unencodedv or encodedv elements respectively.<br><br>4CC may contain characters that must be escaped in XML. 4CC strings shall be escaped according to W3C Extensible Markup Language (XML):2008, Section 2.4. |
| `@maximumSAPPeriod` | O | when present, specifies the maximum SAP interval in seconds of all contained media streams, where the SAP interval is the maximum time interval between the T<sub>SAP</sub> of any two successive SAPs of types 1 to 3 inclusive of one media stream in the associated Representations.<br><br>If not present on any level, the value is unknown. |
| `@startWithSAP` | O | when present and greater than 0, specifies that in the associated Representations, each Media Segment starts with a SAP of type less than or equal to the value of this attribute value in each media stream.<br><br>A Media Segment starts with a SAP in a media stream if the stream contains a SAP in that Media Segment, I<sub>SAU</sub> is the index of the first access unit that follows I<sub>SAP</sub> and I<sub>SAP</sub> is contained in the Media Segment.<br><br>If not present on any level, the value is unknown. |
| <!--PB printed 66--> `@maxPlayoutRate` | O | specifies the maximum playout rate as a multiple of the regular playout rate, which is supported with the same decoder profile and level requirements as the normal playout rate.<br><br>If not present on any level, the value is 1. |
| `@codingDependency` | O | When present and 'true', for all contained media streams, specifies that there is at least one access unit that depends on one or more other access units for decoding. When present and 'false', for any contained media stream, there is no access unit that depends on any other access unit for decoding (e.g. for video all the pictures are intra coded). If not specified on any level, there may or may not be coding dependency between access units. |
| `@scanType` | O | specifies the scan type of the source material of the video media component type. The value may be equal to one of "progressive", "interlaced" and "unknown". If not specified on any level, the scan type is "progressive". |
| `@selectionPriority` | OD<br>default=1 | specifies the selection priority for the described data structures, i.e. the one described by the containing element. In the absence of other information, higher numbers are the preferred selection over lower numbers. |
| `@tag` | O | specifies the tag of the Representation, Adaptation Set or Preselection which may be used for selection purposes towards the decoder.<br>NOTE This attribute is primarily introduced for the usage of Pre-Selections and Adaptation Sets, but future use for Representation and Sub-Representations is not precluded. |
| **FramePacking** | 0 … N | specifies frame-packing arrangement information of the video media component type.<br><br>When no FramePacking element is provided for a video component, frame-packing shall not used for the video media component.<br><br>For details, see subclauses 5.8.1 and 5.8.4.6. |
| **AudioChannelConfiguration** | 0 … N | specifies the audio channel configuration of the audio media component type.<br><br>For details, see subclauses 5.8.1 and 5.8.4.7. |
| **ContentProtection** | 0 … N | specifies information about content protection schemes used for the associated Representations.<br><br>For details, see subclauses 5.8.1 and 5.8.4.1. |
| <!--PB printed 67--> **OutputProtection** | 0 … 1 | specifies information about output protection schemes required for presenting the associated Representations<br><br>For details, see subclauses 5.8.4.12. |
| **EssentialProperty** | 0 … N | specifies information about the containing element that is considered essential by the Media Presentation author for processing the containing element.<br><br>For details, see subclause 5.8.4.8. |
| **SupplementalProperty** | 0 … N | specifies supplemental information about the containing element that may be used by the DASH Client optimizing the processing.<br><br>For details, see subclause 5.8.4.9. |
| **InbandEventStream** | 0 … N | specifies the presence of an inband event stream in the associated Representations.<br><br>For details, refer to subclause 5.10. |
| **Switching** | 0 … N | Specifies a switch-to times and types for the associated Representations. For more details, refer to subclause 0.<br><br>These elements shall only be present if the @timescale value is the same for all Representations in one Adaptation Set and if the Segment Timeline is used for segment duration signalling. |
| **RandomAccess** | 0 … N | Specifies a random access times and types for the associated Representations. For more details, refer to subclause [5.3.5.5](05-3-5-representation.md#c5-3-5-5).<br><br>These elements shall only be present if the @timescale value is the same for all Representations in one Adaptation Set and if the Segment Timeline is used for segment duration signalling. |
| **GroupLabel** | 0 … N | specifies a summary label for a group of Labels. For more details, refer to subclause 5.3.10. |
| **Label** | 0 … N | specifies a textual description of the element that may be used for annotation and selection purposes. For more details, refer to subclause 5.3.10. |
| **ProducerReferenceTime** | 0 … N | specifies the presence of and possibly values of producer reference time in the associated Representations.<br><br>For details refer to subclause 5.12. |
| <!--PB printed 68--> **ContentPopularityRate** | 0 … N | indicates a level of popularity of the containing entity (i.e., the Adaptation Set, Representation or Preselection) within the Media Presentation. For details, see subclause 5.14.<br><br>NOTE This element is primarily introduced for the usage of Pre-Selections and Adaptation Sets but use for Representation and Sub-Representations is not precluded. |
| **Resync** | 0 … N | Specifies information on Segments’ resynchronization points.<br><br>For details refer to subclause 5.3.12. |  

Key

For attributes: M=mandatory, O=optional, OD=optional with default value, CM=conditionally mandatory

For elements: <minOccurs>..<maxOccurs> (N=unbounded)

Elements are bold; attributes are non-bold and preceded with an @.

<a id="c5-3-7-3"></a>

#### 5.3.7.3 XML syntax

```xml
<xs:complexType name="RepresentationBaseType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Representation base (common attributes and elements)</xs:documentation>
    </xs:annotation>
    <xs:sequence>
        <xs:element name="FramePacking" type="DescriptorType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="AudioChannelConfiguration" type="DescriptorType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="ContentProtection" type="ContentProtectionType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="OutputProtection" type="DescriptorType" minOccurs="0"/>
        <xs:element name="EssentialProperty" type="DescriptorType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="SupplementalProperty" type="DescriptorType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="InbandEventStream" type="EventStreamType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="Switching" type="SwitchingType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="RandomAccess" type="RandomAccessType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="GroupLabel" type="LabelType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="Label" type="LabelType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="ProducerReferenceTime" type="ProducerReferenceTimeType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="ContentPopularityRate" type="ContentPopularityRateType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="Resync" type="ResyncType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:any namespace="##other" processContents="lax" minOccurs="0" maxOccurs="unbounded"/>
    </xs:sequence>
    <xs:attribute name="profiles" type="ListOfProfilesType"/>
    <xs:attribute name="width" type="xs:unsignedInt"/>
    <xs:attribute name="height" type="xs:unsignedInt"/>
    <xs:attribute name="sar" type="RatioType"/>
    <xs:attribute name="frameRate" type="FrameRateType"/>
    <xs:attribute name="audioSamplingRate" type="AudioSamplingRateType"/>
    <xs:attribute name="mimeType" type="xs:string"/>
    <xs:attribute name="segmentProfiles" type="ListOf4CCType"/>
    <xs:attribute name="codecs" type="CodecsType"/>
    <xs:attribute name="containerProfiles" type="ListOf4CCType"/>
    <xs:attribute name="maximumSAPPeriod" type="xs:double"/>
    <xs:attribute name="startWithSAP" type="SAPType"/>
    <xs:attribute name="maxPlayoutRate" type="xs:double"/>
    <xs:attribute name="codingDependency" type="xs:boolean"/>
    <xs:attribute name="scanType" type="VideoScanType"/>
    <xs:attribute name="selectionPriority" type="xs:unsignedInt" default="1"/>
    <xs:attribute name="tag" type="TagType"/>
    <xs:anyAttribute namespace="##other" processContents="lax"/>
</xs:complexType>
<xs:simpleType name="AudioSamplingRateType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Audio Sampling Rate</xs:documentation>
    </xs:annotation>
    <xs:restriction base="UIntVectorType">
        <xs:minLength value="1"/>
        <xs:maxLength value="2"/>
    </xs:restriction>
</xs:simpleType>
<xs:simpleType name="VideoScanType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Video Scan type enumeration</xs:documentation>
    </xs:annotation>
    <xs:restriction base="xs:string">
        <xs:enumeration value="progressive"/>
        <xs:enumeration value="interlaced"/>
        <xs:enumeration value="unknown"/>
    </xs:restriction>
</xs:simpleType>
<xs:simpleType name="CodecsType">
    <xs:restriction base="xs:string">
        <xs:pattern value="&charset;&squote;&language;&squote;&id_list;">
            <xs:annotation>
                <xs:documentation>RFC6381 fancy-list without enclosing double quotes</xs:documentation>
            </xs:annotation>
        </xs:pattern>
        <xs:pattern value="&id_simple;(,&id_simple;)*">
            <xs:annotation>
                <xs:documentation>RFC6381 simp-list without enclosing double quotes</xs:documentation>
            </xs:annotation>
        </xs:pattern>
    </xs:restriction>
</xs:simpleType>
```
<!--PB printed 69-->
<!--(page breaks fell inside the XML block above)-->

## Transcription notes

> **Spec note:** Table 14 `Switching`: "For more details, refer to subclause 0." - broken cross-reference, reproduced as printed.
>
> **Spec note:** Table 14 `@containerProfiles`: "productions ofIETF RFC 6381:2011" (missing space) as printed.
>
> **Spec note:** Table 14 `@width`/`@height`/`@sar` rows are analysed with ETSI TS 103 285 in [`dash-23009-1-2022-aspect.md`](dash-23009-1-2022-aspect.md); this file is authoritative for the table text.
