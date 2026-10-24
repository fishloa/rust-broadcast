# ETSI TS 103 285 V1.4.1 (2023-09) DVB-DASH profile - clause map

Source: `private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf` (149 pages; PDF page = printed page). The files here
are **original-wording structured notes**; the page-verified **verbatim** text is private:
`private/specs-verified/dash/etsi-ts-103-285/<same file name>` (record: `VERIFICATION.md` there; pdf2md raw in `pdf2md-raw/`).
Companions: [IOP v4.3](../dashif-iop-4.3/INDEX.md), [IOP Part 10](../dashif-iop-part10-v5.0.0/INDEX.md),
[`specs/rules/emsg-rules.md`](../../../../specs/rules/emsg-rules.md). Verification summary: [../VERIFICATION.md](../VERIFICATION.md).

## Front matter and clauses 1-7 (PDF 1-61)

| Clause | Title | Status | File |
|---|---|---|---|
| (front) | Intellectual Property Rights, Foreword, Trademarks (pp. 10-11) | not-client-relevant (legal/organisational boilerplate; no player behaviour) | - |
| (front) | Modal verbs terminology (p. 11) | transcribed | [ch01-03](ch01-03-front-scope-refs-defs.md) |
| 1 | Scope (p. 12) | transcribed | [ch01-03](ch01-03-front-scope-refs-defs.md) |
| 2.1 / 2.2 | Normative / Informative references (pp. 12-14) | transcribed (list only) | [ch01-03](ch01-03-front-scope-refs-defs.md) |
| 3.1 | Terms (pp. 15-17) | transcribed | [ch01-03](ch01-03-front-scope-refs-defs.md) |
| 3.2 / 3.3 | Symbols (void) / Abbreviations (pp. 17-19) | transcribed | [ch01-03](ch01-03-front-scope-refs-defs.md) |
| 4.1 | DVB Profile of MPEG-DASH (profile URNs 2014/2017) | transcribed | [ch04](ch04-dash-constraints-http-sync.md) |
| 4.2.1-4.2.8 | MPD constraints (common, Live, On Demand) | transcribed | [ch04](ch04-dash-constraints-http-sync.md) |
| 4.2.9 | Constraints for low latency content | transcribed | [ch04](ch04-dash-constraints-http-sync.md) |
| 4.3 | Segment Format Constraints | transcribed | [ch04](ch04-dash-constraints-http-sync.md) |
| 4.4 | Presence of Attributes and Elements | transcribed | [ch04](ch04-dash-constraints-http-sync.md) |
| 4.5 | Dimension Constraints | transcribed | [ch04](ch04-dash-constraints-http-sync.md) |
| 4.6 | Server Requirements | transcribed | [ch04](ch04-dash-constraints-http-sync.md) |
| 4.7.1-4.7.3 | Availability Time Synchronization (UTCTiming) | transcribed | [ch04](ch04-dash-constraints-http-sync.md) |
| 4.8.1-4.8.4 | MPD Update Requirements | transcribed | [ch04](ch04-dash-constraints-http-sync.md) |
| 5.1.1-5.1.4 | H.264/AVC (profiles, sample entries, codec string, Table 1) | transcribed | [ch05](ch05-video.md) |
| 5.2.1-5.2.4 | HEVC (sample entries, codec string, Table 2, SEI location) | transcribed | [ch05](ch05-video.md) |
| 5.2.5 | Colour format / transfer characteristics (Table 3) | transcribed | [ch05](ch05-video.md) |
| 5.2.6 / 5.2.7 | HLG10 / PQ10 (incl. urn:dvb:dash:hdr-dmi) | transcribed | [ch05](ch05-video.md) |
| 5.2.8.1-5.2.8.4 | Temporal layers (urn:dvb:dash:highest_temporal_id:2017) | transcribed | [ch05](ch05-video.md) |
| 5.3.1-5.3.8 | VVC (sample entries, conformance points, Table 3a, SEI, colour/HDR) | transcribed | [ch05](ch05-video.md) |
| 5.4.1-5.4.3 | AVS3 manifest, conformance points, codecs (Table 3b) | transcribed | [ch05](ch05-video.md) |
| 5.4.4.1-5.4.4.4 | AVS3 elementary streams and sample definitions | transcribed (pointer-only text defining the 'avs3' sample entry; the elementary-stream prose is informative for a client, not encoder-side only) | [ch05](ch05-video.md) |
| 5.4.5-5.4.8 | AVS3 CMAF track, colour (Table 3c), HLG10, PQ10 | transcribed | [ch05](ch05-video.md) |
| 6.1.1 / 6.1.2 | Audio Adaptation Sets, Role scheme (Tables 4-6) | transcribed | [ch06](ch06-audio.md) |
| 6.2 | HE-AACv2 (Table 7) | transcribed | [ch06](ch06-audio.md) |
| 6.3.1-6.3.3 | Dolby E-AC-3 / AC-4 (Tables 8, 9) | transcribed | [ch06](ch06-audio.md) |
| 6.4.1 / 6.4.2 | DTS and DTS-UHD (Table 10) | transcribed | [ch06](ch06-audio.md) |
| 6.5 | MPEG Surround (Table 11) | transcribed | [ch06](ch06-audio.md) |
| 6.6.1-6.6.3 | Service continuity / fallback adaptation set | transcribed | [ch06](ch06-audio.md) |
| 6.7.1-6.7.6 | NGA Preselections (Tables 12-16) | transcribed | [ch06](ch06-audio.md) |
| 6.8.1-6.8.4 | MPEG-H Audio (Table 17) | transcribed | [ch06](ch06-audio.md) |
| 7.1.1 / 7.1.2 | Subtitles carriage, EBU-TT-D, Roles (Tables 18-20) | transcribed | [ch07](ch07-subtitles.md) |
| 7.2.0-7.2.4 | Downloadable fonts (Table 21) | transcribed | [ch07](ch07-subtitles.md) |
| 7.3 | Example (informative) | transcribed | [ch07](ch07-subtitles.md) |

