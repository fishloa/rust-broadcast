---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Annex K (normative) - DASH Service Description, K.1 to K.5"
pdf_pages: "316-326"
printed_pages: "308-318"
kind: implementer notes in original wording (not a transcription)
verbatim_source: private/specs-verified/dash/23009-1/annex-k-service-description.md
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION-C1.md
---

# Annex K - DASH Service Description (implementer notes)

## K.1-K.2 Purpose and model (pp. 308-309)

Without guidance each client picks its own rate adaptation, buffering and therefore latency, so the same service behaves differently on
different players. The annex lets the service provider state targets that the client or its embedding application can honour. It defines:
an abstract parameter set (K.3), its carriage in the MPD (K.4), and example usage (K.5). Using the parameters is up to clients unless an
application standard requires more. A description can be scoped to specific clients or environments.

Figure K.1 (p. 309; PDF p. 317) extends the client model with an extra input "Service Description" feeding playback control. The
information may arrive through the MPD (K.4), application signalling or be generated in the application. Redraw: MPD and segment data go
to the DASH access engine; MPD metadata drive Selection Logic, which yields the selected Adaptation Sets; the Service Description feeds
Playback Control alongside; media engine outputs media + timing and events.

## K.3 Abstract semantics (pp. 309-313)

Abstract syntax types: Integer, Real, Boolean, Enum, String, Object (unordered unique string keys), List (ordered; entries unnamed in
concrete syntax). A service description is a named list of entries; delivery and update timing are out of scope.

### K.3.2 Service Latency (Table K.1)

| Parameter | Type | Gloss |
|---|---|---|
| ReferenceID | Integer | which Producer Reference Time element (by id) the latency is measured against when several exist |
| TargetLatency | Integer (ms) | preferred presentation latency relative to the producer reference time; aim for it as far as player capability and conditions allow; may be reachable only by low-latency players on good networks |
| MaximumLatency | Integer (ms) | provider's wish that content not be presented if latency would exceed this |
| MinimumLatency | Integer (ms) | provider's lower bound, for example to stay consistent with second-screen apps or overlays |
| QualityType | URI | how to read `Quality`; absent means linear scale |
| Entry | Object | list of latency/quality pairs |
| Entry.Latency | Integer (ms) | latency value |
| Entry.Quality | Integer | quality at that latency, 0 lowest to 100 highest |

### K.3.3 Playback Rate (Table K.2)

| Key | Type | Gloss |
|---|---|---|
| MaxPlaybackRate | Real | largest rate the provider considers appropriate for automatic latency/buffer adjustment in normal play; 1.0 is nominal |
| MinPlaybackRate | Real | smallest such rate |

### K.3.4 Operating Quality (Table K.3)

| Key | Type | Gloss |
|---|---|---|
| MediaType | string | `video`, `audio` or `any` |
| MinQualityRanking | Integer | smallest desired `@qualityRanking` among played Representations |
| MaxQualityRanking | Integer | largest desired `@qualityRanking` |
| TargetQualityRanking | Integer | target `@qualityRanking` |
| QualityRankingType | URL | how to read `@qualityRanking`; absent means linear |
| MaxQualityDifference | Integer | recommended maximum `@qualityRanking` spread among Representations played together (typically within one Preselection) |

### K.3.5 Operating Bandwidth (Table K.4)

| Key | Type | Gloss |
|---|---|---|
| MediaType | string | `video`, `audio`, `any` (each type individually) or `all` (aggregate of all types) |
| MinBandwidth | Integer | smallest desired aggregate `@bandwidth` of concurrently played Representations of that type |
| MaxBandwidth | Integer | largest desired aggregate |
| TargetBandwidth | Integer | desired aggregate (the printed description says "largest desired aggregated value", same wording as Max; recorded as printed) |

## K.4 MPD carriage (pp. 313-318)

### K.4.1 Rules

- A `Scope` descriptor (application defined) can restrict the description to particular clients or operating modes. A client that does not
  recognise any of the scope descriptors ignores the description. No `Scope` means all clients.
- The provider must keep the description self-consistent (for example minimum <= target <= maximum latency). Clients are expected to ignore
  descriptions with inconsistent or unachievable settings.

### K.4.2 Element inventory (Tables K.5 to K.9)

