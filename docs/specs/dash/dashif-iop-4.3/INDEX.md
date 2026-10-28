# DASH-IF Interoperability Points v4.3 (2018-11-15) - chapter map

Source: `private/specs/dashif_iop_v4.3_guidelines.pdf` (254 pages; printed page = PDF page - 21 in the main body, annexes
carry their own numbering). The files here are **original-wording structured notes** (facts, formulas, URNs, rule
lists, inventories, spec-defect notes). The page-verified **verbatim** text is private:
`private/specs-verified/dash/dashif-iop-4.3/<same file name>`, with its record in
`private/specs-verified/dash/dashif-iop-4.3/VERIFICATION.md` and pdf2md raw output in `pdf2md-raw/`.

Status values: `transcribed` / `transcribed-as-notes` = covered by a notes file (verbatim exists privately);
`not-client-relevant` = not transcribed, reason given. §3.2.9 trick mode was already transcribed:
[dashif-iop-4.3-trickmode.md](./dashif-iop-4.3-trickmode.md) (short page-verified quotes; not duplicated).

Companions: [Part 10 events](../dashif-iop-part10-v5.0.0/INDEX.md), [DVB-DASH TS 103 285](../etsi-ts-103-285/INDEX.md),
live-ingest design [`docs/dashif-ingest/DESIGN.md`](../../../dashif-ingest/DESIGN.md) (DASH-IF Live Media Ingest 1.3,
`private/specs-md/dashif_live_media_ingest_v1.3.md`, not in scope here), `emsg`:
[`specs/rules/emsg-rules.md`](../../../../specs/rules/emsg-rules.md), [`mp4-emsg/docs/emsg.md`](../../../../mp4-emsg/docs/README.md#private-spec-transcriptions),
ISO/IEC 23009-1: [`transmux/docs/README.md`](../../../../transmux/README.md).
Verification summary: [../VERIFICATION.md](../VERIFICATION.md).

## Chapters 1-3 and front matter (PDF 1-82)


| PDF pages | Printed | Clause | Status | File / reason |
|---|---|---|---|---|
| 1-2 | - | Title page, blank | not-client-relevant | cover; title/version/date are in each file's front matter |
| 3 | 1 | Scope | transcribed | [ch00-front-matter.md](./ch00-front-matter.md) |
| 4 | 2 | Disclaimer | transcribed | [ch00-front-matter.md](./ch00-front-matter.md) |
| 5-11 | 3-9 | Contents | not-client-relevant | navigation aid (table of contents) |
| 11-13 | 9-10 | List of Figures, List of Tables | not-client-relevant | navigation aid; the figures/tables themselves are transcribed where they occur |
| 13-15 | 11-13 | Acronyms, abbreviations and definitions | transcribed | [ch00-front-matter.md](./ch00-front-matter.md) |
| 16-21 | 14-19 | References [1]-[91] | transcribed | [ch00-front-matter.md](./ch00-front-matter.md) (needed to resolve every `[n]` citation) |
| 22-25 | 1-4 | 1 Introduction (Tables 1, 2, 3: IOP/extension/identifier tables) | transcribed | [ch01-introduction.md](./ch01-introduction.md) |
| 26-33 | 5-12 | 2 Context and Conventions (2.1 relation to MPEG-DASH, 2.2 version history/compatibility, 2.3 key words, 2.4 interoperability points/profiles) | transcribed | [ch02-context-and-conventions.md](./ch02-context-and-conventions.md) |
| 33 | 12 | 3 Usage of MPEG-DASH, 3.1 Scope | transcribed | [ch03-1-scope.md](./ch03-1-scope.md) |
| 33-35 | 12-14 | 3.2.1 Introduction (segment format constraints) | transcribed | [ch03-2-dash-formats.md](./ch03-2-dash-formats.md) |
| 35-36 | 14-15 | 3.2.2 MPD constraints for v1 & v2 clients (3.2.2.1, 3.2.2.2) | transcribed | [ch03-2-dash-formats.md](./ch03-2-dash-formats.md) |
| 36-37 | 15-16 | 3.2.3 Segment format constraints | transcribed | [ch03-2-dash-formats.md](./ch03-2-dash-formats.md) |
| 37-38 | 16-17 | 3.2.4 Presence of attributes and elements | transcribed | [ch03-2-dash-formats.md](./ch03-2-dash-formats.md) |
| 38 | 17 | 3.2.5 MPD dimension constraints; 3.2.6 Generic metadata | transcribed | [ch03-2-dash-formats.md](./ch03-2-dash-formats.md) |
| 38-40 | 17-19 | 3.2.7 DASH timing model (3.2.7.1-3.2.7.4, Figure 1) | transcribed-as-notes (figure redrawn as description + ASCII) | [ch03-2-dash-formats.md](./ch03-2-dash-formats.md) |
| 40-41 | 19-20 | 3.2.8 Bandwidth and minimum buffer time | transcribed | [ch03-2-dash-formats.md](./ch03-2-dash-formats.md) |
| 41-42 | 20-21 | 3.2.9 Trick mode support | transcribed elsewhere (not duplicated) | [dashif-iop-4.3-trickmode.md](./dashif-iop-4.3-trickmode.md); re-verified against rendered pp.41-42, no discrepancy |
| 42-43 | 21-22 | 3.2.10 Adaptation Set constraints (bitstream switching) | transcribed | [ch03-2-dash-formats.md](./ch03-2-dash-formats.md) |
| 44-46 | 23-25 | 3.2.11 Media time information of Segment (three timing example tables) | transcribed-as-notes (tables rebuilt from render) | [ch03-2-dash-formats.md](./ch03-2-dash-formats.md) |
| 46-47 | 25-26 | 3.2.12 Content offering with Periods (period continuity/connectivity) | transcribed | [ch03-2-dash-formats.md](./ch03-2-dash-formats.md) |
| 47-48 | 26-27 | 3.2.13 Adaptation Set media type; 3.2.14 Seek preview and thumbnail navigation | transcribed | [ch03-2-dash-formats.md](./ch03-2-dash-formats.md) |
| 48-49 | 27-28 | 3.2.15 MPD location and reference resolution (client algorithm) | transcribed | [ch03-2-dash-formats.md](./ch03-2-dash-formats.md) |
| 49-51 | 28-30 | 3.3 Client implementation requirements and guidelines (3.3.1-3.3.4, Figure 2) | transcribed-as-notes (figure described) | [ch03-3-client-requirements-transport-switching.md](./ch03-3-client-requirements-transport-switching.md) |
| 51-52 | 30-31 | 3.4 Transport and protocol-related issues (3.4.1-3.4.4: HTTP/1.1 RFCs, byte ranges, cookies, transforming proxies) | transcribed | same file |
| 52-53 | 31-32 | 3.5 Synchronization considerations | transcribed | same file |
| 53 | 32 | 3.6 Different service types; 3.7 Ad insertion (pointers only) | transcribed | same file |
| 53-54 | 32-33 | 3.8 Switching across Adaptation Sets | transcribed | same file |
| 54-55 | 33-34 | 3.9.1 Introduction; 3.9.2 Adaptation Set labeling options (lead-in to Table 4) | transcribed | [ch03-9-content-selection.md](./ch03-9-content-selection.md) |
| 55-62 | 34-41 | Table 4 Adaptation Set attributes and elements and usage (general, video, audio, subtitle: roles, accessibility, lang, codecs, audioSamplingRate, selectionPriority, ...) | transcribed | same file |
| 62-63 | 41-42 | 3.9.3 Content model (Figure 3) | transcribed-as-notes (figure described) | same file |
| 64-68 | 43-47 | 3.9.4 Signalling requirements and recommendations (3.9.4.1-3.9.4.8) | transcribed | same file |
| 68-72 | 47-51 | 3.9.5 Client processing reference model (3.9.5.1, 3.9.5.2 steps 1-9) | transcribed | same file |
| 73 | 52 | 3.10.1 On-demand services introduction | transcribed | [ch03-10-on-demand-services.md](./ch03-10-on-demand-services.md) |
| 73-77 | 52-56 | 3.10.2 On-demand services with live profiles (Table 5, segment information derivation) | transcribed | same file |
| 77-81 | 56-60 | 3.10.3 On-demand services with the On-Demand profile (Table 6, sidx generation, client operation) | transcribed | same file |
| 82 | 61 | 3.10.4 Mixed on-demand content | transcribed | same file |

## Chapter 4 - Live services (PDF 82-143)

No clause of chapter 4 is marked not-client-relevant: even the service-provider clauses (4.3.3, 4.4.3, 4.5.2, 4.6.2, 4.11.2-4.11.4) define the offering that a client must interpret (availability windows, update semantics, loss signalling), so they are transcribed completely.

| Clause | Title | Printed pp. | Status | File |
|---|---|---|---|---|
| 4 / 4.1 | Live Services - Introduction (Table 7) | 61-62 | transcribed | [ch04-01](ch04-01-live-intro-and-preliminaries.md) |
| 4.2 | Overview Dynamic and Live Media Presentations (Figure 4 client models) | 62-65 | transcribed | [ch04-01](ch04-01-live-intro-and-preliminaries.md) |
| 4.3 / 4.3.1 | Dynamic Segment Download - Background and Assumptions | 65 | transcribed | [ch04-01](ch04-01-live-intro-and-preliminaries.md) |
| 4.3.2 (4.3.2.1 - 4.3.2.2.9) | Preliminaries: MPD information (Table 8), segment information derivation (PSwc/PEwc, SAST/SAET/ASAST formulas, k[NOW]), URL templating, synchronized playout | 65-71 | transcribed | [ch04-01](ch04-01-live-intro-and-preliminaries.md) |
| 4.3.3 (4.3.3.1 - 4.3.3.5) | Service offering requirements: general, parameter settings (TSB/SPD/SDURATION/MBT), example (Figure 5), Periods (Table 11), Segment Timeline (Table 12), joining recommendation / MPD anchors (`#t=`, `posix:`) | 71-80 | transcribed | [ch04-02](ch04-02-dynamic-segment-download.md) |
| 4.3.4 (4.3.4.1 - 4.3.4.5) | Client operation: basic operation (Figure 6), segment list, multi-Period, joining / initial buffering / live-edge join strategies, requirements | 80-85 | transcribed | [ch04-02](ch04-02-dynamic-segment-download.md) |
| 4.3.5 | Additional DVB-DASH alignment aspects | 85 | transcribed | [ch04-02](ch04-02-dynamic-segment-download.md) |
| 4.3.6 | Considerations on live edge | 85-86 | transcribed | [ch04-03](ch04-03-live-edge-and-mpd-updates.md) |
| 4.4 (4.4.1 - 4.4.4) | Simple Live with MPD updates: background, Table 13/14, PEwc = NOW + MUP, MUP semantics, permitted updates, Segment Timeline (Table 15), last segment message `lmsg`, last-segment-number descriptor, MPD-based client operation (FetchTime, conditional GET) | 86-93 | transcribed | [ch04-03](ch04-03-live-edge-and-mpd-updates.md) |
| 4.5 (4.5.1 - 4.5.3) | MPD and Segment-based (Main) Live: Table 16, `urn:mpeg:dash:event:2012` (Table 17, Figure "4" emsg example, Table 18), advanced client model (Figure 7), MPD validity expiration, extended segment information | 93-101 | transcribed | [ch04-04](ch04-04-mpd-and-segment-based-live.md) |
| 4.6 (4.6.1 - 4.6.4) | Provisioning of live content in on-demand mode; client behaviour; live-to-on-demand transition | 101-103 | transcribed | [ch04-04](ch04-04-mpd-and-segment-based-live.md) |
| 4.7 (4.7.1 - 4.7.3) | Availability time synchronisation: `UTCTiming` schemes and client rules | 103-105 | transcribed | [ch04-05](ch04-05-time-sync-robust-operation.md) |
| 4.8 (4.8.1 - 4.8.8) | Robust operation: HTTP error handling (A.7 restated), segmenter sync loss, encoder drift, 404 / Date header, redundant tools | 105-107 | transcribed | [ch04-05](ch04-05-time-sync-robust-operation.md) |
| 4.9 (4.9.1 - 4.9.3) | Interoperability aspects: Simple Live / Main Live operation points (service and client requirement lists) | 107-108 | transcribed | [ch04-05](ch04-05-time-sync-robust-operation.md) |
| 4.10 (4.10.1 - 4.10.4) | Trick mode for live services (3.2.9 signalling: [dashif-iop-4.3-trickmode.md](./dashif-iop-4.3-trickmode.md)) | 109-110 | transcribed | [ch04-05](ch04-05-time-sync-robust-operation.md) |
| 4.11 (4.11.1 - 4.11.5) | Deployment scenarios: reliable consistent-delay live service (Figure 8), consistent latency, unanticipated new Periods, segment duration variations, losses (Figure 9), minimizing MPD updates, segment stream (Figure 10), simple / main live packager configurations, client requirements | 110-122 | transcribed | [ch04-06](ch04-06-deployment-scenarios.md) |

Printed p. 61 (PDF 82) also carries clause 3.10.4 (Mixed On-Demand Content) above the chapter 4 heading; that clause belongs to the chapter 3 slice, not to this one.

## Chapters 5-11 and annexes (PDF 143-254)

PDF page = printed page + 21 for the main body (chapters 1-11); the annexes carry their own printed numbering (annex pp.1-26, footers read "v4.1" in the annexes). Status: transcribed | not-client-relevant.

| Clause | Title | PDF pp. | Status | File / reason |
|---|---|---|---|---|
| 5 (5.1-5.1.3.7) | Ad Insertion in DASH: introduction, definitions, DASH concepts (remote elements/XLink, Periods, events, MPD updates, session info, tracking) | 143-147 | transcribed | [ch05-ad-insertion.md](./ch05-ad-insertion.md) |
| 5.2 | Architectures | 147 | transcribed | ch05 |
| 5.3 (5.3.1-5.3.5) | Server-based architecture: mapping to DASH, workflows (cue interpretation, cancelation, early termination, ad decision, on demand, slates, blackouts, tracking), "Top Gun" example MPD, query parameters | 148-158 | transcribed | ch05 |
| 5.4 (5.4.1-5.4.3) | App-based architecture: SCTE 35 events (emsg and in-MPD), asset identifiers, workflows | 158-162 | transcribed | ch05 |
| 5.5 (5.5.1-5.5.3) | Extensions for ad insertion: asset identifiers, remote periods, user-defined events | 162-163 | transcribed | ch05 |
| 5.6 (5.6.1-5.6.2) | Interoperability aspects | 163-164 | transcribed | ch05 |
| 6.1, 6.2 (6.2.1-6.2.4) | Media coding: video general, H.264/AVC, H.265/HEVC (Tables 19-21), video metadata | 164-168 | transcribed | [ch06-media-coding.md](./ch06-media-coding.md) |
| 6.2.5 (6.2.5.1-6.2.5.2) | Video Adaptation Set constraints, bitstream switching | 169-170 | transcribed | ch06 |
| 6.2.6 | Tiles of thumbnail images | 170-172 | transcribed | ch06 |
| 6.3 (6.3.1-6.3.3) | Audio: general, HE-AACv2 (Table 22), audio metadata | 173-174 | transcribed | ch06 |
| 6.4 (6.4.1-6.4.6) | Auxiliary components: subtitles/closed captioning, CEA-608/708 in SEI (ABNF, examples), IMSC1 (Table 23), side-loaded TTML/WebVTT, annotation of subtitles | 174-179 | transcribed | ch06 |
| 7.1-7.3 | Content protection: introduction, HTTPS and DASH, base technologies summary | 179-181 | transcribed | [ch07-content-protection.md](./ch07-content-protection.md) |
| 7.4 (7.4.1-7.4.2) | ISO BMFF support for CENC and DRM: box hierarchy (Figures 21-22), Table 24 | 181-184 | transcribed | ch07 |
| 7.5 (7.5.1-7.5.3.4) | Periodic re-authorization (Figure 23) | 184-186 | transcribed | ch07 |
| 7.6 (7.6.1-7.6.2.4) | MPD support for encryption/DRM signalling: ContentProtection descriptors (mp4protection, UUID), cenc:pssh, Clear Key | 186-189 | transcribed | ch07 |
| 7.7 (7.7.1-7.7.9) | Additional content protection constraints, client interactions with DRM systems | 189-194 | transcribed | ch07 |
| 7.8 | Workflow overview (Figures 24-25) | 194-197 | transcribed | ch07 |
| 8 (8.1-8.5) | Interoperability points: DASH-AVC/264 main/high, IOP simple, IOP main | 197-200 | transcribed | [ch08-interoperability-points.md](./ch08-interoperability-points.md) |
| 9.1-9.3 | Multi-channel audio: scope, technologies (Tables 25-30), client implementation guidelines | 200-205 | transcribed | [ch09-multichannel-audio.md](./ch09-multichannel-audio.md) |
| 9.4 (9.4.1-9.4.7) | Extensions: @profiles identifiers and conformance definitions per audio codec | 205-209 | transcribed | ch09 |
| 10.1-10.2 | UHD extensions: HEVC 4k (elementary stream, HDR dynamic metadata table, mapping to DASH, file format, adaptation set constraints, compatibility) | 209-215 | transcribed | [ch10-uhd-extensions.md](./ch10-uhd-extensions.md) |
| 10.3 | HEVC HDR PQ10 | 216-220 | transcribed | ch10 |
| 10.4 | UHD dual-stream (Dolby Vision): definition, EL SEI requirements (Tables 31-32), mapping to DASH, codec parameter signalling | 220-225 | transcribed | ch10 (SEI payload syntax itself is in Annex E/F, not transcribed) |
| 11 (11.1-11.3) | VP9 extensions: DASH-specific aspects, bitstream switching, VP9-HD/UHD/HDR IOPs | 225-228 | transcribed | [ch11-vp9-extensions.md](./ch11-vp9-extensions.md) |
| Annex A | Examples for profile signalling | 229 | transcribed | [annex-a-profile-signalling.md](./annex-a-profile-signalling.md) |
| Annex B (B.1-B.5) | Live services: use cases, baseline architecture (Figures 27-28), multicast, typical problems, advanced use cases | 230-235 | transcribed | [annex-b-live-use-cases.md](./annex-b-live-use-cases.md) |
| Annex C (C.1-C.7.2.2, Table 33) | Dolby Vision streams within ISOBMFF (`dvcC` box, `dvhe`/`dvh1` sample entries, dual-track constraints, `tfra`) | 236-240 | transcribed-as-notes | [annex-c-dolby-vision-isobmff.md](./annex-c-dolby-vision-isobmff.md) (a demuxer reading Dolby Vision init segments needs the box layout; added after independent verification) |
| Annex D (D.1, D.1.1, D.1.1.1, D.1.2, B.1.2.1, B.1.3, B.1.3.1) | Signaling Dolby Vision profiles and levels (Table D.1, profile string, Table B.2, level string, codec string examples, device capabilities) | 241-244 | transcribed | [annex-d-dolby-vision-signalling.md](./annex-d-dolby-vision-signalling.md) (clause numbers B.1.x printed inside Annex D are kept as printed) |
| Annex E (E.1-E.2, Tables C.1-C.3, C.8) | Display management message (DM_data() SEI syntax/semantics) | 245-250 | not-client-relevant | bitstream SEI payload syntax, decoded below the client's manifest/segment layer |
| Annex F (F.1-F.2, Table D.1 CM_data(), Table D.2) | Composing metadata message (CM_data() SEI syntax) | 251-253 (top) | not-client-relevant | bitstream SEI payload syntax (printed table numbers collide with Annex D's) |
| Annex G | Sample dual-layer MPD | 253-254 | transcribed | [annex-g-sample-dual-layer-mpd.md](./annex-g-sample-dual-layer-mpd.md) |
