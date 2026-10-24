---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Clause 5.3.9.4 - Segment template"
pdf_pages: "90-91"
printed_pages: "82-83"
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION.md
---

<a id="c5-3-9-4"></a>

#### 5.3.9.4 Segment template

<a id="c5-3-9-4-1"></a>

##### 5.3.9.4.1 Overview

The Segment template is defined by the SegmentTemplate element. In this case, specific identifiers that are substituted by dynamic values assigned to Segments, to create a list of Segments. The substitution rules are provided in subclause [5.3.9.4.4](#c5-3-9-4-4).

The semantics of the attributes and elements for the Segment list are provided in subclause [5.3.9.4.2](#c5-3-9-4-2), Table 20. The XML syntax of the Segment Information is provided in subclause [5.3.9.4.3](#c5-3-9-4-3).

<a id="c5-3-9-4-2"></a>

##### 5.3.9.4.2 Semantics

Table 20 — Semantics of SegmentTemplate element

| Element or Attribute Name | Use | Description |
|---|---|---|
| **SegmentTemplate** |  | specifies Segment template information. |
| ***MultipleSegmentBaseInformation*** |  | Provides the Multiple Segment base information as defined in subclause [5.3.9.2](05-3-9-segment-information.md#c5-3-9-2). |
| `@media` | O | specifies the template to create the Media Segment List.<br><br>For more details, refer to subclause [5.3.9.4.4](#c5-3-9-4-4). |
| `@index` | O | specifies the template to create the Index Segment List. If neither the $Number$ nor the $Time$ identifier is included, this provides the URL to a Representation Index.<br><br>For more details, refer to subclause [5.3.9.4.4](#c5-3-9-4-4). |
| `@initialization` | O | specifies the template to create the Initialization Segment. Neither $Number$ nor the $Time$ identifier shall be included.<br><br>For more details, refer to subclause [5.3.9.4.4](#c5-3-9-4-4). |
| `@bitstreamSwitching` | O | specifies the template to create the Bitstream Switching Segment. Neither $Number$ nor the $Time$ identifier shall be included.<br><br>For more details, refer to subclause [5.3.9.4.4](#c5-3-9-4-4). |

Key

For attributes: M=mandatory, O=optional, OD=optional with default value, CM=conditionally mandatory

For elements: <minOccurs>...<maxOccurs> (N=unbounded)

Elements are **bold**; attributes are non-bold and preceded with an @.

<a id="c5-3-9-4-3"></a>

##### 5.3.9.4.3 XML syntax

```xml
<xs:complexType name="SegmentTemplateType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Segment Template</xs:documentation>
    </xs:annotation>
    <xs:complexContent>
        <xs:extension base="MultipleSegmentBaseType">
            <xs:attribute name="media" type="xs:string"/>
            <xs:attribute name="index" type="xs:string"/>
            <xs:attribute name="initialization" type="xs:string"/>
            <xs:attribute name="bitstreamSwitching" type="xs:string"/>
        </xs:extension>
    </xs:complexContent>
</xs:complexType>
```
<!--PB printed 82-->
<!--(page breaks fell inside the XML block above)-->

<a id="c5-3-9-4-4"></a>

##### 5.3.9.4.4 Template-based Segment URL construction

The SegmentTemplate@media attribute, the SegmentTemplate@index attribute, the SegmentTemplate@initialization attribute and the SegmentTemplate@bitstreamSwitching attribute each contain a string that may contain one or more of the identifiers as listed in Table 21.

In each URL, the identifiers from Table 20 shall be replaced by the substitution parameter defined in Table 16. Identifier matching is case-sensitive. If the URL contains unescaped $ symbols which do not enclose a valid identifier, then the result of URL formation is undefined. In this case, it is expected that the DASH Client ignores the entire containing Representation element and the processing of the MPD continues as if this Representation element was not present. The format of the identifier is also specified in Table 21.

Each identifier may be suffixed, within the enclosing '$' characters, with an additional format tag aligned with the printf format tag as defined in IEEE 1003.1-2008<sup>[10]</sup> following this prototype:

%0[width]d

The width parameter is an unsigned integer that provides the minimum number of characters to be printed. If the value to be printed is shorter than this number, the result shall be padded with zeros. The value is not truncated even if the result is larger.

The Media Presentation shall be authored such that the application of the substitution process results in valid Segment URLs.

Strings outside identifiers shall only contain characters that are permitted within URLs according to IETF RFC 3986.

Table 21 — Identifiers for URL templates

| $\<Identifier\>$ | Substitution parameter | Format |
|---|---|---|
| *$$* | Is an escape sequence, i.e. "$$" is replaced with a single "$" | not applicable |
| *$RepresentationID$* | This identifier is substituted with the value of the attribute **Representation**`@id` of the containing Representation. | The format tag shall not be present. |
| *$Number$* | This identifier is substituted with the *number* of the corresponding Segment, if *$SubNumber$* is not present in the same string.<br><br>If $SubNumber$ is present, this identifier is substituted with the *number* of the corresponding Segment sequence. For details, refer to subclauses [5.3.9.6.4](05-3-9-6-segment-timeline.md#c5-3-9-6-4) and [5.3.9.6.5](05-3-9-6-segment-timeline.md#c5-3-9-6-5). | The format tag may be present.<br><br>If no format tag is present, a default format tag with `width`=1 shall be used. |
| *$Bandwidth$* | This identifier is substituted with the value of **Representation**`@bandwidth` attribute value. | The format tag may be present.<br><br>If no format tag is present, a default format tag with `width`=1 shall be used. |
| <!--PB printed 83--> *$Time$* | This identifier is substituted with the value of the MPD start time of the Segment being accessed. For the Segment Timeline, this means that this identifier is substituted with the value of the **SegmentTimeline**`@t` attribute for the Segment being accessed. Either `$Number$` or `$Time$` may be used but not both at the same time. | The format tag may be present.<br><br>If no format tag is present, a default format tag with `width`=1 shall be used. |
| *$SubNumber$* | This identifier is substituted with the *number* of the corresponding Segment in a Seqment Sequence. This identifier shall only be present if either *$Number$* or *$Time$* are present as well. For details, refer to subclauses [5.3.9.6.4](05-3-9-6-segment-timeline.md#c5-3-9-6-4) and [5.3.9.6.5](05-3-9-6-segment-timeline.md#c5-3-9-6-5). | The format tag may be present.<br><br>If no format tag is present, a default format tag with `width`=1 shall be used. |

## Transcription notes

> **Spec note:** 5.3.9.4.4: "the identifiers from Table 20 shall be replaced by the substitution parameter defined in Table 16" - the printed table numbers are wrong (identifiers are in Table 21, there is no substitution-parameter table 16 for this purpose). Reproduced as printed; the normative table is Table 21.
>
> **Spec note:** Table 21 `$SubNumber$` row contains "Seqment Sequence" (typo) as printed.
