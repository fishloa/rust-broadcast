---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Clause 5.3.4 - Media content component"
pdf_pages: "58-59"
printed_pages: "50-51"
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION.md
---

<a id="c5-3-4"></a>

### 5.3.4 Media content component

<a id="c5-3-4-1"></a>

#### 5.3.4.1 Overview

Each Adaptation Set contains one or more media content components. The properties of each media content component are described by a ContentComponent element or may be described directly on the AdaptationSet element if only one media content component is present in the Adaptation Set. ContentComponent elements are contained in an AdaptationSet element.

The semantics of the attributes and elements within a ContentComponent element are provided in Table 8 in subclause [5.3.4.2.](#c5-3-4-2) The XML syntax of the ContentComponent element is provided in subclause [5.3.4.3.](#c5-3-4-3)

<a id="c5-3-4-2"></a>

#### 5.3.4.2 Semantics

Table 8 — Semantics of ContentComponent element

| Element or Attribute Name | Use | Description |
|---|---|---|
| **ContentComponent** |  | Description of a content component. |
| `@id` | O | specifies an identifier for this media component. The attribute shall be unique in the scope of the containing Adaptation Set.<br><br>The value of this attribute should be the media component identifier in the media segment (i.e. the Track Id in ISO BMFF segments and PID in MPEG-2 TS segments) described by this **ContentComponent** element.<br><br>Multiplexing of media components may also be provided on elementary stream level, for example for next generation audio codecs, different audio components may be mixed. In this case, the id may also point to specific components in a single elementary stream. |
| `@lang` | O | Same semantics as in Table 5 for `@lang` attribute. |
| `@contentType` | O | Same semantics as in Table 5 for `@contentType` attribute. |
| `@par` | O | Same semantics as in Table 5 for `@par` attribute. |
| `@tag` | O | specifies the tag of the Content Component which may be used for selection purposes towards the decoder. |
| **Accessibility** | 0 ... N | Same semantics as in Table 5 for **Accessibility** element. |
| **Role** | 0 ... N | Same semantics as in Table 5 for **Role** element. |
| **Rating** | 0 ... N | Same semantics as in Table 5 for **Rating** element. |
| **Viewpoint** | 0 ... N | Same semantics as in Table 5 for **Viewpoint** element. |

<!--PB printed 50-->

Key

For attributes: M=mandatory, O=optional, OD=optional with default value, CM=conditionally mandatory, F=fixed

For elements: <minOccurs>...<maxOccurs> (N=unbounded)

Elements are bold; attributes are non-bold and preceded with an @; list of elements and attributes is in italics bold referring to those taken from the Base type that has been extended by this type.

<a id="c5-3-4-3"></a>

#### 5.3.4.3 XML syntax

```xml
<xs:complexType name="ContentComponentType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Content Component</xs:documentation>
    </xs:annotation>
    <xs:sequence>
        <xs:element name="Accessibility" type="DescriptorType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="Role" type="DescriptorType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="Rating" type="DescriptorType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="Viewpoint" type="DescriptorType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:any namespace="##other" processContents="lax" minOccurs="0" maxOccurs="unbounded"/>
    </xs:sequence>
    <xs:attribute name="id" type="xs:unsignedInt"/>
    <xs:attribute name="lang" type="xs:language"/>
    <xs:attribute name="contentType" type="RFC6838ContentTypeType"/>
    <xs:attribute name="par" type="RatioType"/>
    <xs:attribute name="tag" type="TagType"/>
    <xs:anyAttribute namespace="##other" processContents="lax"/>
</xs:complexType>
<xs:simpleType name="TagType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Tag</xs:documentation>
    </xs:annotation>
    <xs:restriction base="xs:string"/>
</xs:simpleType>
```
