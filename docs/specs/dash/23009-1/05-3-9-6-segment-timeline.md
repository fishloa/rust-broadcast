---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Clause 5.3.9.6 - Segment timeline"
pdf_pages: "96-100"
printed_pages: "88-92"
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION.md
---

<a id="c5-3-9-6"></a>

#### 5.3.9.6 Segment timeline

<a id="c5-3-9-6-1"></a>

##### 5.3.9.6.1 General

The SegmentTimeline element expresses the earliest presentation time and presentation duration (in units based on the @timescale attribute) for each Segment in the Representation. The use is an alternative to providing the @duration attribute and provides four additional features:

- the specification of arbitrary Segment durations,
- the specification of accurate Segment durations for one media stream where the duration expresses presentation duration of the Segment, and
- the signalling of discontinuities of the Media Presentation timeline for which no Segment data are present in a specific Representation.
- the ability to signal Segment sequences. For more details, refer to subclause [5.3.9.6.4.](#c5-3-9-6-4) Segment sequences shall only be used if explicitly permitted by the profile in use.

For compactness, the syntax of this element includes run-length compression to express a sequence of Segments having constant duration.

The SegmentTimeline element shall contain a list of S elements each of which describes a sequence of contiguous segments of identical MPD duration. The S element contains a mandatory @d attribute specifying the MPD duration, an optional @r repeat count attribute specifying the number of contiguous Segments with identical MPD duration minus one and an optional @t time attribute. The value of the @t attribute minus the value of the @presentationTimeOffset specifies the MPD start time of the first Segment in the series.

The @r attribute has a default value of zero (i.e., a single Segment in the series) when not present. For example, a repeat count of three means there are four contiguous Segments, each with the same MPD duration. The value of the @r attribute of the S element may be set to a negative value indicating that the duration indicated in @d repeats until the S@t of the next S element or if it is the last S element in the SegmentTimeline element until the end of the Period or the next update of the MPD, i.e. it is treated in the same way as the @duration attribute for a full period.

Any @d value shall not exceed the value of MPD@maxSegmentDuration.

The @d attribute shall represent the accurate presentation duration of the Segment except for the following conditions are met:

- the @availabilityTimeOffset is present for the Representation,
- the @availabilityTimeComplete is set to FALSE for the Representation,
- the S element is the one with the largest S@t value in the MPD, and
- the non-adjusted segment availability time of the Segment is greater than or equal to the publish time of the MPD.

This means that for such a Segment, the @d attribute may be updated with a new version of the MPD, or a new S element may be added to reflect a different duration of the Segment than the nominal one. A DASH Client should not rely on nominal segment durations for the derivation of Segment addresses in the case the $Time$ identifier is present, but instead the duration of the Segment may be derived by parsing the Segment and determining the value or by updating the MPD after the non-adjusted segment availability time.

<!--PB printed 88-->

The textual order of the S elements within the SegmentTimeline element shall match the numbering (and thus time) order of the corresponding Media Segments.

When the SegmentTemplate is in use and the $Time$ identifier is present in the SegmentTemplate@media then

- If a Segment Index ('sidx') box is present, then the values of the SegmentTimeline shall describe accurate timing of each Media Segment. Specifically, these values shall reflect the information provided in the Segment index ('sidx') box, i.e.
  - the value of @timescale shall be identical to the value of the timescale field in the first 'sidx' box,
  - the value of S@t shall be identical to the value of the earliest_presentation_time in the first 'sidx' box of the Media Segment described in S,
  - the value of S@d shall be identical to sum of the values of all Subsegment_duration fields in the first 'sidx' box of the Media Segment described in S.
- If a Segment Index ('sidx') box is not present, then the derivation of the earliest presentation time shall be based on the media internal data. The details depend on the segment format in use and further restriction on the segment format may apply.
- The Segment URL for a Media Segment is obtained by replacing the $Time$ identifier by the earliest presentation time obtained from the SegmentTimeline.

NOTE As the earliest presentation time of the next Media Segment in the same Representation can be derived from the actual Media Segment, e.g. by the use of the Segment Index, the Segment URL can be generated without reading of the updated MPD that contains the update to the Segment Timeline.

The semantics of the attributes and elements for Segment Timeline are provided in subclause [5.3.9.6.2](#c5-3-9-6-2), Table 22. The XML syntax of the Segment Timeline is provided in subclause [5.3.9.6.3.](#c5-3-9-6-3)

<a id="c5-3-9-6-2"></a>

##### 5.3.9.6.2 Semantics

Table 22 — Semantics of SegmentTimeline element

| Element or Attribute Name | Use | Description |
|---|---|---|
| **SegmentTimeline** |  | specifies the Segment timeline information |
| **S** | 0 .. N | specifies Segment start time and duration for a contiguous sequence of segments of identical durations, referred to as series in the following.<br><br>NOTE The **S** elements are ordered in sequence of increasing values of the attribute `@t`. |
| <!--PB printed 89--> `@t` | O | this value of this attribute minus the value of the `@presentationTimeOffset` specifies the MPD start time, in `@timescale` units, of the first Segment in the series. The MPD start time is relative to the beginning of the Period.<br><br>The value of this attribute shall be equal to or greater than the sum of the previous **S** element earliest presentation time and the sum of the contiguous Segment durations.<br><br>If the value of the attribute is greater than what is expressed by the previous **S** element, it expresses discontinuities in the timeline.<br><br>If not present, then the value shall be assumed to be zero for the first **S** element and for the subsequent **S** elements, the value shall be assumed to be the sum of the previous **S** element's earliest presentation time and contiguous duration [i.e. previous **S**`@t` + `@d` * (`@r` + 1)]. |
| `@n` | O | specifies the Segment number of the first Segment in the series.<br><br>The value of this attribute shall be at least one greater than the number of previous **S** elements plus the `@startNumber` attribute value, if present. If the value of `@n` is greater than one plus the previously calculated Segment number, it expresses that one or more prior Segments in the timeline are unavailable. |
| `@d` | M | specifies the Segment duration or the duration of a Segment sequence, in units of the value of the `@timescale`. |
| `@k` | OD<br>default: 1 | specifies the number of Segments that are included in a Segment Sequence. The attribute shall not be present unless explicitly permitted by the profile.<br><br>For more details, refer to subclause [5.3.9.6.3.](#c5-3-9-6-3) |
| `@r` | OD<br>default: 0 | specifies the repeat count of the number of following contiguous Segments or Segment Sequences with the same duration expressed by the value of `@d`. This value is zero-based (e.g. a value of three means four Segments or Segment Sequences in the contiguous series). A negative value of the `@r` attribute of the **S** element indicates that the duration indicated in `@d` attribute repeats until the start of the next **S** element, the end of the Period or until the next MPD update. |

<!--PB printed 90-->

Key

For attributes: M=mandatory, O=optional, OD=optional with default value, CM=conditionally mandatory

For elements: <minOccurs>...<maxOccurs> (N=unbounded)

Elements are **bold**; attributes are non-bold and preceded with an @.

<a id="c5-3-9-6-3"></a>

##### 5.3.9.6.3 XML syntax

```xml
<xs:complexType name="SegmentTimelineType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Segment Timeline</xs:documentation>
    </xs:annotation>
    <xs:sequence>
        <xs:element name="S" minOccurs="0" maxOccurs="unbounded">
            <xs:complexType>
                <xs:attribute name="t" type="xs:unsignedLong"/>
                <xs:attribute name="n" type="xs:unsignedLong"/>
                <xs:attribute name="d" type="xs:unsignedLong" use="required"/>
                <xs:attribute name="r" type="xs:integer" default="0"/>
                <xs:attribute name="k" type="xs:unsignedLong" default="1"/>
                <xs:anyAttribute namespace="##other" processContents="lax"/>
            </xs:complexType>
        </xs:element>
        <xs:any namespace="##other" processContents="lax" minOccurs="0" maxOccurs="unbounded"/>
    </xs:sequence>
    <xs:anyAttribute namespace="##other" processContents="lax"/>
</xs:complexType>
```

<a id="c5-3-9-6-4"></a>

##### 5.3.9.6.4 Segment Sequences

Segment sequences in the Segment Timeline may be signalled with including a @k attribute in the S element of the SegmentTimeline. The @k shall only be present if all of the following requirements are fulfilled:

- the addressing scheme for the associated Representation is using Segment template with either $Number$ or hierarchical templating and sub-numbering as defined in subclause [5.3.9.6.5](#c5-3-9-6-5),
- the profile explicitly allows the usage of Segment sequences.

If @k is present and greater than 1, then it specifies that sequence described by @d is accurate in timing but contains @k Segments.

The MPD duration of the Segment is determined as the integer value of @d divided by the value of @k and determines the MPD start time and therefore the Segment availability start time. MPD duration of the Segments is not required to exactly match the media duration of the Segments.

The integer of the quotient of the value of @d and the value of @k of any S element shall not exceed the quotient of @d and the value of @k minus 1 of any other S element in the Segment Timeline.

The concatenation of all Segments in a Segment sequence shall have an accurate segment duration according of the value of @d.

Any Segment in a Segment sequence may carry inband events.

<a id="c5-3-9-6-5"></a>

##### 5.3.9.6.5 Hierarchical Templating and Sub-Numbering

If the Segment template contains a $SubNumber$ value and a Segment Timeline signalling with Segment sequence is used, then

<!--PB printed 91-->

- if $Time$ is present, the $Time$ is replaced with the earliest presentation time of the Segment sequence for all Segments in the Segment Sequence,
- if $Number$ is present, the $Number$ is replaced with the number of the Segment sequence, i.e. with the number as if every Segment sequence in the Segment timeline is treated as single Segment,
- and in both cases the $SubNumber$ is replaced with the Segment number of the Segment Sequence, with 1 being the number of the first Segment in the sequence.

NOTE The earliest presentation time of the next Segment Sequence in the same Representation can be derived from the sum of the earliest presentation time of the current Segment Sequence and the duration of the Segment resulting from the concatenation of all Media Segments in a Segment Sequence. In case of ISO BMFF, this can be accomplished by summing the track runs of segments in the seqment sequence.

## Transcription notes

> **Spec note:** Table 22 `@k`: "For more details, refer to subclause 5.3.9.6.3" - Segment Sequences are 5.3.9.6.4. As printed.
>
> **Spec note:** 5.3.9.6.5 NOTE: "seqment sequence" typo as printed.
>
> **Spec note:** 5.3.9.6.1: "The `@d` attribute shall represent the accurate presentation duration of the Segment except for the following conditions are met" - sic.
