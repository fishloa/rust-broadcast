---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Annex G (informative) - MPD Examples and MPD Usage, G.1 to G.22"
pdf_pages: "264-291"
printed_pages: "256-283"
kind: implementer notes in original wording (not a transcription); example MPDs below are NEW, written for this repository
verbatim_source: private/specs-verified/dash/23009-1/annex-g-examples.md
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION-C1.md
---

# Annex G - MPD examples and usage (outline plus fresh test MPDs)

## How the annex is organised

Annex G has 22 numbered examples (pp. 256-283). The standard publishes every example as a file named `example-G<subsection>[<order>].mpd` at
`https://standards.iso.org/iso-iec/23009/-1/ed-5/en`; the Ed.6 bundle in `private/specs/iso_iec_23009-1_dash_2026_ed6_schema/` holds
machine-readable copies (`example_G1.mpd` ... `example_G30-2.mpd`, `example_G21_patch.mpp`, `example_G11_remote.period.xml`, ...). Per project
rule the spec's example documents are **not** copied here: use those files as conformance inputs from the private tree, and use the
**fresh** MPDs at the end of this file for public tests. The tool-converted original text (XML collapsed onto single lines by the converter) is
in `private/specs-verified/dash/23009-1/annex-g-examples.md`.

Note: the Ed.6 bundle contains examples G.23 to G.30 and H.x that are beyond the Ed.5 text (Ed.5 stops at G.22); see
[`ED6-SCHEMA-DIFF.md`](ED6-SCHEMA-DIFF.md) from PR #1516.

## Per-example outline