| Element / attribute | Use / cardinality | Type | Default | Maps to |
|---|---|---|---|---|
| `ServiceDescription` | 0..N in the MPD (parent placement per clause 5.3.1, see [05-3-1-hierarchy.md](05-3-1-hierarchy.md)) | element | - | whole description |
| `ServiceDescription@id` | M | unsigned int | - | unique id among the MPD's service descriptions |
| `ServiceDescription.Scope` | 0..N | Descriptor | - | scope selector |
| `ServiceDescription.Latency` | 0..N | LatencyType | - | K.3.2 |
| `Latency@referenceId` | CM | unsigned int | - | ReferenceID |
| `Latency@target` | O | unsigned int | - | TargetLatency |
| `Latency@max` | O | unsigned int | - | MaximumLatency |
| `Latency@min` | O | unsigned int | - | MinimumLatency |
| `Latency.QualityLatency` | 0..N | list of latency/quality integer pairs (comma separated pairs, white-space separated within a pair) | - | Entry list |
| `QualityLatency@type` | O | anyURI | - | QualityType |
| `ServiceDescription.PlaybackRate` | 0..N | PlaybackRateType | - | K.3.3 |
| `PlaybackRate@max` | O | double | - | MaxPlaybackRate |
| `PlaybackRate@min` | O | double | - | MinPlaybackRate |
| `ServiceDescription.OperatingQuality` | 0..N | OperatingQualityType | - | K.3.4 |
| `OperatingQuality@mediaType` | OD | enum `video` `audio` `any` | `any` | MediaType |
| `OperatingQuality@min` / `@max` / `@target` | O | unsigned int | - | Min/Max/TargetQualityRanking |
| `OperatingQuality@type` | O | anyURI | - | QualityRankingType |
| `OperatingQuality@maxDifference` | O | unsigned int | - | MaxQualityDifference |
| `ServiceDescription.OperatingBandwidth` | 0..N | OperatingBandwidthType | - | K.3.5 |
| `OperatingBandwidth@mediaType` | OD | enum `video` `audio` `any` `all` | `all` | MediaType |
| `OperatingBandwidth@min` / `@max` / `@target` | O | unsigned int | - | Min/Max/TargetBandwidth |

Units: latency values are milliseconds (K.3.2). All complex types allow other-namespace elements and attributes (lax).

> **Spec note:** Table K.5 describes `OperatingBandwidth` as "operating quality targets" (copy of the `OperatingQuality` text). Recorded
> as printed; the semantics are those of K.3.5.

The syntax of K.4.3 (types `ServiceDescriptionType`, `LatencyType`, `UIntPairsWithIDType`, `PlaybackRateType`, `OperatingQualityType`,
`OperatingBandwidthType`) is not copied; take it from `DASH-MPD.xsd` in the Ed.6 schema directory (see [annex-b-schema.md](annex-b-schema.md)).

Where the element sits: see the MPD element table in [05-3-1-hierarchy.md](05-3-1-hierarchy.md) (Ed.5 places `ServiceDescription` as a child of `MPD`).

## K.5 Example client usage (p. 318)

- With a `TargetLatency` and a Producer Reference Time of the considered type, aim to present each sample at
  (producer wall-clock time for that sample) + target latency.
- Monitor continuously; within `MinPlaybackRate`..`MaxPlaybackRate` adjust speed to catch up to or hold the target latency.
- If provided, keep quality and bandwidth within the min/max ranges of `OperatingQuality`/`OperatingBandwidth`.

Own-words control loop for a low-latency client:

```
live_latency = now_wallclock - (producer_time_of_playing_sample)
err = live_latency - TargetLatency
rate = clamp(1.0 + k * err, MinPlaybackRate, MaxPlaybackRate)       # k a tuning constant of the implementation
if live_latency > MaximumLatency: seek toward the live edge (do not present stale content)
if live_latency < MinimumLatency: slow down or hold (never present earlier than MinimumLatency)
```
(Only the bounds and meanings are specified by the annex; the proportional rule and `k` are an implementation choice.)

## Fresh minimal example (ours, for tests)

```xml
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic" availabilityStartTime="2026-01-01T00:00:00Z"
     minimumUpdatePeriod="PT2S" minBufferTime="PT1S" profiles="urn:mpeg:dash:profile:isoff-live:2011">
  <ServiceDescription id="1">
    <Latency target="3500" min="2000" max="8000" referenceId="7"/>
    <PlaybackRate min="0.96" max="1.04"/>
    <OperatingBandwidth mediaType="video" min="1000000" max="6000000" target="3000000"/>
  </ServiceDescription>
  <Period id="p0" start="PT0S"/>
</MPD>
```