## Clauses 8-11 and annexes (PDF 61-149)

| Clause | Title | Status | File |
|---|---|---|---|
| 8.1 | Content Protection: Introduction | transcribed | [ch08](ch08-content-protection.md) |
| 8.2 | Background (informative): pssh, ContentProtection in MPD, key rotation | transcribed | [ch08](ch08-content-protection.md) |
| 8.3 | Encryption of Different Representations (same default_KID per AdaptationSet; ContentProtection at AdaptationSet level) | transcribed | [ch08](ch08-content-protection.md) |
| 8.4 | MPD Content Protection descriptors (mp4protection cenc/cbcs, default_KID, UUID descriptors) | transcribed | [ch08](ch08-content-protection.md) |
| 8.5.0-8.5.2 | pssh handling, key rotation (>= 120 s), random-access requirement | transcribed | [ch08](ch08-content-protection.md) |
| 8.6.0 / 8.6.1 | Encryption usage (cenc/cbcs, NAL clear rules, Table 22), saio constraints | transcribed | [ch08](ch08-content-protection.md) |
| 8.7 | Mixing non-encrypted and encrypted content | transcribed | [ch08](ch08-content-protection.md) |
| 9.1.1 | Events: background (informative) | transcribed | [ch09](ch09-events.md) |
| 9.1.2.1-9.1.2.4 | Content programme metadata (urn:dvb:iptv:cpm:2014, Tables 23-24, parental rating, example) | transcribed | [ch09](ch09-events.md) |
| 9.1.3 | Application messages (presentationTimeOffset, contentEncoding) | transcribed | [ch09](ch09-events.md) |
| 9.1.4 | Events for the Player (urn:mpeg:dash:event:2012 MPD validity expiration, publishTime) | transcribed | [ch09](ch09-events.md) |
| 9.1.5 | Long duration inband events (repetition rules, 10 concurrent events) | transcribed | [ch09](ch09-events.md) |
| 9.1.6 | Monitoring of InbandEventStreams | transcribed | [ch09](ch09-events.md) |
| 9.1.7.1-9.1.7.5 | Hybrid Digital Radio inband events (urn:dvb:dash:radiovis:2016, values 1-3) | transcribed | [ch09](ch09-events.md) |
| 9.1.8 | DVB application signalling (urn:dvb:dash:appsignalling:2016) | transcribed | [ch09](ch09-events.md) |
| 9.1.9 | SCTE-35 ad slot signalling (urn:scte:scte35:2014:xml+bin) | transcribed | [ch09](ch09-events.md) |
| 10.0 / 10.1 | Player Behaviour: introduction, DVB profile support (2014 / 2017, clause lists) | transcribed | [ch10-1](ch10-1-profile-bitstreams-switching.md) |
| 10.2 | ISO BMFF Player constraints | transcribed | [ch10-1](ch10-1-profile-bitstreams-switching.md) |
| 10.3.1 / 10.3.2 | Bitstreams, luminance resolutions (Tables 25, 26, 27, 27a), frame rates, variable resolution | transcribed | [ch10-1](ch10-1-profile-bitstreams-switching.md) |
| 10.4 | Audio/Video switching between Representations (seamless definitions) | transcribed | [ch10-1](ch10-1-profile-bitstreams-switching.md) |
| 10.5.1-10.5.3 | Playback across Periods (association, period continuity, non-continuous playback, Figures 1a/1b) | transcribed | [ch10-2](ch10-2-playback-across-periods.md) |
| 10.6 | Trick Mode Implementation (informative) | transcribed (informative; IOP trick-mode signalling is in [dashif-iop-4.3-trickmode](../dashif-iop-4.3/dashif-iop-4.3-trickmode.md), not duplicated) | [ch10-3](ch10-3-trickmode-buffering.md) |
| 10.7 | Player Buffering (about 300 s cap) | transcribed | [ch10-3](ch10-3-trickmode-buffering.md) |
| 10.8.1 | Player resiliency introduction (informative) | transcribed | [ch10-4](ch10-4-resiliency.md) |
| 10.8.2.1-10.8.2.4 | BaseURL handling: @priority/@weight/@serviceLocation, blacklist, selection, change, worked example | transcribed (example boxes transcribed from the rendered page; they are images in the PDF) | [ch10-4](ch10-4-resiliency.md) |
| 10.8.3 | DNS - HTTP Player | transcribed | [ch10-4](ch10-4-resiliency.md) |
| 10.8.4 / 10.8.5 | Error condition types; Table 28 (errors and categories) | transcribed | [ch10-4](ch10-4-resiliency.md) |
| 10.8.6 | Requirements on Players; Table 29 (actions, retries); Figure 1c | transcribed | [ch10-4](ch10-4-resiliency.md) |
| 10.9.1-10.9.7 | Live Services: live detection, start position (45 s, suggestedPresentationDelay, MPD anchors), segment templates, 24/7, MPD update timing, timeShiftBufferDepth, dynamic-to-static playback | transcribed | [ch10-5](ch10-5-live-services.md) |
| 10.10 | MPD Processing (Role main, selectionPriority, FramePacking/EssentialProperty) | transcribed | [ch10-6](ch10-6-mpd-processing-http.md) |
| 10.11 | HTTP Support (HTTP/1.1, byte ranges, gzip, redirects, MPD.Location, keep-alive, timeouts) | transcribed | [ch10-6](ch10-6-mpd-processing-http.md) |
| 10.12.1-10.12.5 | Reporting errors: metrics mechanism, Reporting descriptor (urn:dvb:dash:reporting:2014, @reportingUrl/@probability), Tables 30-32, DVBErrors | transcribed (the Reporting descriptor and DVBErrors metric are read by a playback client) | [ch10-7](ch10-7-reporting.md) |
| 10.13 | Reporting media duration and current play position to applications (informative) | transcribed | [ch10-7](ch10-7-reporting.md) |
| 10.14-10.16 | HEVC colour compatibility, display composition and HDR, HEVC temporal layers | transcribed | [ch10-8](ch10-8-hdr-hfr-nga-decryption.md) |
| 10.17 | Compatibility (ignore rules, cmfc, unknown boxes, availabilityTimeOffset) | transcribed | [ch10-8](ch10-8-hdr-hfr-nga-decryption.md) |
| 10.18 / 10.19 | Player requirements for NGA; Decryption (cenc/cbcs) | transcribed | [ch10-8](ch10-8-hdr-hfr-nga-decryption.md) |
| 10.20.1-10.20.8 | Low latency presentation: applicability, media access, service description (examples), presentation time target (Figure 2), AdaptationSet selection, catch-up (Figure 3), joining (Figures 4-5, Tables 33-34), drift avoidance and leap seconds | transcribed | [ch10-9](ch10-9-low-latency.md) |
| 11.1 | Use of Profiles | transcribed | [ch11](ch11-content-provider-guidelines.md) |
| 11.2.0-11.2.2 | Video resolutions and frame rates in Adaptation Sets (Table 35 recommendations) | not-client-relevant (informative content-provider encoding guidance; no Player rule) | - |
| 11.3.0-11.3.3 | Audio in Adaptation Sets (Tables 36-40 typical bit rates, DTS profiles) | not-client-relevant (informative encoder bit-rate recommendations) | - |
| 11.4-11.7 | Interlaced/progressive, HD/SD, AVC/HEVC mixing, subtitles | transcribed (short informative text: explains constraints Players rely on) | [ch11](ch11-content-provider-guidelines.md) |
| 11.8 | A/V switching across Period boundaries | transcribed | [ch11](ch11-content-provider-guidelines.md) |
| 11.9.1-11.9.5 | Multiple serving locations using BaseURLs (priority, load balancing, combination, Figure 6, relative/absolute) | transcribed | [ch11](ch11-content-provider-guidelines.md) |
| 11.10-11.13 | Using events, asset identifiers, 24/7 considerations, cache control | transcribed | [ch11](ch11-content-provider-guidelines.md) |
| 11.14-11.17 | WCG/HDR, High Frame Rate (temporal layer examples), subtitles, NGA terminology | transcribed | [ch11](ch11-content-provider-guidelines.md) |
| 11.18.1-11.18.3 | MPDs with low latency content | transcribed | [ch11](ch11-content-provider-guidelines.md) |
| 11.19 | Converting a dynamic to a static MPD | transcribed | [ch11](ch11-content-provider-guidelines.md) |
| Annex A (A.1, A.2) | Summary of fixes to 2014 and 2017 profiles (Tables A.1, A.2) | transcribed (informative change log; shows which Player rules were clarified) | [annex-a](annex-a-fixes.md) |
| Annex B (B.1-B.3) | MPD examples for Next Generation Audio (SRSP, SRMP, MRMP; AC-4, MPEG-H, DTS-UHD) | transcribed (informative, XML only) | [annex-b](annex-b-nga-mpd-examples.md) |
| Annex C (C.1-C.3.4) | Example DASH NGA system model (Figures C.1-C.3) | transcribed (informative) | [annex-c](annex-c-nga-system-model.md) |
| Annex D (D.1, D.2) | DVB metrics reporting examples | transcribed (informative) | [annex-d-e](annex-d-e-examples.md) |
| Annex E | MPD example for low latency | transcribed (informative) | [annex-d-e](annex-d-e-examples.md) |
| History | Document history (p. 149) | not-client-relevant (version table: V1.1.1 May 2015, V1.2.1 March 2018, V1.3.1 February 2020, V1.4.1 September 2023) | - |