| # | Title (our paraphrase) | Printed pages | Profile claimed | Elements/attributes exercised |
|---|---|---|---|---|
| G.1 | Static on-demand, two CDNs, languages, subtitles, protection | 256-257 | `isoff-on-demand:2011` | two `BaseURL` at MPD level; per-Representation `BaseURL`; `@subsegmentAlignment`, `@subsegmentStartsWithSAP`; `ContentProtection` with a UUID scheme; `Role` (`dub`, and a subtitle role using the non-`:2011` scheme `urn:mpeg:dash:role`); `lang`; TTML AdaptationSet; six video Representations 256 kbit/s to 2 Mbit/s |
| G.2 | Dynamic live, about 2 s Segments | 257-258 | `isoff-live:2011` | `type=dynamic`, `minimumUpdatePeriod`, `timeShiftBufferDepth`, `availabilityStartTime`, `publishTime`; `SegmentTemplate` with `$Time$` and `SegmentTimeline` (`S@t,d,r`); `$Bandwidth%` printed with a stray `%` (typo kept in the source); `segmentAlignment="0"` for audio; prose lists first resulting URLs (timeline times 0, 180180, 360360 ...) |
| G.3 | MPEG-2 TS Simple profile | 258-259 | `mp2t-simple:2011` | `video/mp2t`; `ContentComponent` (video + two audio langs); `SegmentTemplate` with `@index` and `@bitstreamSwitching` templates, `$Number%05d$`; `bitstreamSwitching`, `subsegmentAlignment`; `maxSegmentDuration`; `Period@duration` |
| G.4 | Stereo views from three cameras | 260-261 | `isoff-main:2011` | two Periods; Period-level `SegmentList` with `Initialization`; `Role` with scheme `urn:mpeg:dash:stereoid:2011` (values like `l1 r0`); duplicate AdaptationSet with the same `Representation@id` for 2D clients; `@dependencyId` for MVC enhancement views |
| G.5 | SVC alternative streams | 261-262 | `isoff-on-demand:2011` | three Representations chained by `@dependencyId` (space-separated list); `SegmentBase@indexRange`; AdaptationSet `minBandwidth`/`maxBandwidth` |
| G.6 | Trick play with Sub-Representations | 262-263 | `isoff-on-demand:2011` | `ContentComponent`; `SubRepresentation@level`, `@dependencyLevel`, `@contentComponent`, `@maxPlayoutRate`; `SegmentBase@indexRange` |
| G.7 | Content protected by multiple schemes | 263-264 | `isoff-on-demand:2011` | `ContentProtection` with `urn:mpeg:dash:mp4protection:2011` and `@value`; custom DRM scheme with foreign-namespace children (`drm:License`, `drm:Content`); clear subtitles |
| G.8 | Role + Viewpoint matching (partial MPD) | 264-265 | not a complete MPD | `@group`; `Role` values `main`, `supplementary`; `Viewpoint` equality (scheme and value) deciding which sets may be combined. The XML comment states it will not validate |
| G.9 | Event messaging | 265-267 | `isoff-live:2011` | Period `EventStream` (`@schemeIdUri`, `@timescale`, `@value`) with `Event@presentationTime,duration,id,messageData`; `InbandEventStream` for `urn:mpeg:dash:event:2012` (value 1, MPD validity expiration) and an application scheme; sample `emsg` field values (`timescale` 1000, `presentation_time_delta` 8000, `event_duration` 0xFFFFFFFF, XML message payload) |
| G.10 | Adaptation Set linking (mosaic) | 267-268 | `isoff-live:2011` | `SupplementalProperty` SRD (`urn:mpeg:dash:srd:2014`), `urn:mpeg:dash:sai:2014`; `EmptyAdaptationSet` carrying `EssentialProperty urn:mpeg:dash:mpd-as-linking:2015` with a URL fragment (`#period=..&as=..`, `timeOffset`); `SegmentBase@indexRange,indexRangeExact` |
| G.11 | Remote Period entity (xlink) | 268-270 | `isoff-live:2011` | `Period@xlink:href`, `@xlink:actuate="onRequest"`; remote Period file with its own `@start`; `AssetIdentifier`; `SegmentTemplate@presentationTimeOffset`, `@startNumber` continuing across Periods; `AudioChannelConfiguration` |
| G.12 | Directory-limit workaround | 270-271 | `isoff-main:2011` | Period-level `BaseURL` + `SegmentTemplate` changed per Period; `urn:mpeg:dash:period-continuity:2015` `SupplementalProperty`; matching `AdaptationSet@id`; `presentationTimeOffset` equal to previous Period's length x timescale |
| G.13 | Data URLs | 271-272 | `isoff-ext-live:2014` | `SegmentTemplate@initialization` as RFC 2397 `data:video/mp4;base64,...`; moving `@initialization` from set level to Representation level; `InbandEventStream` with a tag URI scheme; `maxPlayoutRate` |
| G.14 | Live with timing | 272-273 | `isoff-ext-live:2014` | `UTCTiming` (`urn:mpeg:dash:utc:http-xsdate:2014`); `LeapSecondInformation` (`availabilityStartLeapOffset`, `nextAvailabilityStartLeapOffset`, `nextLeapChangeTime`); large `presentationTimeOffset`; `maxSegmentDuration`; `minimumUpdatePeriod=PT1H` with inband expiry events |
| G.15 | Preselection by descriptor | 273-274 | `isoff-live:2011` | `EssentialProperty urn:mpeg:dash:preselection:2016` with values such as `"1,2 3"` (tag, then component ids) on a main set and bare descriptors on dependent sets; `Role` main/dub; MPEG-H audio codec string |
| G.16 | Preselection by element | 274-275 | `isoff-live:2011` | `Preselection@id,tag,lang,preselectionComponents` (space-separated Adaptation Set ids); `Label`, `Role`, `AudioChannelConfiguration` inside `Preselection` |
| G.17 | Preselection with ContentComponent | 275-276 | `isoff-live:2011` | single multiplexed audio Representation described by three `ContentComponent`s; `Preselection` selecting components by id |
| G.18 | Low-latency streaming | 276-277 | `isoff-ext-live:2014` | `ServiceDescription` with `Scope`, `Latency@min,max,target,referenceId`, `PlaybackRate@min,max` (see [annex-k-service-description.md](annex-k-service-description.md)); `ProducerReferenceTime` (`@id,wallClockTime,presentationTime`) with nested `UTCTiming`; `SegmentTemplate@availabilityTimeOffset`, `@availabilityTimeComplete="false"`; `LeapSecondInformation` |
| G.19 | CMAF content profile | 277-278 | `cmaf:2019` + `isoff-live:2011` | `@profiles` listing both; `mimeType="video/mp4 profiles='cmfc'"` as printed; `segmentProfiles="cmfs cmff"`; `SegmentTimeline` with `S@r`; see [08-profiles.md](08-profiles.md) 8.12 |
| G.20 | Resynchronization | 278-279 | `urn:dvb:dash:profile:dvb-dash:2014` | `Resync@type,dT,dImin,dImax,marker` on AdaptationSet and Representation; worked byte arithmetic: minimum/maximum distance in bytes = `@bandwidth * dImin` (or `dImax`) per second-of-timescale unit as the prose computes (for bandwidth 500000 and dImin 0.03125: 15,625 bytes; max with 0.09375: 46,875); `dT` read against `@timescale` (500000 with timescale 1000000 = 500 ms); `ProducerReferenceTime@inband`, `@type`; `availabilityTimeOffset`; `qualityRanking`; client use of marker per [annex-a-client-behaviour.md](annex-a-client-behaviour.md) A.12.3 |
| G.21 | MPD patch | 280-282 | `isoff-live:2011` | MPD with `PatchLocation@ttl`; separate patch document (`application/dash-patch+xml`, namespace `urn:mpeg:dash:schema:mpd-patch:2020`): root `Patch@mpdId,originalPublishTime,publishTime`; operations `replace`/`add` with XPath `sel`; adds `S` entries to `SegmentTimeline` |
| G.22 | Failover content | 282-283 | `isoff-live:2011` | `SegmentTemplate/FailoverContent/FCS@t,d` on one Representation to signal a missing Segment; prose: with Segment `i` missing in A but present in B-D, a client on A fetches the same-numbered Segment from another Representation for that slot |

