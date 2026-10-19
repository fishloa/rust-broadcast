# DASH-IF Interoperability Points v4.3 — trick mode support (§3.2.9)

Source: `private/specs/dashif_iop_v4.3_guidelines.pdf` (DASH-IF IOP v4.3, 254 pages; fetched from
`https://dashif.org/docs/DASH-IF-IOP-v4.3.pdf`), §3.2.9, printed pp. 20-21 (PDF pp. 41-42), **checked against the
rendered pages**. Verbatim below (line breaks of the PDF removed).

> 3.2.9. Trick Mode Support — Trick Modes are used by DASH clients in order to support fast forward, seek, rewind
> and other operations in which typically the media, especially video, is displayed in a speed other than the normal
> playout speed. In order to support such operations, it is recommended that the content author adds
> Representations at lower frame rates in order to support faster playout with the same decoding and rendering
> capabilities.
>
> However, Representations targeted for trick modes are typically not be suitable for regular playout. If the
> content author wants to explicitly signal that a Representation is only suitable for trick mode cases, but not
> for regular playout, the following is recommended:
>
> - add one or multiple Adaptation Sets that that only contains trick modes Representations
> - annotate each Adaptation Set with an **EssentialProperty** descriptor or **SupplementalProperty** descriptor
>   with URL **"http://dashif.org/guidelines/trickmode"** and the **@value the value of @id attribute of the
>   Adaptation Set to which these trick mode Representations belong**. The trick mode Representations must be
>   time-aligned with the Representations in the main Adaptation Set. The value may also be a white-space
>   separated list of @id values. In this case the trick mode Adaptation Set is associated to all Adaptation Sets
>   with the values of the @id.
> - signal the playout capabilities with the attribute **@maxPlayoutRate for each Representation** in order to
>   indicate the accelerated playout that is enabled by the signaled codec profile and level.
> - If the Representation is encoded without any coding dependency on the elementary stream level, i.e. each sample
>   is a SAP type 1, then it is recommended to set the @codingDependency attribute to FALSE.
> - If multiple trick mode Adaptation Sets are present for one main Adaptation Set, then sufficient signaling
>   should be provided to differentiate the different trick mode Adaptation Sets. Different Adaptation Sets for
>   example may be provided as thumbnails (low spatial resolution), for fast forward or rewind (no coding
>   dependency with @codingDependency set to false and/or lower frame rates), longer values for @duration to
>   improve download frequencies or different @maxPlayoutRate values. Note also that the @bandwidth value should
>   be carefully documented to support faster than real-time download of Segments.
>
> If an Adaptation Set in annotated with the EssentialProperty descriptor with URI
> "http://dashif.org/guidelines/trickmode then the DASH client shall not select any of the contained
> Representations for regular playout.
>
> For trick modes for live services, the same annotation should be used. More details on service offerings are
> provided in section 4.10.

(The IOP v4.3 table of defined identifiers, near the front of the document, lists
`http://dashif.org/guidelines/trickmode` as "Defines a trick mode Adaptation, Functionality, 3.2.9".)

## What this settles, and what it does not

1. The only trick-mode **scheme URI** defined by any specification held in the private repo is
   `http://dashif.org/guidelines/trickmode`. The URN `urn:mpeg:dash:trickmode:2016` (what `transmux::dash`
   emits today, `TRICKMODE_SCHEME`) appears in **none** of: ISO/IEC 23009-1:2022 (5th ed.), ETSI TS 103 285
   v1.4.1, DASH-IF IOP v4.3. It is therefore unverified.
2. `@value` is "the value of @id attribute of the Adaptation Set to which these trick mode Representations
   belong" — i.e. the id of the **main** AdaptationSet, and (23009-1 Table 5) that id is an unsigned integer
   (`dash-23009-1-2022-ids.md`). Free-form values such as `main-0` reference nothing.
3. IOP's clause is a *recommendation* ("it is recommended ... the following is recommended"); with an
   EssentialProperty the client "shall not select" the contained Representations for regular playout.
4. `@maxPlayoutRate` is signalled "for each Representation" in IOP; ETSI TS 103 285 §10.6.2.1 (page-verified earlier,
   `isobmff-tkhd.md` neighbours) says the Representations "will indicate @maxPlayoutRate, and the Adaptation Set
   that contains them will include the attribute @codingDependency="false", and may contain an Essential
   Descriptor"; a Role descriptor with value "alternative" "may be used". TS 103 285 text read from the md only
   (not page-rendered in this pass).

## Not covered

- Whether ISO/IEC 23009-1 editions other than 2022 (or an amendment) define `urn:mpeg:dash:trickmode:2016`:
  no held source does, so its provenance is unknown.
- IOP §4.10 (trick modes for live services) and §4.10.4 (live-to-VoD conversion): not transcribed.
