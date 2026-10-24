---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Clause 5.3.9.7 - Failover Content Signalling"
pdf_pages: "100-101"
printed_pages: "92-93"
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION.md
---

<a id="c5-3-9-7"></a>

#### 5.3.9.7 Failover Content Signalling

<a id="c5-3-9-7-1"></a>

##### 5.3.9.7.1 General

In certain cases, a time-continuous section of a Representation or an Adaptation Set is not properly represented, for example due to errors in the content ingest or encoding. However, at least at the DASH level, the content may be offered properly by adding a Missing Segments as defined in 6.2.6 or by including content that is encoded but representing some failover content instead of the main content.

The FailoverContent element enables identification of such sections of Representations that include alternative “failover content” and should be avoided by the DASH Client, for example by switching to a Representation that does not include such failover content.

The FailoverContent element contains a list of failover content sections, each expressed by one FCS element. The FCS element contains an optional @d attribute specifying the MPD duration of the alternative content section and a mandatory @t time attribute whereby the value of the @t attribute minus the value of the @presentationTimeOffset specifies the MPD start time of the alternative content section.

The semantics of the attributes and elements for failover content are provided in 5.3.9.7.2, Table 23. The XML syntax of the Failover Content signalling is provided in 5.3.9.7.3.

<a id="c5-3-9-7-2"></a>

##### 5.3.9.7.2 Semantics

Table 23 — Semantics of FailoverContent element

| Element or Attribute Name | Use | Description |
|---|---|---|
| **FailoverContent** |  | specifies sections that contain failover content. |
| `@valid` | OD<br>default: `true` | specifies whether the corresponding data results in a valid HTTP request or if the request is invalid and it is expected that a 404 is received when requesting the associated Media Segment. |
| **FCS** | 1 .. N | specifies one continuous section of failover content. |
| `@t` | M | this value of this attribute minus the value of the `@presentationTimeOffset` specifies the MPD start time, in `@timescale` units, of the first sample in the alternative content section.<br><br>The value of this attribute shall be equal to or greater than the sum of the previous FCS element earliest presentation time and the sum of the contiguous section durations. |
| <!--PB printed 92--> `@d` | O | specifies the alternative content section duration in units of the value of the `@timescale`.<br><br>If not present, the alternative content section lasts until the start of the next **FCS** element, or until the end of the Period or until the end of MPD duration, whichever occurring first in the timeline. |

Legend:
For attributes: M=mandatory, O=optional, OD=optional with default value, CM=conditionally mandatory.
For elements: <minOccurs>...<maxOccurs> (N=unbounded)
Elements are **bold**; attributes are non-bold and preceded with an @.

<a id="c5-3-9-7-3"></a>

##### 5.3.9.7.3 XML syntax

```xml
<xs:complexType name="FailoverContentType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Failover Content</xs:documentation>
    </xs:annotation>
    <xs:sequence>
        <xs:element name="FCS" maxOccurs="unbounded">
            <xs:complexType>
                <xs:attribute name="t" type="xs:unsignedLong" use="required"/>
                <xs:attribute name="d" type="xs:unsignedLong"/>
                <xs:anyAttribute namespace="##other" processContents="lax"/>
            </xs:complexType>
        </xs:element>
        <xs:any namespace="##other" processContents="lax" minOccurs="0" maxOccurs="unbounded"/>
    </xs:sequence>
    <xs:attribute name="valid" type="xs:boolean" default="true"/>
    <xs:anyAttribute namespace="##other" processContents="lax"/>
</xs:complexType>
```