### Annex I examples (pp. 298-302) {#annex-i-examples}

Four short MPDs on static on-demand profile: I.2.4.1 inherit whole MPD-URL query (`$querypart$`, `useMPDUrlQuery`), I.2.4.2 server-side
`xlink` `UrlQueryInfo` with `queryString`, I.2.4.3 client feedback through a URN placeholder plus a companion `SupplementalProperty` for that
URN, I.2.4.4 selecting a single parameter with `token=$query:token$`. Semantics in [annex-i-urlparam.md](annex-i-urlparam.md).

### Oddities in the printed examples (recorded, not corrected)

- G.2/G.9/G.13-style templates print `$Bandwidth%` where `$Bandwidth$` is meant; the G.2 prose also writes `50000` in one URL and `500000` in the next.
- G.3 prose shows `$Number%05$`, the MPD text uses `$Number%05d$`.
- G.1 declares a mixed-quality subtitle role scheme without `:2011`.
- G.20 prints a `publishTime` split across a line break and `DASH￾MPD.xsd`/`init￾stream` with a soft-hyphen artefact in the text layer; the render should be consulted before using those strings.
- G.9 shows the application event scheme as `urn:org:example:xscte3` in the MPD prose and `urn:org:example:xscte35` in the `emsg` field list.

## Fresh minimal example MPDs (ours, for tests)

Each is well-formed and uses only features named in the clauses above. Hosts use the reserved `.test` TLD.

### F1. Static on-demand with `BaseURL` + `SegmentBase` (cf. G.1, G.5)

```xml
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT30S" minBufferTime="PT2S"
     profiles="urn:mpeg:dash:profile:isoff-on-demand:2011">
  <BaseURL>https://a.test/vod/</BaseURL>
  <Period>
    <AdaptationSet mimeType="video/mp4" codecs="avc1.4d401f" subsegmentAlignment="true" subsegmentStartsWithSAP="1">
      <Representation id="lo" bandwidth="400000" width="426" height="240"><BaseURL>lo.mp4</BaseURL><SegmentBase indexRange="800-1199"/></Representation>
      <Representation id="hi" bandwidth="1600000" width="1280" height="720"><BaseURL>hi.mp4</BaseURL><SegmentBase indexRange="900-1499"/></Representation>
    </AdaptationSet>
    <AdaptationSet mimeType="audio/mp4" codecs="mp4a.40.2" lang="en" subsegmentAlignment="true" subsegmentStartsWithSAP="1">
      <Representation id="aud" bandwidth="96000"><BaseURL>aud.mp4</BaseURL><SegmentBase indexRange="700-1099"/></Representation>
    </AdaptationSet>
  </Period>
</MPD>
```

### F2. Dynamic live with `$Number$` and a fixed duration (cf. G.2)

```xml
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic" availabilityStartTime="2026-01-01T00:00:00Z" publishTime="2026-01-01T01:00:00Z"
     minimumUpdatePeriod="PT4S" timeShiftBufferDepth="PT60S" suggestedPresentationDelay="PT8S" minBufferTime="PT4S"
     profiles="urn:mpeg:dash:profile:isoff-live:2011">
  <Period id="p0" start="PT0S">
    <AdaptationSet mimeType="video/mp4" segmentAlignment="true" startWithSAP="1">
      <SegmentTemplate timescale="1000" duration="4000" startNumber="1" initialization="$RepresentationID$/init.mp4" media="$RepresentationID$/$Number$.m4s"/>
      <Representation id="v1" codecs="avc1.4d401f" width="640" height="360" bandwidth="900000"/>
    </AdaptationSet>
  </Period>
</MPD>
```
Expected segment `i` availability (A.3.4) with `ATO = 0`: `availabilityStartTime + (i-1)*4 s + 4 s`; the 3rd Segment becomes available at
`2026-01-01T00:00:12Z`.

