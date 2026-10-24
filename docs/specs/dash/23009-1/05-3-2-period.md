---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Clause 5.3.2 - Period"
pdf_pages: "41-47"
printed_pages: "33-39"
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION.md
---

<a id="c5-3-2"></a>

### 5.3.2 Period

<a id="c5-3-2-1"></a>

#### 5.3.2.1 Overview

A Media Presentation consists of one or more Periods. A Period is defined by a **Period** element in the **MPD** element.

<!--PB printed 33-->

Three types of Periods are defined:

- Regular Period,
- Early Available Period,
- Early Terminated Period.

The type of the Period, as well as the PeriodStart time of a regular Period or early terminated Period is determined as follows:

- If the attribute @start is present in the Period, then the Period is a regular Period or an early terminated Period and the PeriodStart is equal to the value of this attribute.
- If the @start attribute is absent, but the previous Period element contains a @duration attribute then this new Period is also a regular Period or an early terminated Period. The start time of the new Period PeriodStart is the sum of the start time of the previous Period PeriodStart and the value of the attribute @duration of the previous Period.
- If (i) @start attribute is absent, and (ii) the Period element is the first in the MPD, and (iii) the MPD@type is 'static′, then the PeriodStart time shall be set to zero.
- If (i) @start attribute is absent, and (ii) the previous Period element does not contain a @duration attribute or the Period element is the first in the MPD, and (iii) the MPD@type is 'dynamic', then this Period is an Early Available Period (see below for details).
- If (i) @duration attribute is present, and (ii) the next Period element contains a @start attribute or the @minimumUpdatePeriod is present, then this Period is an Early Terminated Period (see below for details).

For any regular Period and early terminated Period, the following holds: PeriodStart reflects the actual time that should elapse after playing the media of all prior Periods in this Media Presentation relative to the PeriodStart time of the first Period in the Media Presentation. The Period extends until the PeriodStart of the next Period, or until the end of the Media Presentation in the case of the last Period. For regular Periods, the difference between the PeriodStart time of a Period and either the PeriodStart time of the following Period, if this is not the last Period, or the value of the MPD@mediaPresentationDuration if this is the last one, is the presentation duration in Media Presentation time of the media content represented by the Representations in this Period. For Early Terminated Periods, the value of the Period@duration is the presentation duration in Media Presentation time of the media content represented by the Representations in this Period.

Early Available Periods may be used to advertise initialization of other non-media data before the media data itself is available. Period elements documenting early available Periods shall not occur before any Period element documenting a regular Period. For Early Available Periods, any resources that are announced in such a Period element shall be available. The data contained in such a Period element does not represent a Period in the Media Presentation. Only when the PeriodStart time becomes known through an update of the MPD, such a Period element represents a regular Period. However, an update of the MPD may even remove a Period element representing an Early Available Period in later updates of the MPD as long as no PeriodStarttime is associated with the Period.To avoid dereferencing of a remote element entity containing a Period element solely to determine the Period timeline, e.g. in case of seeking, Period@start or previous Period′s Period@duration should be present in the MPD.

