---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Clause 5.3.3 - Adaptation Sets"
pdf_pages: "47-58"
printed_pages: "39-50"
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION.md
---

<a id="c5-3-3"></a>

### 5.3.3 Adaptation Sets

<a id="c5-3-3-1"></a>

#### 5.3.3.1 Overview

Each Period consists of one or more Adaptation Sets. An Adaptation Set is described by an AdaptationSet element. AdaptationSet elements are contained in a Period element.

An Adaptation Set contains alternate Representations, i.e. only one Representation within an Adaptation Set is expected to be presented at a time. All Representations contained in one Adaptation Set represent the same media content components and therefore contain media streams that are considered to be perceptually equivalent. The Adaptation Set and the contained Representations shall be prepared and contain sufficient information such that seamless switching (as defined in subclause [4.5.1](04-overview.md#c4-5-1)) across different Representations in one Adaptation Set is enabled. If an Adaptation Set is expected to be consumed by DASH Clients with restrictions in terms of switching, then the Media Presentation author should provide sufficient means to enable seamless switching under these restrictions.

<!--PB printed 39-->

Representations are arranged into Adaptation Sets according to the media content component properties of the media content components present in the Representations, namely:

- the language as described by the @lang attribute,
- the media content component type described by the @contentType attribute,
- the picture aspect ratio as described by the @par attribute,
- the role property as described by the Role elements,
- the accessibility property as described by the Accessibility elements,
- the viewpoint property as described by the Viewpoint elements,
- the rating property as described by the Rating elements.

Representations shall appear in the same Adaptation Set if and only if they have identical values for all of these media content component properties for each media content component.

The values for the elements Role, Accessibility, Viewpoint and Rating are generally not provided within the scope of this document. However, a number of simple schemes are defined in subclause 5.8.5.

If there exist multiple media content components, then the properties of each media content component shall be described by a separate ContentComponent element as defined in subclause [5.3.4](05-3-4-content-component.md#c5-3-4). The ContentComponent element shares common elements and attributes with the AdaptationSet element. Default values, or values applicable to all media content components, may be provided directly in the AdaptationSet element. Attributes present in the AdaptationSet shall not be repeated in the ContentComponent element.

The AdaptationSet element may contain default values for elements and attributes associated to the contained Representations. The list of possible present elements and attributes that are common to AdaptationSet and Representation (and also SubRepresentation) are collected in subclause [5.3.7](05-3-7-common.md#c5-3-7). Any of the common attributes shall only be present either in the AdaptationSet element or in the Representation element, but not in both.

The AdaptationSet element also supports the description of ranges for the @bandwidth, @width, @height and @frameRate attributes associated to the contained Representations, which provide a summary of all values for all the Representations within this Adaptation Set. The Representations contained within an Adaptation Set shall not contain values outside the ranges documented for that Adaptation Set.

Adaptation Sets may be further arranged into groups using the @group attribute. The semantics of this grouping is that the media content within one Period is represented by:

1) either one Representation from group 0, if present,

2) or the combination of at most one Representation from each non-zero group.

<!--PB printed 40-->

If the AdaptationSet@group attribute is not present, then all Representations in this Adaptation Set are assigned to a non-zero group specific to this Adaptation Set.

The semantics of the attributes and elements within an AdaptationSet element are provided in Table 5 in subclause [5.3.3.2](#c5-3-3-2). The XML syntax of the AdaptationSet element is provided in subclause [5.3.3.3](#c5-3-3-3).

<a id="c5-3-3-2"></a>

#### 5.3.3.2 Semantics

Table 5 — Semantics of AdaptationSet element

| Element or Attribute Name | Use | Description |
|---|---|---|
| **AdaptationSet** |  | Adaptation Set description. |
| `@xlink:href` | O | specifies a reference to a remote element entity that shall contain exactly one element of type **AdaptationSet**. |
| `@xlink:actuate` | OD<br>default: `'onRequest'` | specifies the processing instructions, which can be either "onLoad" or "onRequest". |
| `@id` | O | specifies a unique identifier for this Adaptation Set in the scope of the Period. The attribute shall be a unique unsigned integer value in the scope of the containing Period.<br><br>The attribute shall not be present in a remote element entity.<br><br>If not present, no identifier for the Adaptation Set is specified. |
| `@group` | O | specifies an identifier for the group that is unique in the scope of the containing Period. The value is an unsigned integer.<br><br>For details, refer to subclause [5.3.3.1](#c5-3-3-1). |
| ***CommonAttributesElements*** | — | specifies the common attributes and elements (attributes and elements from base type ***RepresentationBaseType***). For details, see subclause [5.3.7](05-3-7-common.md#c5-3-7). |
| `@lang` | O | Declares the language code for this Adaptation Set. The syntax and semantics according to IETF RFC 5646 shall be used.<br><br>If not present, the language code may be defined for each media component or it may be unknown.<br><br>If the language is unknown, the 'und' code for undetermined primary language or the 'zxx' (Non-Linguistic, Not Applicable) code can be used. |
| <!--PB printed 41--> `@contentType` | O | specifies the media content component type for this Adaptation Set. A value of the top-level Content-type 'type' value as defined in 4 of IETF RFC 6838:2013 shall be taken.<br><br>If not present, the media content component type may be defined for each media content component or it may be unknown. |
| `@par` | O | specifies the picture aspect ratio of the video media component type, in the form of a string consisting of two integers separated by ':', e.g. "16:9". When this attribute is present, and the attributes @width and @height for the set of Representations are also present, the picture aspect ratio as specified by this attribute shall be the same as indicated by the values of @width, @height, and @sar, i.e. it shall express the same ratio as (@width * sarx): (@height * sary), with sarx the first number in @sar and sary the second number.<br><br>If not present, the picture aspect ratio may be defined for each media component or it may be unknown. |
| `@minBandwidth` | O | specifies the minimum @bandwidth value in all Representations in this Adaptation Set. This value has the same units as the @bandwidth attribute.<br><br>If not present, the value is unknown. |
| `@maxBandwidth` | O | specifies the maximum @bandwidth value in all Representations in this Adaptation Set. This value has the same units as the @bandwidth attribute.<br><br>If not present, the value is unknown. |
| `@minWidth` | O | specifies the minimum @width value in all Representations in this Adaptation Set. This value has the same units as the @width attribute.<br><br>If not present, the value is unknown. |
| `@maxWidth` | O | specifies the maximum @width value in all Representations in this Adaptation Set. This value has the same units as the @width attribute.<br><br>If not present, the value is unknown. |
| `@minHeight` | O | specifies the minimum @height value in all Representations in this Adaptation Set. This value has the same units as the @height attribute.<br><br>If not present, the value is unknown. |
| `@maxHeight` | O | specifies the maximum @height value in all Representations in this Adaptation Set. This value has the same units as the @height attribute.<br><br>If not present, the value is unknown. |
| <!--PB printed 42--> `@minFrameRate` | O | specifies the minimum @framerate value in all Representations in this Adaptation Set. This value is encoded in the same format as the @frameRate attribute.<br><br>If not present, the value is unknown. |
| `@maxFrameRate` | O | specifies the maximum @framerate value in all Representations in this Adaptation Set. This value is encoded in the same format as the @frameRate attribute.<br><br>If not present, the value is unknown. |
| `@segmentAlignment` | OD<br>default: `false` | When set to `'true'`, this specifies that for any two Representations, X and Y, within the same Adaptation Set, the *m*-th Segment of X and the *n*-th Segment of Y are non-overlapping (as defined in subclause [4.5.3](04-overview.md#c4-5-3)) whenever *m* is not equal to *n*.<br><br>For Adaptation Sets containing Representations with multiple media content components, this attribute value shall be either `'true'` or `'false'`.<br><br>NOTE Previous editions permitted non-boolean values for this attribute. As this was not used in practice and caused confusion, it is deprecated in this edition. |
| <!--PB printed 43--> `@bitstreamSwitching` | O | When this flag is set to `'true'`, the following applies:<br>— All Representations in the Adaptation Set shall have the same number *M* of Media Segments;<br>— Let *R<sub>1</sub>*, *R<sub>2</sub>*, ..., *R*<sub>N</sub> be all the Representations within the Adaptation Set.<br>— Let<br>&emsp;— *S<sub>i,j</sub>*, for *j* > 0, be the *j<sup>th</sup>* Media Segment in the *i<sup>th</sup>* Representation (i.e. *R<sub>i</sub>*)<br>&emsp;— if present, let *S<sub>i,0</sub>* be the Initialization Segment in the *i<sup>th</sup>* Representation, and<br>&emsp;— if present, let *B<sub>i</sub>* be the Bitstream Switching Segment in the *i<sup>th</sup>* Representation.<br>— The sequence of<br>&emsp;— any Initialization Segment, if present, in the Adaptation Set, with,<br>&emsp;— if Bitstream Switching Segments are present,<br>&emsp;&emsp;*B<sub>i(1)</sub>*, *S<sub>i(1),1</sub>*, *B<sub>i(2)</sub>*, *S<sub>i(2),2</sub>*, ..., *B<sub>i(k)</sub>*, *S<sub>i(k),k</sub>*, ..., *B<sub>i(M)</sub>*, *S<sub>i(M),M</sub>*<br>&emsp;— else<br>&emsp;&emsp;*S<sub>i(1),1</sub>*, *S<sub>i(2),2</sub>*, ..., *S<sub>i(k),k</sub>*, ..., *S<sub>i(M),M,</sub>*<br>&emsp;&emsp;wherein any *i*(*k*) for all *k* values in the range of 1 to *M*, respectively, is an integer value in the range of 1 to *N*,<br>results in a "conforming Segment track" as defined in subclause [4.5.4](04-overview.md#c4-5-4) with the media format as specified in the `@mimeType` attribute.<br><br>More detailed rules may be defined for specific media formats.<br><br>For more details, refer to subclause 0.<br><br>NOTE When this attribute is set to TRUE, then seamless switching across Representations can be achieved without re-initialization of the decoder. Content authors are encouraged to set this attribute to true only if the media content components across Representations do not need the media decoder to be re-initialized. |
| <!--PB printed 44--> `@subsegmentAlignment` | OD<br>default: `false` | If the `@subsegmentAlignment` for an Adaptation Set is set to `'true'`, all following conditions shall be satisfied:<br>— Each Media Segment shall be indexed (i.e. either it contains a Segment index or there is an Index Segment providing an index for the Media Segment).<br>— For any two Representations, X and Y, within the same Adaptation Set, the *m*-th Subsegment of X and the *n*-th Subsegment of Y are non-overlapping (as defined in subclause [4.5.3](04-overview.md#c4-5-3)) whenever *m* is not equal to *n*.<br><br>NOTE Previous editions permitted non-boolean values for this attribute. As this was not used in practice and caused confusion, it is deprecated in this edition. |
| `@subsegmentStartsWithSAP` | OD<br>default: `0` | when greater than 0, specifies that each Subsegment with `SAP_type` greater than 0 starts with a SAP of type less than or equal to the value of `@subsegmentStartsWithSAP`. A Subsegment starts with SAP when the Subsegment contains a SAP, and for the first SAP, I<sub>SAU</sub> is the index of the first access unit that follows I<sub>SAP</sub>, and I<sub>SAP</sub> is contained in the Subsegment.<br><br>The semantics of `@subsegmentStartsWithSAP` equal to 0 are unspecified. |
| `@initializationSetRef` | O | specifies a white space separated list of Initialization Set identifiers. The Adaptation Set shall conform to all Initialization Sets that are referenced in this attribute. |
| `@initializationPrincipal` | O | specifies the URL of an Initialization Segment that is sufficient to initialize the Adaptation Set. If not present, an Initialization Segment from one of the Representations is sufficient. |
| **Accessibility** | 0 … N | specifies information about accessibility scheme<br><br>For more details, refer to subclauses 5.8.1 and 5.8.4.3. |
| **Role** | 0 … N | specifies information on role annotation scheme<br><br>For more details, refer to subclauses 5.8.1 and 5.8.4.2. |
| **Rating** | 0 … N | specifies information on rating scheme.<br><br>For more details, refer to subclauses 5.8.1 and 5.8.4.4. |
| **Viewpoint** | 0 … N | specifies information on viewpoint annotation scheme.<br><br>For more details, refer to subclauses 5.8.1 and 5.8.4.5. |
| **ContentComponent** | 0…N | specifies the properties of one media content component contained in this Adaptation Set.<br><br>For more details, refer to subclause [5.3.4](05-3-4-content-component.md#c5-3-4). |
| **BaseURL** | 0…N | specifies a base URL that can be used for reference resolution and alternative URL selection. For more details, refer to the description in subclause 5.6. |
| **SegmentBase** | 0...1 | specifies default Segment Base information.<br><br>Information in this element is overridden by information in the Representation.SegmentBase, if present.<br><br>For more details, see subclause [5.3.9](05-3-9-segment-information.md#c5-3-9). |
| **SegmentList** | 0...1 | specifies default Segment List information.<br><br>Information in this element is overridden by information in the Representation.SegmentList, if present.<br><br>For more details, see subclause [5.3.9](05-3-9-segment-information.md#c5-3-9). |
| <!--PB printed 45--> **SegmentTemplate** | 0...1 | specifies default Segment Template information.<br><br>Information in this element is overridden by information in the Representation.SegmentTemplate, if present.<br><br>For more details, see subclause [5.3.9](05-3-9-segment-information.md#c5-3-9). |
| **Representation** | 0 … N | specifies a Representation.<br><br>At least one Representation element shall be present in each Adaptation Set. The actual element may however be part of a remote element entity if xlink is used on the containing AdaptationSet element.<br><br>For more details, refer to subclause [5.3.5](05-3-5-representation.md#c5-3-5). |



Key

For attributes: M=mandatory, O=optional, OD=optional with default value, CM=conditionally mandatory, F=fixed

For elements: <minOccurs>...<maxOccurs> (N=unbounded)

The conditions only hold without using xlink:href. If linking is used, then all attributes are "optional" and <minOccurs=0>.

Elements are bold; attributes are non-bold and preceded with an @; list of elements and attributes is in italics bold referring to those taken from the Base type that has been extended by this type.

<a id="c5-3-3-3"></a>

#### 5.3.3.3 XML syntax

```xml
<xs:complexType name="AdaptationSetType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Adaptation Set</xs:documentation>
    </xs:annotation>
    <xs:complexContent>
        <xs:extension base="RepresentationBaseType">
            <xs:sequence>
                <xs:element name="Accessibility" type="DescriptorType" minOccurs="0" maxOccurs="unbounded"/>
                <xs:element name="Role" type="DescriptorType" minOccurs="0" maxOccurs="unbounded"/>
                <xs:element name="Rating" type="DescriptorType" minOccurs="0" maxOccurs="unbounded"/>
                <xs:element name="Viewpoint" type="DescriptorType" minOccurs="0" maxOccurs="unbounded"/>
                <xs:element name="ContentComponent" type="ContentComponentType" minOccurs="0" maxOccurs="unbounded"/>
                <xs:element name="BaseURL" type="BaseURLType" minOccurs="0" maxOccurs="unbounded"/>
                <xs:element name="SegmentBase" type="SegmentBaseType" minOccurs="0"/>
                <xs:element name="SegmentList" type="SegmentListType" minOccurs="0"/>
                <xs:element name="SegmentTemplate" type="SegmentTemplateType" minOccurs="0"/>
                <xs:element name="Representation" type="RepresentationType" minOccurs="0" maxOccurs="unbounded"/>
            </xs:sequence>
            <xs:attribute ref="xlink:href"/>
            <xs:attribute ref="xlink:actuate" default="onRequest"/>
            <xs:attribute ref="xlink:type" fixed="simple"/>
            <xs:attribute ref="xlink:show" fixed="embed"/>
            <xs:attribute name="id" type="xs:unsignedInt"/>
            <xs:attribute name="group" type="xs:unsignedInt"/>
            <xs:attribute name="lang" type="xs:language"/>
            <xs:attribute name="contentType" type="RFC6838ContentTypeType"/>
            <xs:attribute name="par" type="RatioType"/>
            <xs:attribute name="minBandwidth" type="xs:unsignedInt"/>
            <xs:attribute name="maxBandwidth" type="xs:unsignedInt"/>
            <xs:attribute name="minWidth" type="xs:unsignedInt"/>
            <xs:attribute name="maxWidth" type="xs:unsignedInt"/>
            <xs:attribute name="minHeight" type="xs:unsignedInt"/>
            <xs:attribute name="maxHeight" type="xs:unsignedInt"/>
            <xs:attribute name="minFrameRate" type="FrameRateType"/>
            <xs:attribute name="maxFrameRate" type="FrameRateType"/>
            <xs:attribute name="segmentAlignment" type="xs:boolean" default="false"/>
            <xs:attribute name="subsegmentAlignment" type="xs:boolean" default="false"/>
            <xs:attribute name="subsegmentStartsWithSAP" type="SAPType" default="0"/>
            <xs:attribute name="bitstreamSwitching" type="xs:boolean"/>
            <xs:attribute name="initializationSetRef" type="UIntVectorType"/>
            <xs:attribute name="initializationPrincipal" type="xs:anyURI"/>
        </xs:extension>
    </xs:complexContent>
</xs:complexType>
<xs:simpleType name="RatioType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Ratio Type for sar and par</xs:documentation>
    </xs:annotation>
    <xs:restriction base="xs:string">
        <xs:pattern value="[0-9]*:[0-9]*"/>
    </xs:restriction>
</xs:simpleType>
<xs:simpleType name="FrameRateType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Type for Frame Rate</xs:documentation>
    </xs:annotation>
    <xs:restriction base="xs:string">
        <xs:pattern value="[0-9]+(/[1-9][0-9]*)?"/>
    </xs:restriction>
</xs:simpleType>
<xs:simpleType name="RFC6838ContentTypeType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Type for RFC6838 Content Type</xs:documentation>
    </xs:annotation>
    <xs:restriction base="xs:string">
        <xs:enumeration value="text"/>
        <xs:enumeration value="image"/>
        <xs:enumeration value="audio"/>
        <xs:enumeration value="video"/>
        <xs:enumeration value="application"/>
        <xs:enumeration value="font"/>
    </xs:restriction>
</xs:simpleType>
<xs:simpleType name="SAPType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Stream Access Point type enumeration</xs:documentation>
    </xs:annotation>
    <xs:restriction base="xs:unsignedInt">
        <xs:minInclusive value="0"/>
        <xs:maxInclusive value="6"/>
    </xs:restriction>
</xs:simpleType>
```
<!--PB printed 46-->
<!--(page breaks fell inside the XML block above)-->

<!--PB printed 47-->

<a id="c5-3-3-4"></a>

#### 5.3.3.4 Switching within Adaptation Sets

Switching refers to the presentation of decoded data from one Representation up to a certain time t, and presentation of decoded data of another Representation from time t onwards; for details, refer to subclause [4.3](04-overview.md#c4-3).

The Switching element as defined in Table 6 provides instructions of switch points within an Adaptation Set and the permitted switching options as defined in Table 7. When this element is present, it signals opportunities for simple switching across Representations in one Adaptation Set. This element may be used instead of the attributes @segmentAlignment or @bitstreamSwitching.

Table 7 defines different switching strategies that provide instructions to the client on the procedures to switch appropriately within an Adaptation Set.

Table 6 — Switch point signalling

| Element or Attribute Name | Use | Description |
|---|---|---|
| **Switching** |  | Switching logic description for the associated Representation |
| `@interval` | M | specifies the interval between two switching points in the scale of the `@timescale` on Representation level. Any Segment for which the earliest presentation time minus the `@t` value of the **S** element describing the segment is an integer multiple of the product of `@timescale` and `@interval` is a switch-to opportunity, i.e. it enables to switch to this Representation with the switching strategy as defined by the `@type` value.<br><br>The value should be chosen such that the resulting time matches MPD start time of segments, otherwise no switching will be described. |
| `@type` | OD<br>default: `'media'` | specifies the switching strategy for the switch points identified in by the `@interval` attribute. Switching strategies are defined in Table 7. |

Table 7 — Switching strategies

| Type | Description |
|---|---|
| `media` | Media level switching: in this case, switching is possible at the switch point by decoding and presenting switch-from Representation up to switch point t, initializing the switch-to Representation with the associated Initialization Segment and continue decoding and presenting the switch-to Representation from time t onwards. |
| `bitstream` | Bitstream switching: in this case, switching is possible at the switch point by decoding and presenting switch-from Representation up to switch point t, and continue decoding and presenting the switch-to Representation from time t onwards. More specifically, the concatenation of two Representations at the switch point results in a "conforming Segment track" as defined in 4.5.4 with the media format as specified in the `@mimeType` attribute.<br><br>Initialization of the switch-to Representation is not necessary and is not recommended.<br><br>In order to enable this feature, it is recommended to use the same Initialization Segment for all Representations in the Adaptation Set, i.e. the highest profile/level is signalled in the Initialization Segment. |

<!--PB printed 48-->

The XML schema snippet is as follows:

```xml
<xs:complexType name="SwitchingType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Switching</xs:documentation>
    </xs:annotation>
    <xs:attribute name="interval" type="xs:unsignedInt" use="required"/>
    <xs:attribute name="type" type="SwitchingTypeType" default="media"/>
    <xs:anyAttribute namespace="##other" processContents="lax"/>
</xs:complexType>
<xs:simpleType name="SwitchingTypeType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Switching Type type enumeration</xs:documentation>
    </xs:annotation>
    <xs:restriction base="xs:string">
        <xs:enumeration value="media"/>
        <xs:enumeration value="bitstream"/>
    </xs:restriction>
</xs:simpleType>
```

<a id="c5-3-3-5"></a>

#### 5.3.3.5 Switching across Adaptation Sets

Representations in two or more Adaptation Sets may provide the same content or the content may be offered such that seamless switching is desired (for example in case of multiple viewpoints). In addition, the content may be time-aligned and may be offered such that seamless switching across Representations in different Adaptation Sets is simplified. Typical examples are the offering of the same content with different codecs, for example H.264/AVC and H.265/HEVC and the content author wants to provide such information to the receiver in order to seamlessly switch Representations (as defined in subclause [4.5.1](04-overview.md#c4-5-1)) across different Adaptation Sets.

A content author may signal such seamless switching property across Adaptation Sets by providing a Supplemental Descriptor along with an Adaptation Set with @schemeIdURI set to urn:mpeg:dash:adaptation-set-switching:2016 and the @value is a comma-separated list of Adaptation Set IDs that may be seamlessly switched to from this Adaptation Set.

If the content author signals the ability of Adaptation Set switching and as @segmentAlignment or @subsegmentAlignment are set to TRUE, the (Sub)Segment alignment element shall be valid for all Representations in all Adaptation Sets for which the @id value is included in the @value attribute of the Supplemental descriptor.

For Adaptation Sets containing Representations, when two Adaptation Sets signal Adaptation Set switching, then for any two Representations, X and Y, within the union of the two Adaptation Sets, the m-th Subsegment of X and the n-th Subsegment of Y shall be non-overlapping (as defined in subclause [4.5.3](04-overview.md#c4-5-3)) whenever m is not equal to n.

If the content author signals the ability of Adaptation Set switching and Switching element is provided, the signalled switch points apply for all Representations in all Adaptation Sets for which the @id value is included in the @value attribute of the Supplemental descriptor.

As an example, a content author may signal that seamless switching across an H.264/AVC Adaptation Set with AdaptationSet@id="4" and an HEVC Adaptation Set with AdaptationSet@id="5" is possible by adding a Supplemental Descriptor to the H.264/AVC Adaptation Set with @schemeIdURI set to urn:mpeg:dash:adaptation-set-switching:2016 and the @value="5" and by adding a Supplemental Descriptor to the HEVC Adaptation Set with @schemeIdURI set to urn:mpeg:dash:adaptation-set-switching:2016 and the @value="4".

In addition, if the content author signals the ability of Adaptation Set switching for any Adaptation Sets then the parameters as defined for an Adaption Set shall also hold for all Adaptation Sets that are included in the @value attribute. This constraint may result that the switching may only be signalled with one Adaptation Set, but not with both as for example one Adaptation Set signalling may include all spatial resolutions of another one, whereas it is not the case the other way around.

<!--PB printed 49-->

## Transcription notes

> **Spec note:** Table 5 `@bitstreamSwitching`: "For more details, refer to subclause 0." - a broken cross-reference field in the PDF, reproduced as printed.
>
> **Spec note:** Table 5 `@id` and `@group` rows are also quoted (page-verified) in [`dash-23009-1-2022-ids.md`](dash-23009-1-2022-ids.md); this file is the authoritative statement of the table, the ids file adds the xsd/id-type analysis. `@par` is analysed in [`dash-23009-1-2022-aspect.md`](dash-23009-1-2022-aspect.md).
>
> **Spec note:** Table 5 `@bitstreamSwitching` cell: nested list levels are written with `&emsp;` indentation inside the table cell.