### F3. Live with `SegmentTimeline`, `$Time$`, two Periods and continuity (cf. G.12)

```xml
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic" availabilityStartTime="2026-02-01T00:00:00Z" publishTime="2026-02-01T00:30:00Z"
     minimumUpdatePeriod="PT2S" minBufferTime="PT2S" profiles="urn:mpeg:dash:profile:isoff-live:2011">
  <Period id="a" start="PT0S" duration="PT20S">
    <AdaptationSet id="1" mimeType="video/mp4" segmentAlignment="true" startWithSAP="1">
      <SegmentTemplate timescale="90000" initialization="a/init.mp4" media="a/$Time$.m4s">
        <SegmentTimeline><S t="0" d="180000" r="9"/></SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v" codecs="avc1.4d401f" width="640" height="360" bandwidth="800000"/>
    </AdaptationSet>
  </Period>
  <Period id="b" start="PT20S">
    <AdaptationSet id="1" mimeType="video/mp4" segmentAlignment="true" startWithSAP="1">
      <SupplementalProperty schemeIdUri="urn:mpeg:dash:period-continuity:2015" value="a"/>
      <SegmentTemplate timescale="90000" presentationTimeOffset="1800000" initialization="b/init.mp4" media="b/$Time$.m4s">
        <SegmentTimeline><S t="1800000" d="180000" r="-1"/></SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v" codecs="avc1.4d401f" width="640" height="360" bandwidth="800000"/>
    </AdaptationSet>
  </Period>
</MPD>
```
(Period `a` has 10 Segments of 2 s: `S r=9`.)

### F4. Events: `EventStream` and `InbandEventStream`

Note: the Ed.6 XSD rejects an `Event@messageData` attribute (Ed.5 clause 5.10.2 and the G.9 example use it); this fresh example uses element content only so that it validates against the Ed.6 schema. See `ED6-SCHEMA-DIFF.md`.

```xml
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT60S" minBufferTime="PT2S"
     profiles="urn:mpeg:dash:profile:isoff-live:2011">
  <Period id="p">
    <EventStream schemeIdUri="urn:example:test:events" value="marker" timescale="1000">
      <Event presentationTime="5000" duration="2000" id="1">hello</Event>
      <Event presentationTime="15000" id="2">plain text body</Event>
    </EventStream>
    <AdaptationSet mimeType="video/mp4" segmentAlignment="true" startWithSAP="1">
      <InbandEventStream schemeIdUri="urn:mpeg:dash:event:2012" value="1"/>
      <SegmentTemplate timescale="1000" duration="2000" startNumber="1" initialization="i.mp4" media="s$Number$.m4s"/>
      <Representation id="v" codecs="avc1.4d401f" width="640" height="360" bandwidth="700000"/>
    </AdaptationSet>
  </Period>
</MPD>
```

### F5. Preselection element (cf. G.16)

```xml
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT10S" minBufferTime="PT2S"
     profiles="urn:mpeg:dash:profile:isoff-live:2011">
  <Period id="p">
    <AdaptationSet id="10" mimeType="audio/mp4" codecs="mp4a.40.2" segmentAlignment="true" startWithSAP="1">
      <SegmentTemplate timescale="48000" duration="96000" startNumber="1" initialization="me-init.mp4" media="me-$Number$.m4s"/>
      <Representation id="me" bandwidth="64000"/>
    </AdaptationSet>
    <AdaptationSet id="11" mimeType="audio/mp4" codecs="mp4a.40.2" lang="fr" segmentAlignment="true" startWithSAP="1">
      <SegmentTemplate timescale="48000" duration="96000" startNumber="1" initialization="fr-init.mp4" media="fr-$Number$.m4s"/>
      <Representation id="fr" bandwidth="32000"/>
    </AdaptationSet>
    <Preselection id="1" tag="fr" lang="fr" preselectionComponents="10 11"><Label>Francais</Label></Preselection>
  </Period>
</MPD>
```

### F6. CMAF profile (cf. G.19)

```xml
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT12S" minBufferTime="PT2S"
     profiles="urn:mpeg:dash:profile:cmaf:2019">
  <Period id="1">
    <AdaptationSet id="1" contentType="video" mimeType="video/mp4" codecs="avc1.4d401f" containerProfiles="cmfc" maxWidth="1280" maxHeight="720"
                   segmentAlignment="true" startWithSAP="1">
      <SegmentTemplate timescale="30" initialization="$RepresentationID$/init" media="$RepresentationID$/$Number$">
        <SegmentTimeline><S t="0" d="60" r="5"/></SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v1" bandwidth="500000" width="640" height="360"/>
      <Representation id="v2" bandwidth="1200000" width="1280" height="720"/>
    </AdaptationSet>
  </Period>
</MPD>
```

