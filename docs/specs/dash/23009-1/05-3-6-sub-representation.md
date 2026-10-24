---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Clause 5.3.6 - Sub-Representation"
pdf_pages: "70-71"
printed_pages: "62-63"
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION.md
---

<a id="c5-3-6"></a>

### 5.3.6 Sub-Representation

<a id="c5-3-6-1"></a>

#### 5.3.6.1 Overview

Sub-Representations are embedded in regular Representations and are described by the SubRepresentation element. SubRepresentation elements are contained in a Representation element.

The SubRepresentation element describes properties of one or several media content components that are embedded in the Representation. It may for example describe the exact properties of an embedded audio component (e.g. codec, sampling rate, etc.), an embedded sub-title (e.g. codec) or it may describe some embedded lower quality video layer (e.g. some lower frame rate, etc.).

Sub-Representations and Representation share some common attributes and elements.

In case the @level attribute is present in the SubRepresentation element,

- Sub-Representations provide the ability for accessing a lower quality version of the Representation in which they are contained. In this case, Sub-Representations for example allow extracting the audio track in a multiplexed Representation or may allow for efficient fast-forward or rewind operations if provided with lower frame rate;
- the Initialization Segment and/or the Media Segments and/or the Index Segments shall provide sufficient information such that the data can be easily accessed through HTTP partial GET requests. The details on providing such information shall be defined by the media format in use. For media formats defined in this document, the Subsegment Index as defined in subclause 6.3.2.4shall be used.

If the @level attribute is absent, then the SubRepresentation element is solely used to provide a more detailed description for media streams that are embedded in the Representation.

The semantics of the attributes and elements within a Sub-Representation are provided in Table 13 of subclause [5.3.6.2](#c5-3-6-2). The XML syntax of the Sub-Representation type is provided in subclause [5.3.6.3](#c5-3-6-3).

<a id="c5-3-6-2"></a>

#### 5.3.6.2 Semantics

Table 13 — Semantics of SubRepresentation element

| Element or Attribute Name | Use | Description |
|---|---|---|
| **SubRepresentation** |  | specifies a Sub-Representation. |
| `@level` | O | specifies the Sub-Representation level. If `@level` attribute is present and for media formats used in this document, a Subsegment Index as defined in subclause 6.3.2.4 shall be available for each Media Segment in the containing Representation.<br><br>If not present, then the **SubRepresentation** element is solely used to provide a more detailed description for media streams that are embedded in the Representation. |
| `@dependencyLevel` | O | specifies the set of Sub-Representations within this Representation that this Sub-Representation depends on in the decoding and/or presentation process as a whitespace-separated list of `@level` values.<br><br>If not present, the Sub-Representation can be decoded and presented independently of any other Representation. |
| <!--PB printed 62--> `@bandwidth` | CM<br>shall be present if `@level` is present | Identical to the `@bandwidth` definition in Representation but applied to this Sub-Representation. This attribute shall be present if the `@level` attribute is present. |
| `@contentComponent` | O | if present, specifies the set of all media content components that are contained in this Sub-Representation as a whitespace-separated list of values of **ContentComponent**`@id` values.<br><br>if not present, the Sub-Representation is not assigned to a media content component. |
| ***CommonAttributesElements*** | - | Common Attributes and Elements (attributes and elements from base type ***RepresentationBaseType***). For details, see subclause [5.3.7](05-3-7-common.md#c5-3-7). |

Key

For attributes: M=mandatory, O=optional, OD=optional with default value, CM=conditionally mandatory

For elements: <minOccurs>...<maxOccurs> (N=unbounded)

Elements are **bold**; attributes are non-bold preceded with an @, List of elements and attributes is in ***italics bold*** referring to those taken from the Base type that has been extended by this type.

<a id="c5-3-6-3"></a>

#### 5.3.6.3 XML syntax

```xml
<xs:complexType name="SubRepresentationType">
    <xs:annotation>
        <xs:documentation xml:lang="en">SubRepresentation</xs:documentation>
    </xs:annotation>
    <xs:complexContent>
        <xs:extension base="RepresentationBaseType">
            <xs:attribute name="level" type="xs:unsignedInt"/>
            <xs:attribute name="dependencyLevel" type="UIntVectorType"/>
            <xs:attribute name="bandwidth" type="xs:unsignedInt"/>
            <xs:attribute name="contentComponent" type="StringVectorType"/>
        </xs:extension>
    </xs:complexContent>
</xs:complexType>
```
