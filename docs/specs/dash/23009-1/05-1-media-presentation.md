---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Clauses 5.1 and 5.2 - Media Presentation, Media Presentation Description"
pdf_pages: "30-34"
printed_pages: "22-26"
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION.md
---

<a id="c5"></a>

# 5 Media Presentation

<a id="c5-1"></a>

## 5.1 General

A Media Presentation is a collection of data that is accessible to a DASH Client to provide a streaming service to the user.

A Media Presentation is described by an MPD including possible updates of the MPD. The MPD is defined in subclause [5.2](#c5-2) and the MPD update mechanism is defined in subclause 5.4. Assembly of a fragmented MPD is defined in subclause 5.5. The data model that constitutes a Media Presentation is defined in subclause [5.2.3.5](#c5-2-3-5). In subclause 5.6, the formats and processing of URLs in the MPD is introduced. Program information is defined in subclause 5.7. Descriptors associated to Representations or collections thereof are provided in subclause 5.8. DASH metric collection description is specified in subclause 5.9. Subclause 5.10 defines Events and Event Streams. Subclause 5.11 defines MPD chaining. Subclause 5.12 defines the Producer Reference Time. Subclause 5.13 defines how to deal with leap seconds. Subclause 5.14 defines content popularity rates.

<a id="c5-2"></a>

## 5.2 Media Presentation Description

<a id="c5-2-1"></a>

### 5.2.1 General

The Media Presentation Description (MPD) is a document that contains metadata required by a DASH Client to construct appropriate HTTP-URLs to access Segments and to provide the streaming service to the user.

NOTE 1 Actual playback of the media streams included in the Representations is not controlled by the MPD information. Playback is controlled by the media engine operating on the media streams contained in the Representations in the usual way.

The format of URLs in the MPD and the process to generate HTTP GET and partial GET requests from URLs provided in the MPD is defined in subclause 5.6.

The MPD is an XML document that shall be formatted according to the XML schema provided in Annex B. Some context on the schema is provided in subclause [5.2.2](#c5-2-2).

The extension of the DASH XML schema (as provided in Annex B), in particular the addition of XML attributes or elements in the DASH namespace, is reserved to ISO/IEC. Elements and attributes that have been added to the namespace compared to earlier revisions of this document are documented in subclause [5.2.3](#c5-2-3).

The MPD shall be authored such that, after XML attributes or elements in the DASH namespace but not in the XML schema documented in Annex B are removed, the result is a valid XML document formatted according to that schema and that conforms to this document.

In addition, the MPD shall be authored such that, after XML attributes or elements in the other namespaces than the DASH namespace are removed, the result is a valid XML document formatted according to that schema and that conforms to this document.

NOTE 2 Based on the last two paragraphs, if DASH Clients remove all XML attributes and elements from the MPD in the DASH namespace and in other namespaces that are not in the XML schema documented in Annex B, the MPD results in a valid XML document which complies with this document. The DASH Client can use such a resulting MPD for presentation of a conforming Media Presentation.

In addition, rules for authoring of MPDs conforming to a specific profile are provided in 8. Certain profiles as defined in 8 may permit ignoring certain elements and attributes. However, this has no effect on the general MPD conformance rules defined in this subclause.

Following XML rules, the MPD document shall contain exactly one MPD element as specified in subclause [5.2.3.5](#c5-2-3-5).

<!--PB printed 22-->

The MIME type of the MPD document is defined in Annex C.

The encoding of the MPD shall be UTF-8 as defined in IETF RFC 3629. All data provided in extension namespaces shall be UTF-8 as defined in IETF RFC 3629. If binary data needs to be added, it shall be included in Base64 as described in IETF RFC 4648 within a UTF-8 encoded element with a proper name space or identifier, such that an XML parser knows how to process or ignore it.

The delivery of the MPD is outside the scope of this document. However, if the MPD is delivered over HTTP, then the MPD document may be transfer encoded for transport, as described in IETF RFC 7230.

NOTE 3 As an example, the GZip algorithm as defined in IETF RFC 1952<sup>[3]</sup> can be used for Transfer-encoding.

NOTE 4 MPD encryption is not a normative part of this document. However, if operating in an insecure environment and required by the content/service provider, elements and attributes of MPD can be encrypted to protect their confidentiality by using the syntax and processing rules specified in the “XML Encryption Syntax and Processing” by W3C<sup>[8]</sup>.

NOTE 5 MPD integrity protection is not a normative part of this document. However, if operating in an insecure environment and required by the content/service provider, the digital signing and verification procedures specified in the “XML Signature Syntax and Processing” by W3C<sup>[9]</sup> can be used to protect data origin authenticity and integrity of the MPD.

Selected MPD examples are provided in Annex G.

<a id="c5-2-2"></a>

### 5.2.2 Schema

The initial part of the XML schema of the MPD is provided below, including namespace and other definitions. Specific types, elements and attributes are introduced in the remainder of this subclause. The complete normative MPD schema is provided in Annex B. In case of inconsistencies, the schema in Annex B takes precedence both over the XML syntax snippets provided in this clause and all prose text in this document.

Implementors are supported by files available at: https://standards.iso.org/iso-iec/23009/-1/ed-5/en. These files include the schema as well as all examples provided in Annex G.

```xml
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xlink="http://www.w3.org/1999/xlink"
xmlns="urn:mpeg:dash:schema:mpd:2011" targetNamespace="urn:mpeg:dash:schema:mpd:2011"
elementFormDefault="qualified" attributeFormDefault="unqualified">
    <xs:import namespace="http://www.w3.org/1999/xlink"
schemaLocation="http://www.w3.org/XML/2008/06/xlink.xsd">
        <xs:annotation>
              <xs:documentation xml:lang="en">
              xlink import. URL is per https://www.w3.org/TR/xlink/  Users are encouraged to store a local copy
        </xs:documentation>
        </xs:annotation>
    </xs:import>
    <xs:annotation>
        <xs:appinfo>Media Presentation Description</xs:appinfo>
        <xs:documentation xml:lang="en">
              This Schema defines the Media Presentation Description for MPEG-DASH.
        </xs:documentation>
    </xs:annotation>
    <xs:element name="MPD" type="MPDtype">
        <xs:annotation>
              <xs:documentation xml:lang="en">
              MPD: main element
        </xs:documentation>
        </xs:annotation>
    </xs:element>
    <xs:complexType name="MPDtype">
        <xs:annotation>
              <xs:documentation xml:lang="en">
              MPD Type
        </xs:documentation>
        ...
        </xs:annotation>
<!--PB printed 23-->
    </xs:complexType>
</xs:schema>
```

<a id="c5-2-3"></a>

### 5.2.3 Elements and Attributes added in revisions and amendments

<a id="c5-2-3-1"></a>

#### 5.2.3.1 Overview

In amendments and revisions of this document, the schema defined in Annex B may have been extended. In order to track this, subclause [5.2.3](#c5-2-3) tracks the addition of elements and attributes.

By following the rules in subclause [5.2.1](#c5-2-1), a single MPD can be authored for clients that implement different versions of this document.

<a id="c5-2-3-2"></a>

#### 5.2.3.2 Elements and Attributes added in the second edition of this document (ISO/IEC 23009-1:2014)

ISO/IEC 23009-1:2014 added the following elements and attributes to the schema defined in Annex B compared to the 2012 edition (ISO/IEC 23009-1:2012/Cor 1:2013):
- `MPD@publishTime`
- `MPD.EssentialProperty`
- `MPD.SupplementalProperty`
- `Period.AssetIdentifier`
- `Period.EventStream`
- `Period.SupplementalProperty`
- `RepresentationBase.InbandEventStream`
- `SegmentBase@availabilityTimeOffset`
- `SegmentBase@availabilityTimeComplete`
- `BaseURL@availabilityTimeOffset`
- `BaseURL@availabilityTimeComplete`
- `Subset@id`
- `SegmentTimeline.S@n`

<a id="c5-2-3-3"></a>

#### 5.2.3.3 Elements and Attributes added in the third edition of this document (ISO/IEC 23009-1:2019)

ISO/IEC 23009-1:2019 added the following elements and attributes to the schema defined in Annex B compared to the 2014 revision (ISO/IEC 23009-1:2014) of this document:
- `MPD.UTCTiming`
- `Period.GroupLabel`
- `Period.Preselection`
- `Period.EmptyAdaptationSet`

<!--PB printed 24-->
- `RepresentationBase.Switching`
- `RepresentationBase.RandomAccess`
- `RepresentationBase.GroupLabel`
- `RepresentationBase.Label`
- `RepresentationBase@selectionPriority`
- `Representation@tag`
- `Representation@associationId`
- `Representation@associationType`
- `SegmentBase@presentationDuration`
- `SegmentBase@timeShiftBufferDepth`
- `SegmentTimeline.S@n`
- `SegmentTimeline.S@k`

<a id="c5-2-3-4"></a>

#### 5.2.3.4 Elements and Attributes added in the fourth edition (ISO/IEC 23009-1)

This revision adds the following elements and attributes to the schema defined in Annex B compared to the 2019 revision (ISO/IEC 23009-1:2019) of this document:
- `MPD.ServiceDescription`
- `MPD.InitializationSet`
- `MPD.InitializationGroup`
- `MPD.InitializationPresentation`
- `MPD.LeapSecondInformation`
- `Period.ServiceDescription`
- `EventStream@presentationTimeOffset`
- `Event@contentEncoding`
- `AdaptationSet@initializationSetRef`
- `RepresentationBase.ProducerReferenceTime`
- `RepresentationBase.ContentPopularityRate`
- `Preselection@order`

<!--PB printed 25-->
- `SegmentBase.FailoverContent`
- `SegmentBase@eptDelta`
- `MultiSegmentBase@endNumber`
- `BaseURL@timeShiftBufferDepth`

<a id="c5-2-3-5"></a>

#### 5.2.3.5 Elements and Attributes added in the fifth edition (ISO/IEC 23009-1:2021)

This revision adds the following elements and attributes to the schema defined in Annex B compared to the 2020 revision (ISO/IEC 23009-1:2020) of this document:
- `MPD.ContentProtection`
- `MPD.PatchLocation`
- `Period.ContentProtection`
- `AdaptationSet@initializationPrincipal`
- `RepresentationBase.Resync`
- `RepresentationBase@containerProfiles`
- `Representation.ExtendedBandwidth`
- `ContentProtection@ref`
- `ContentProtection@refId`
- `ContentProtection@robustness`
- `RepresentationBase.OutputProtection`

## Transcription notes

> **Spec note:** 5.1: "The data model that constitutes a Media Presentation is defined in subclause 5.2.3.5" - 5.2.3.5 is the fifth-edition additions list; the data model is 5.3. Reproduced as printed.
>
> **Spec note:** 5.2.3.4 heading reads "(ISO/IEC 23009-1)" without a year, as printed. In the 5.2.3.x lists, bold element names and plain attribute names are written as `Element@attribute` / `Element.Child` code spans.
>
> **Spec note:** 5.2.2: the schema fragment is reproduced with whitespace normalised; the PDF shows irregular indentation. The `...` lines are in the PDF.