### F7. Low latency: service description plus availability offset (cf. G.18, G.20)

```xml
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic" availabilityStartTime="2026-03-01T00:00:00Z" publishTime="2026-03-01T00:10:00Z"
     minimumUpdatePeriod="PT1H" minBufferTime="PT2S" profiles="urn:mpeg:dash:profile:isoff-ext-live:2014">
  <ServiceDescription id="1"><Latency target="3000" min="1500" max="9000" referenceId="1"/><PlaybackRate min="0.95" max="1.05"/></ServiceDescription>
  <Period id="p" start="PT0S">
    <AdaptationSet id="1" contentType="video" mimeType="video/mp4" segmentAlignment="true" startWithSAP="1">
      <ProducerReferenceTime id="1" inband="true" type="encoder" wallClockTime="2026-03-01T00:00:00Z" presentationTime="0"/>
      <Resync type="0" dT="500000" dImin="0.05" dImax="0.1"/>
      <SegmentTemplate timescale="1000000" duration="4000000" startNumber="1" availabilityTimeOffset="3.5" availabilityTimeComplete="false"
                       initialization="init-$RepresentationID$.m4s" media="chunk-$RepresentationID$-$Number$.m4s"/>
      <Representation id="r" codecs="avc1.4d401f" width="960" height="400" bandwidth="600000"/>
    </AdaptationSet>
  </Period>
  <UTCTiming schemeIdUri="urn:mpeg:dash:utc:http-xsdate:2014" value="https://time.test/now"/>
</MPD>
```

### F8. MPD with `PatchLocation` and a matching patch (cf. G.21)

MPD:
```xml
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" id="live-x" type="dynamic" publishTime="2026-04-01T00:00:00Z" availabilityStartTime="2026-03-01T00:00:00Z"
     minimumUpdatePeriod="PT2S" minBufferTime="PT1S" profiles="urn:mpeg:dash:profile:isoff-live:2011">
  <PatchLocation ttl="30">patch.mpp?pt=2026-04-01T00:00:00Z</PatchLocation>
  <Period id="p1" start="PT0S">
    <AdaptationSet id="1" mimeType="video/mp4" segmentAlignment="true" startWithSAP="1">
      <SegmentTemplate timescale="1000" initialization="i.mp4" media="$Time$.m4s"><SegmentTimeline><S t="0" d="2000" r="2"/></SegmentTimeline></SegmentTemplate>
      <Representation id="v" codecs="avc1.4d401f" width="640" height="360" bandwidth="700000"/>
    </AdaptationSet>
  </Period>
</MPD>
```
Patch (`application/dash-patch+xml`):
```xml
<Patch xmlns="urn:mpeg:dash:schema:mpd-patch:2020" mpdId="live-x" originalPublishTime="2026-04-01T00:00:00Z" publishTime="2026-04-01T00:00:02Z">
  <replace sel="/MPD/@publishTime">2026-04-01T00:00:02Z</replace>
  <add sel="/MPD/Period[@id='p1']/AdaptationSet[@id='1']/SegmentTemplate/SegmentTimeline"><S t="6000" d="2000"/></add>
</Patch>
```

### F9. Failover content (cf. G.22)

```xml
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic" availabilityStartTime="2026-05-01T00:00:00Z" publishTime="2026-05-01T00:05:00Z"
     minimumUpdatePeriod="PT2S" minBufferTime="PT2S" profiles="urn:mpeg:dash:profile:isoff-live:2011">
  <Period id="p">
    <AdaptationSet mimeType="video/mp4" segmentAlignment="true" startWithSAP="1">
      <SegmentTemplate timescale="1000" startNumber="1" initialization="$RepresentationID$/init.mp4" media="$RepresentationID$/$Number$.mp4">
        <SegmentTimeline><S t="0" d="2000" r="9"/></SegmentTimeline>
      </SegmentTemplate>
      <Representation id="B" bandwidth="800000" codecs="avc1.4d401f" width="640" height="360"/>
      <Representation id="A" bandwidth="2000000" codecs="avc1.640020" width="1280" height="720">
        <SegmentTemplate><FailoverContent><FCS t="8000" d="2000"/></FailoverContent></SegmentTemplate>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>
```

## Transcription notes

- The converter collapsed each XML block onto a single line in the private copy; use the Ed.6 `example_G*.mpd` files for line-structured text.
- Tables in this file are our own inventory, not copies of spec tables.