The semantics of the attributes and elements within a Period element are provided in Table 4 of subclause [5.3.2.2.](#c5-3-2-2) The XML syntax of the Period element is provided in subclause [5.3.2.3.](#c5-3-2-3)

<!--PB printed 34-->

<a id="c5-3-2-2"></a>

#### 5.3.2.2 Semantics

Table 4 — Semantics of Period element

| Element or Attribute Name | Use | Description |
|---|---|---|
| **Period** |  | specifies the information of a Period. |
| `@xlink:href` | O | specifies a reference to a remote element entity that is either empty or contains one or multiple top-level elements of type **Period**. |
| `@xlink:actuate` | OD<br>default: `onRequest` | specifies the processing instructions, which can be either "onLoad" or "onRequest".<br><br>This attribute shall not be present if the `@xlink:href` attribute is not present. |
| `@id` | O | specifies an identifier for this Period. The identifier shall be unique within the scope of the Media Presentation.<br><br>If the **MPD**`@type` is "dynamic", then this attribute shall be present and shall not change in case the MPD is updated.<br><br>If not present, no identifier for the Period is provided.<br><br>In case of a remote period element, every **Period**`@id` within the remote entity has to be unique within the media presentation. If the generator of the remote entity is unaware of the **Period**`@id` values used within the media presentation, it can use unique identifiers (such as UUIDs) as values for **Period**`@id`. If the xlink resolver maintains the same generated id for every client and for each time that the remote period is requested, the remote element is cacheable. |
| `@start` | O | if present, specifies the *PeriodStart* time of the Period. The *PeriodStart* time is used as an anchor to determine the MPD start time of each Media Segment as well as to determine the presentation time of each access unit in the Media Presentation timeline.<br><br>If not present, refer to the details in subclause [5.3.2.1.](#c5-3-2-1)<br><br>The value of PeriodStart, together with the value of the **MPD**`@availabilityStartTime` enables to derive the Segment availability times for dynamic media presentations. For details, refer to subclause [5.3.9.](05-3-9-segment-information.md#c5-3-9) |
| `@duration` | O | if present, specifies the duration of the Period to determine the *PeriodStart* time of the next Period.<br><br>If not present, refer to the details in subclause [5.3.2.1.](#c5-3-2-1) |
| `@bitstreamSwitching` | OD<br>Default: `false` | When set to `'true'`, this is equivalent as if the **AdaptationSet**`@bitstreamSwitching` for each Adaptation Set contained in this Period is set to `'true'`. In this case, the **AdaptationSet**`@bitstreamSwitching` attribute shall not be set to `'false'` for any Adaptation Set in this Period. |
| <!--PB printed 35--> **BaseURL** | 0…N | specifies a base URL that can be used for reference resolution and alternative URL selection. For more details, refer to the description in subclause 5.6. |
| **SegmentBase** | 0...1 | specifies default Segment Base information.<br><br>Information in this element is overridden by information in AdaptationSet.SegmentBase and Representation.SegmentBase, if present.<br><br>For more details, see subclause [5.3.9.](05-3-9-segment-information.md#c5-3-9) |
| **SegmentList** | 0...1 | specifies default Segment List information.<br><br>Information in this element is overridden by information in AdaptationSet.SegmentList and Representation.SegmentList, if present.<br><br>For more details, see subclause [5.3.9.](05-3-9-segment-information.md#c5-3-9) |
| **SegmentTemplate** | 0...1 | specifies default Segment Template information.<br><br>Information in this element is overridden by information in AdaptationSet.SegmentTemplate and Representation.SegmentTemplate, if present.<br><br>For more details, see subclause [5.3.9.](05-3-9-segment-information.md#c5-3-9) |
| **AssetIdentifier** | 0...1 | specifies that this Period belongs to a certain asset.<br><br>For more details, see subclause 5.8.5.7. |
| **EventStream** | 0...N | specifies an event stream.<br><br>For more details, see subclause 5.10.2. |
| **ServiceDescription** | 0... N | specifies the service description that is intended by the service provider on how the service is expected to be consumed in this Period.<br><br>For details refer to Table 3 in subclause [5.3.1.3.](05-3-1-hierarchy.md#c5-3-1-3)<br><br>If any element is present in the Period, then this information overrides all Service Description information provided on MPD level with the same scope. |
| **ContentProtection** | 0 … N | specifies information about content protection and encryption schemes used in this Media Presentation. If present on this level, it shall include the @refId attribute.<br><br>For details, see subclauses 5.8.1 and 5.8.4.1. |
| **AdaptationSet** | 0...N | specifies an Adaptation Set.<br><br>At least one Adaptation Set shall be present in each Period unless the value of the @duration attribute of the Period is set to zero.<br><br>For more details, see 5.3.3. |
| **Subset** | 0...N | specifies a Subset. For more details, see subclause [5.3.8.](05-3-8-subsets.md#c5-3-8) |
| <!--PB printed 36--> **SupplementalProperty** | 0...N | specifies supplemental information about the containing element that may be used by the DASH Client optimizing the processing.<br><br>For details, see subclause 5.8.4.9. |
| **EmptyAdaptationSet** | 0...N | specifies an Adaptation Set that does not contain any Representation element. The empty Adaptation Set is of the same type as a regular Adaptation Set but shall neither contain an xlink nor contain any Representation element.<br><br>This element shall only be present, if an Essential Descriptor is present with @schemeIDURI set to "urn:mpeg:dash:mpd-as-linking:2015".<br><br>For more details, see subclause 5.8.5.8. |
| **GroupLabel** | 0 … N | A summary label for a group of Labels.<br><br>For more details, refer to subclause 5.3.10. |
| **Preselection** | 0 … N | specifies a preselection, i.e. a combination of Adaptation Sets that form a specific experience and can be selected for joint decoding and rendering.<br><br>For more details, refer to subclause 5.3.11. |


Key

For attributes: M=mandatory, O=optional, OD=optional with default value, CM=conditionally mandatory

For elements: <minOccurs>...<maxOccurs> (N=unbounded)

The conditions only hold without using xlink:href. If linking is used, then all attributes are "optional" and <minOccurs=0>.

Elements are bold; attributes are non-bold and preceded with an @.

<a id="c5-3-2-3"></a>

#### 5.3.2.3 XML syntax

```xml
<xs:complexType name="PeriodType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Period</xs:documentation>
    </xs:annotation>
    <xs:sequence>
        <xs:element name="BaseURL" type="BaseURLType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="SegmentBase" type="SegmentBaseType" minOccurs="0"/>
        <xs:element name="SegmentList" type="SegmentListType" minOccurs="0"/>
        <xs:element name="SegmentTemplate" type="SegmentTemplateType" minOccurs="0"/>
        <xs:element name="AssetIdentifier" type="DescriptorType" minOccurs="0"/>
        <xs:element name="EventStream" type="EventStreamType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="ServiceDescription" type="ServiceDescriptionType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="ContentProtection" type="ContentProtectionType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="AdaptationSet" type="AdaptationSetType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="Subset" type="SubsetType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="SupplementalProperty" type="DescriptorType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="EmptyAdaptationSet" type="AdaptationSetType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="GroupLabel" type="LabelType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:element name="Preselection" type="PreselectionType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:any namespace="##other" processContents="lax" minOccurs="0" maxOccurs="unbounded"/>
    </xs:sequence>
    <xs:attribute ref="xlink:href"/>
    <xs:attribute ref="xlink:actuate" default="onRequest"/>
    <xs:attribute ref="xlink:type" fixed="simple"/>
    <xs:attribute ref="xlink:show" fixed="embed"/>
    <xs:attribute name="id" type="xs:string"/>
    <xs:attribute name="start" type="xs:duration"/>
    <xs:attribute name="duration" type="xs:duration"/>
    <xs:attribute name="bitstreamSwitching" type="xs:boolean" default="false"/>
    <xs:anyAttribute namespace="##other" processContents="lax"/>
</xs:complexType>
```
<!--PB printed 37-->
<!--(page breaks fell inside the XML block above)-->

<a id="c5-3-2-4"></a>

#### 5.3.2.4 Content offering with multiple Periods

Content with multiple Periods may be created for different reasons, for example:

- to enable splicing of content, for example for ad insertion,
- to provide synchronization in segment numbering, e.g. compensate non-constant segment durations,
- to remove or add certain Representations in an Adaptation Set,
- to remove or add certain Adaptation Sets,
- to remove or add content offering on certain CDNs,
- to enable signalling of shorter segments, if produced by the encoder.

Periods provide opportunities for resync, for ad insertion, for adding and removing Representations. However, in certain circumstances, the content across Period boundaries is continuous and, in this case, continuous playout of the client is expected.

In certain circumstances, the Media Presentation is offered such that the next Period is a continuation of the content in the previous Period, possibly the immediately following Period or in a later Period (e.g. after an advertisement Period had been inserted).

The content provider may express that the media components contained in two Adaptation Sets in two different Periods are associated by assigning equivalent Asset Identifiers to both Periods and by identifying both Adaptation Sets with identical value for the attribute @id. Association expresses a logical continuation of the media component in the next Period and may for example be used by the client to continue playing an associated Adaptation Set in the new Period.

In addition, two Adaptation Sets in one MPD are period-continuous if all the following holds:

- The Adaptation Sets are associated.
- The sum of the value of the @presentationTimeOffset and the presentation duration of all Representations in one Adaptation Set are identical to the value of the @presentationTimeOffset of the associated Adaptation Set in the next Period.
- If Representations in both Adaptation Sets have the same value for @id, then they shall have functionally equivalent Initialization Segments, i.e. the Initialization Segment may be used to continue the play-out of the Representation. The concatenation of the Initialization Segment of the first Period, if present, and all consecutive Media Segments in the Representation in the first Period and subsequently the concatenation with all consecutive Media Segments in the Representation of the second Period shall represent a conforming Segment track as defined in subclause [4.5.4](04-overview.md#c4-5-4) conforming to the media type as specified in the @mimeType attribute for the Representation in the first Period. Additionally, the @mimeType attribute for the Representation in the next Period shall be the same as one of the first Period.

<!--PB printed 38-->

Media Presentations should signal period-continuous Adaptation Sets by using a supplemental descriptor on Adaptation Set level with @schemeIdUri set to "urn:mpeg:dash:period-continuity:2015" with

- the @value of the descriptor matching the value of an @id of a Period that is contained in the MPD, and
- the value of the AdaptationSet@id being the same in both Periods.

MPD should signal period-continuous Adaptation Sets if the MPD contains Periods with identical Asset Identifiers.

There exist special cases for which the media in one Adaptation Set is a continuation of the previous one, but the timestamps are not continuous. Examples are timestamp wrap around, encoder reset, splicing, or other aspects. Two Adaptation Sets in one MPD are period-connected if all conditions from period-continuity from above hold, except that the timestamps across Period boundaries may be non-continuous but adjusted by the value of the @presentationTimeOffset at the Period boundary. However, for example the Initialization Segment is equivalent within the two Adaptation Sets. Media Presentations should signal period-connected Adaptation Sets by using a supplemental descriptor on Adaptation Set level with @schemeIdUri set to "urn:mpeg:dash:period-connectivity:2015".

Period continuity implies period connectivity.

For appropriate client behaviour, please refer to A.9.

<a id="c5-3-2-5"></a>

#### 5.3.2.5 Special Periods in Live Services

In order to address live services for which Periods are extended into the future and Periods are gradually removed due to the timeshift buffer depth, special types of Periods are defined as follows:

- Partially Unavailable Period: any Period in a dynamic Media Presentation for which the sum of the MPD@availabilityStartTime and the value of Period@start is smaller than the value of @publishTime minus the value of MPD@timeShiftBufferDepth. For this Period, some Segments may no longer be available as they have been fallen out of the time shift buffer window.
- Live-Edge Period: newest regular Period in a dynamic Media Presentation, for which the Period duration is unknown, and Segment availability start time of at least one Segment is greater than the value of MPD@publishTime. For such Periods, the exact duration of Segments that are not yet available, may be unknown.

Note that an MPD may contain both types of Periods, and further, a single Period may be both at the same time, a Partially Unavailable Period and a Live-Edge Period.

## Transcription notes

> **Spec note:** 5.3.2.1: apostrophes in `'static'` / `'dynamic'` are U+2032 PRIME in the text layer and render as straight apostrophes; written as `'`.
>
> **Spec note:** 5.3.2.5 heading "Special Periods in Live Services": the two list items are italic-led definitions (*Partially Unavailable Period*, *Live-Edge Period*).
