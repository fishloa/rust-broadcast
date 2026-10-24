---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Clause 5.3 intro and 5.3.1 - Hierarchical data model, MPD element"
pdf_pages: "34-41"
printed_pages: "26-33"
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION.md
---

<a id="c5-3"></a>

## 5.3 Hierarchical data model

<a id="c5-3-1"></a>

### 5.3.1 General

<a id="c5-3-1-1"></a>

#### 5.3.1.1 Overview

A Media Presentation as described in the MPD consists of a sequence of one or more Periods as described in subclause [5.3.2.](05-3-2-period.md#c5-3-2)

- Each Period contains one or more Adaptation Sets as described in subclause [5.3.3.](05-3-3-adaptation-set.md#c5-3-3) In case an Adaptation Set contains multiple media content components, then each media content component is described individually as defined in subclause [5.3.4.](05-3-4-content-component.md#c5-3-4)
- Each Adaptation Set contains one or more Representations as described in subclause [5.3.5.](05-3-5-representation.md#c5-3-5)
- Adaptation Sets, Representations and Sub-Representations share common attributes and elements that are described in subclause [5.3.7.](05-3-7-common.md#c5-3-7)
- Preselections may be used to select experiences as defined in subclause 5.3.11. Each Preselection references to one or more media content components within one or multiple Adaptation Sets.

<!--PB printed 26-->

- Each Period may contain one or more Subsets that restrict combination of Adaptation Sets for presentation. Subsets are described in subclause [5.3.8.](05-3-8-subsets.md#c5-3-8)
- Each Representation consists of one or more Segments described in sublause 5.15. Segment Information is introduced in subclause [5.3.9.](05-3-9-segment-information.md#c5-3-9) Segments contain media data and/or metadata to access, decode and present the included media content. Representations may also include Sub-Representations as defined in subclause [5.3.5.6](05-3-5-representation.md#c5-3-5-6) to describe and extract partial information from a Representation.
- Each Segment consists of one or more Subsegments. Subsegments are described in subclause 6.2.3.2.
- Labels for annotation of different hierarchies are defined in subclause 5.3.10.

The summary of the semantics of the attributes and elements within an MPD element are provided in

Table 3, subclause [5.3.1.2.](#c5-3-1-2) The XML syntax of the MPD element is provided in subclause [5.3.1.3.](#c5-3-1-3)

<a id="c5-3-1-2"></a>

#### 5.3.1.2 Semantics

Table 3 — Semantics of MPD element

| Element or Attribute Name | Use | Description |
|---|---|---|
| **MPD** |  | The root element that carries the Media Presentation Description for a Media Presentation. |
| `@id` | O | specifies an identifier for the Media Presentation. It is recommended to use an identifier that is unique within the scope in which the Media Presentation is published.<br><br>If not specified, no MPD-internal identifier is provided. However, for example the URL to the MPD may be used as an identifier for the Media Presentation. |
| `@profiles` | M | specifies a list of Media Presentation profiles as described in 8.<br><br>The contents of this attribute shall conform to either the pro-simple or pro-fancy productions of IETF RFC 6381:2011, Section 4.5, without the enclosing DQUOTE characters, i.e. including only the unencodedv or encodedv elements respectively.<br><br>As profile identifier a restricted URI format as defined in 8.1 shall be used. |
| <!--PB printed 27--> `@type` | OD<br>default: `static` | specifies the type of the Media Presentation. For static Media Presentations (`@type="static"`), all Segments are available between the `@availabilityStartTime` and the `@availabilityEndTime`. For dynamic Media Presentations (`@type="dynamic"`), Segments typically have different availability times. For details, refer to subclause [5.3.9.5.3.](05-3-9-5-segment-information-derivation.md#c5-3-9-5-3)<br><br>In addition, the Media Presentation Description may be updated in dynamic Media Presentations, i.e. the `@minimumUpdatePeriod` may be present.<br><br>NOTE Static Media Presentations are typically used for On-Demand services, whereas dynamic Media Presentations are used for live services. |
| `@availabilityStartTime` | CM<br>shall be present for `@type='dynamic'` | For `@type='dynamic'`, this attribute shall be present. In this case, it specifies the anchor for the computation of the earliest availability time (in UTC) for any Segment in the Media Presentation.<br><br>For `@type=static`” if present, it specifies the Segment availability start time for all Segments referred to in this MPD. If not present, all Segments described in the MPD shall become available at the time the MPD becomes available. |
| `@publishTime` | OD<br>shall be present for `@type='dynamic'` | specifies the wall-clock time when the MPD was generated and published at the origin server. MPDs with a later value of `@publishTime` shall be an update as defined in subclause 5.4 to MPDs with earlier `@publishTime`. |
| `@availabilityEndTime` | O | specifies the latest Segment availability end time for any Segment in the Media Presentation. When not present, the value is unknown. |
| `@mediaPresentationDuration` | O | specifies the duration of the entire Media Presentation. If the attribute is not present, the duration of the Media Presentation is unknown.<br><br>This attribute shall be present when neither the attribute MPD@minimumUpdatePeriod nor the Period@duration of the last Period are present. |
| <!--PB printed 28--> `@minimumUpdatePeriod` | O | If this attribute is present, it specifies the smallest period between potential changes to the MPD. This can be useful to control the frequency at which a client checks for updates.<br><br>From a client perspective, after a client fetches an MPD, it specifies the minimum period during which the MPD remains valid. Validity is defined in subclause 5.4.<br><br>If this attribute is not present, it indicates that the MPD does not change.<br><br>If MPD@type is not 'dynamic', @minimumUpdatePeriod shall not be present.<br><br>Details on the use of the value of this attribute are specified in subclause 5.4. |
| `@minBufferTime` | M | specifies a common duration used in the definition of the Representation data rate (see @bandwidth attribute in subclauses [5.3.5.2](05-3-5-representation.md#c5-3-5-2) and [5.3.5.4](05-3-5-representation.md#c5-3-5-4)). |
| `@timeShiftBufferDepth` | O | specifies the duration of the smallest time shifting buffer for any Representation in the MPD that is guaranteed to be available for a Media Presentation with type 'dynamic'. When not present, the value is infinite. This value of the attribute is undefined if the type attribute is equal to 'static'. |
| `@suggestedPresentationDelay` | O | When @type is 'dynamic', it specifies a fixed delay offset in time from the presentation time of each access unit that is suggested to be used for presentation of each access unit. For more details, refer to subclause 7.2.1. When not specified, then no value is provided and the client is expected to choose a suitable value.<br><br>When @type is 'static'the value of the attribute is undefined and may be ignored. |
| `@maxSegmentDuration` | O | specifies the maximum duration of any Segment in any Representation in the Media Presentation, i.e. documented in this MPD and any future update of the MPD. If not present, then the maximum Segment duration shall be the maximum duration of any Segment documented in this MPD. |
| `@maxSubsegmentDuration` | O | specifies the maximum duration of any Media Subsegment in any Representation in the Media Presentation. If not present, the same value as for the maximum Segment duration is implied. |
| <!--PB printed 29--> **ProgramInformation** | 0…N | specifies descriptive information about the program. For more details, refer to the description in subclause 5.7. |
| **BaseURL** | 0…N | specifies a Base URL that can be used for reference resolution and alternative URL selection. For more details, refer to the description in subclause 5.6. |
| **Location** | 0…N | specifies a location at which the MPD is available.<br><br>A reference processing model is provided in Annex A.11. |
| **PatchLocation** | 0 ... N | specifies a location at which the MPD patch document is available. Details on the MPD patch document, this element, and expected processing models are available in subclause 5.15.<br><br>If this element is present, the MPD@id attribute and the MPD@publishTime shall be present.<br><br>When @type is 'static'or the @minimumUpdatePeriod attribute is not present, then value of the element is undefined and may be ignored.<br><br>If this element is not present, no MPD patch document is available.<br><br>If multiple elements are present, any PatchLocation element may be used. |
| **ServiceDescription** | 0 ... N | specifies the service description detailing how the service provider expects the service is to be consumed.<br><br>It shall provide a valid service description as described by Annex K.3 and Annex K.4.<br><br>For details refer to Annex K. |
| **InitializationSet** | 0 ... N | specifies a suitable initialization for a specific media type for the presentation. If present, at least one Period of the Media Presentation shall include at least one Adaptation Set that can be played when initialized by this Initialization Set.<br><br>For details see subclause 5.3.12. |
| **InitializationGroup** | 0 ... N | Specifies a white space separated list of ids of Initialization Sets of the same content type. This indicates that any Period in the Media Presentation has at least one Adaptation Set that conforms to one of the Initialization Sets referenced in this element.<br><br>For details see subclause 5.3.12. |
| <!--PB printed 30--> `@id` | M | specifies an unsigned integer identifier for this Initialization Group. The attribute shall be unique unique among all Initialization Set and Initialization Group ids in the MPD. |
| `@profiles` | O | specifies the profiles which the listed Initialization Groups conform to. The value shall be a subset of the value in the MPD@profiles attribute.<br><br>If not present the value is inferred to be the same as the MPD@profiles attribute.<br><br>The same syntax is used as defined for the MPD@profiles attribute. |
| `@contentType` | O | specifies the content type of Initialization Sets listed in the element. |
| **InitializationPresentation** | 0 ... N | specifies a white space separated list of ids of Initialization Sets and Initialization Groups to indicate a combination which creates a complete presentation. A client supporting all listed Initialization Sets and Initialization Groups of an Initialization Presentation is expected to be able to play the entire Media Presentation as intended by the service provider.<br><br>For details see subclause 5.3.12. |
| `@id` | M | specifies a unique unsigned integer identifier for this Initialization Presentation. The attribute shall be unique among all Initialization Presentation ids in the MPD. |
| `@profiles` | O | specifies the profiles which the listed Initialization Groups conform to. The value shall be a subset of the value in the MPD@profiles attribute.<br><br>If not present the value is inferred to be the same as the MPD@profiles attribute.<br><br>The same syntax is used as defined for the MPD@profiles attribute. |
| `@contentType` | O | specifies the content type of initialization Groups listed in the element. |
| **ContentProtection** | 0 ... N | specifies information about content protection and encryption schemes used in this Media Presentation. If present on this level, it shall include the @refId attribute.<br><br>For details, see subclauses 5.8.1 and 5.8.4.1. |
| **Period** | 1…N | specifies the information of a Period. For more details, refer to the description in subclause [5.3.2.](05-3-2-period.md#c5-3-2) |
| <!--PB printed 31--> **Metrics** | 0... N | specifies the DASH Metrics.<br><br>For more details, see subclause 5.9. |
| **EssentialProperty** | 0 … N | specifies information about the containing element that is considered essential by the Media Presentation author for processing the containing element.<br><br>For details, see subclause 5.8.4.8. |
| **SupplementalProperty** | 0 … N | specifies supplemental information about the containing element that may be used by the DASH Client optimizing the processing.<br><br>For details, see subclause 5.8.4.9. |
| **UTCTiming** | 0... N | specifies information on ways to obtain a synchronization to wall-clock time as used in this Media Presentation. The order of the elements expresses a preference of choice by the Media Presentation author. For more details, refer to subclause 5.8.4.11. |
| **LeapSecondInformation** | 0... 1 | specifies leap second information affecting MPD timing calculations.<br><br>For details refer to clause 5.13. |


Key

For attributes: M=mandatory, O=optional, OD=optional with default value, CM=conditionally mandatory

For elements: <minOccurs>…<maxOccurs> (N=unbounded)

Elements are bold; attributes are non-bold and preceded with an @.

<a id="c5-3-1-3"></a>

#### 5.3.1.3 XML syntax

```xml
<xs:complexType name="MPDtype">
    <xs:annotation>
        <xs:documentation xml:lang="en">MPD Type</xs:documentation>
    </xs:annotation>
    <xs:sequence>
        <xs:element name="ProgramInformation" type="ProgramInformationType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="BaseURL" type="BaseURLType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="Location" type="xs:anyURI" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="PatchLocation" type="PatchLocationType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="ServiceDescription" type="ServiceDescriptionType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="InitializationSet" type="InitializationSetType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="InitializationGroup" type="UIntVWithIDType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="InitializationPresentation" type="UIntVWithIDType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="ContentProtection" type="ContentProtectionType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="Period" type="PeriodType" maxOccurs="unbounded"/>
        <xs:element name="Metrics" type="MetricsType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="EssentialProperty" type="DescriptorType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="SupplementalProperty" type="DescriptorType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="UTCTiming" type="DescriptorType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="LeapSecondInformation" type="LeapSecondInformationType" minOccurs="0"/>
        <xs:any namespace="##other" processContents="lax" minOccurs="0" maxOccurs="unbounded"/>
    </xs:sequence>
    <xs:attribute name="id" type="xs:string"/>
    <xs:attribute name="profiles" type="ListOfProfilesType" use="required"/>
    <xs:attribute name="type" type="PresentationType" default="static"/>
    <xs:attribute name="availabilityStartTime" type="xs:dateTime"/>
    <xs:attribute name="availabilityEndTime" type="xs:dateTime"/>
    <xs:attribute name="publishTime" type="xs:dateTime"/>
    <xs:attribute name="mediaPresentationDuration" type="xs:duration"/>
    <xs:attribute name="minimumUpdatePeriod" type="xs:duration"/>
    <xs:attribute name="minBufferTime" type="xs:duration" use="required"/>
    <xs:attribute name="timeShiftBufferDepth" type="xs:duration"/>
    <xs:attribute name="suggestedPresentationDelay" type="xs:duration"/>
    <xs:attribute name="maxSegmentDuration" type="xs:duration"/>
    <xs:attribute name="maxSubsegmentDuration" type="xs:duration"/>
    <xs:anyAttribute namespace="##other" processContents="lax"/>
</xs:complexType>
<xs:simpleType name="PresentationType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Presentation Type enumeration</xs:documentation>
    </xs:annotation>
    <xs:restriction base="xs:string">
        <xs:enumeration value="static"/>
        <xs:enumeration value="dynamic"/>
    </xs:restriction>
</xs:simpleType>
<xs:complexType name="UIntVWithIDType">
    <xs:annotation>
        <xs:documentation xml:lang="en">UInt Vector With ID</xs:documentation>
    </xs:annotation>
    <xs:simpleContent>
        <xs:extension base="UIntVectorType">
            <xs:attribute name="id" type="xs:unsignedInt" use="required"/>
            <xs:attribute name="profiles" type="ListOfProfilesType"/>
            <xs:attribute name="contentType" type="RFC6838ContentTypeType"/>
            <xs:anyAttribute namespace="##other" processContents="lax"/>
        </xs:extension>
    </xs:simpleContent>
</xs:complexType>
<xs:simpleType name="UIntVectorType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Whitespace-separated list of unsigned integers</xs:documentation>
    </xs:annotation>
    <xs:list itemType="xs:unsignedInt"/>
</xs:simpleType>
<xs:simpleType name="ListOfProfilesType">
    <xs:annotation>
        <xs:documentation xml:lang="en">List of Profiles</xs:documentation>
    </xs:annotation>
    <xs:restriction base="xs:string">
        <xs:pattern value="(&URN;|&URL;)(&comma_sep;(&URN;|&URL;))*"/>
    </xs:restriction>
</xs:simpleType>
```
<!--PB printed 32-->
<!--(page breaks fell inside the XML block above)-->

## Transcription notes

> **Spec note:** 5.3.1.1: "sublause 5.15" is a typo in the PDF, reproduced as printed.
>
> **Spec note:** Table 3 `@type`: the second sentence of `@availabilityStartTime` reads `For @type=static" if present` (mismatched closing quote) as printed. `@publishTime` carries Use "OD" with the note "shall be present for @type='dynamic'" and no default, as printed.
>
> **Spec note:** Table 3 Use cell for `@type`: "OD, default: static". Page-break markers inside tables sit in the first cell of the first row that starts after the break.
>
> **Spec note:** The normative derivation formulas for availabilityStartTime / timeShiftBufferDepth / minimumUpdatePeriod / suggestedPresentationDelay / minBufferTime are NOT in 5.3.1.2 (Table 3 gives semantics only); they live in clause 7.2.1 and Annex A, outside this transcription range. See VERIFICATION.md GAPS.
