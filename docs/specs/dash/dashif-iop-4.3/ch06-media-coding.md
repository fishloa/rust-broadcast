---
source: DASH-IF Interoperability Points v4.3 (15 Nov 2018), private/specs/dashif_iop_v4.3_guidelines.pdf
clauses: "6 Media Coding Technologies (6.1-6.4.6)"
pdf_pages: "164-179"
printed_pages: "143-158"
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch06-media-coding.md
verified_against_render: true
notes_original_wording: true
---

# IOP v4.3 clause 6 - Media coding signalling (client-relevant notes)

Own-words notes; full text, tables and examples in the private verbatim file. "p." = PDF page.

## 6.2 Video (pp.164-172)

Baseline decoders assumed: H.264/AVC Progressive High L3.1 (720p30; also decodes Constrained Baseline and "Progressive" Main up to L3.1), a Main L3.0 subset allowed, High L4.0 for 1080p30; HEVC Main L3.1 (720p30), Main L4.1 (8-bit, up to 2048x1080@60), Main10 L4.1 (10-bit); UHD in clause 10 (6.2.1).

### H.264/AVC (6.2.2) and HEVC (6.2.3)

| Topic | Rule |
|---|---|
| AVC sample entries | Clients must support `avc1` (parameter sets only in init segment) and `avc3` (in-band); services may use either. SAP types 1 and 2 correspond to IDR frames. |
| HEVC sample entries | Clients must support `hvc1` and `hev1`. |
| Codec strings | RFC 6381 for AVC; 14496-15 Annex E for HEVC. Other strings may be used. |
| AVC strings (Table 19) | Main L3.0 `avc[1,3].4DY01E`; High L3.1 `avc[1,3].64Y01F`; High L4.0 `avc[1,3].64Y028`. The table is typeset sideways; `Y` and the `[1,3]` are printed literally (pp.165-167). |
| HEVC strings (Table 21) | Main L3.1 `hev1/hvc1.1.2.L93.B0`; Main L4.1 `hev1.1.2.L123.B0` / `hvc1.12.L123.B0` (missing dot as printed); Main10 L4.1 `hev1/hvc1.2.4.L123.B0`. |
| IRAP signalling (Table 20) | IDR_N_LP, BLA_N_LP: sync, SAP 1. IDR_W_RADL, BLA_W_RADL: sync, SAP 2 if RADL pictures else 1. BLA_W_LP and CRA: three cases - with RASL: not sync, SAP 3; no RASL but RADL: sync, SAP 2; no leading pictures: sync, SAP 1. If the signaller cannot decide, use the first row for that NAL type. |
| Video metadata | MPD attribute provisioning is in clause 3.2.4. |

### Adaptation Set constraints (6.2.5, pp.169-170)

- Representations of a video AdaptationSet are alternative encodings of the same source; lower resolutions must be exact integer subsamples; only the active picture is encoded (no letterbox bars); sample count scales to the source aspect ratio with `aspect_ratio_idc`.
- `avc1`/`hvc1`: all parameter sets are tied to the init segment's `avcC`/`hvcC`. `avc3`/`hev1`: parameter sets precede the first sample of each Media Segment and the init-segment record is only for initialization, equal to the highest tier/profile/level.
- Within an AdaptationSet: same sample-entry coding name (no mixing avc1/avc3 or avc1/hev1), equal `@timescale` and `tkhd` timescale, equal edit-list offset when edit lists are used with `avc1`/`hvc1`; colour primaries, transfer characteristics and matrix coefficients must not differ (if they differ across sets, signal on the set with Essential/Supplemental Property `urn:mpeg:mpegB:cicp:ColourPrimaries | TransferCharacteristics | MatrixCoefficients`, values per ISO/IEC 23001-8).
- Bitstream switching (`@bitstreamswitching` true): `avc3`/`hev1` only, every IDR preceded by SPS/PPS; init segment carries highest tier/profile/level and size (plus VPS for HEVC); AdaptationSet `@codecs` present = maximum profile/level, Representation `@codecs` optional = max of its segments; no edit lists except On-Demand profile (and identical if present); first presented sample's composition time = first decode time = `tfdt` baseMediaDecodeTime (needs negative composition offsets, `trun` v1); `@presentationTimeOffset` aligns audio/video/subtitle timelines at Period start, init-segment edit lists ignored, PTO at a Period start should coincide with a segment's first frame. Satisfies the DVB-DASH requirement.

### Thumbnail tiles (6.2.6, pp.170-172)

An AdaptationSet with `@contentType="image"` carries tiles (one jpeg/png of a grid of equally spaced thumbnails) via `SegmentTemplate` with `$Number$` only. Grid size is the value of `EssentialProperty` with scheme `http://dashif.org/guidelines/thumbnail_tile`, written `<h>x<v>` (htiles x vtiles).

| Symbol | Meaning / derivation |
|---|---|
| tduration | `@duration` (in `@timescale`, else seconds) = duration of one tile |
| `@bandwidth` | max tile size in bits / tduration |
| `@width`,`@height` | tile pixel size (JPEG limit 64k per side) |
| startnumber | `@startNumber`, default 1 |
| pto | `@presentationTimeOffset`, default 0 |
| ttiles | htiles x vtiles |
| thduration | tduration / ttiles |
| tile presentation time | (tnumber - 1) x tduration - pto |
| thumbnail time in period | (tnumber-1) x tduration + (thnumber-1) x thduration - pto; thumbnails run left to right, top row first; last tile may be padded |
| thumbnail size | width/htiles by height/vtiles |
| tthumbnails | ceil(period duration / thduration) |

The worked example (25x1, duration 125, 899 s Period -> 180 thumbnails, 10 kbit/s) states vertical 256 / horizontal 180, which contradicts the formulas above (height/vtiles = 180, width/htiles = 256 for width 6400, height 180) - treat the example numbers with care (p.172).

## 6.3 Audio (pp.173-174)

- Baseline: stereo HE-AACv2 level 2 (decodes AAC-LC and HE-AAC); clients expected to play AAC-LC, HE-AAC, HE-AACv2. Explicit backward-compatible SBR/PS signalling should be used (shall, for DVB-DASH). Each (sub)segment starts at SAP type 1.
- Codec strings (Table 22), encapsulation 14496-14, SAP 1: AAC `mp4a.40.2`; HE-AAC `mp4a.40.5` (= AOT 2+5); HE-AACv2 `mp4a.40.29` (= AOT 2+5+29).
- With SBR (`mp4a.40.5`/`.29`) `@audioSamplingRate` is the rate after SBR (e.g. 48 kHz over a 24 kHz core); with PS (`.29`) `AudioChannelConfiguration` is the channel layout after PS.
- Metadata comes from 23009-1: `@audioSamplingRate` and `AudioChannelConfiguration` using the ISO/IEC 23001-8 ChannelConfiguration scheme.

## 6.4 Auxiliary components (pp.174-179)

- Subtitle technologies: CEA-608/708; IMSC1-conformant TTML in 14496-30 segments (TTML, SMPTE-TT, EBU-TT subsets); 3GPP Timed Text; WebVTT. Simple cases: CEA-608/708 signalling; otherwise IMSC1. TTML/WebVTT segments are referenced by Representations and downloaded/synchronised like audio/video; sidecar interoperable playback is not specified, but 6.4.5 gives Period-boundary guidance.

### CEA-608/708 in SEI (6.4.3, pp.175-177)

- Carriage (ANSI/SCTE 128-1): SEI payloadType 4 (ITU-T T.35); country code 0xB5; provider code 0x0031; user identifier 0x47413934 ("GA94"); `user_data_type_code` 0x03 = caption data.
- Metadata a client may want: number of services (1-16); per service type 608 or 708 (at most one 608 stream signalled; the stream itself names the 608 channels); for 708: service number (1-31), language (ISO 639.2/B), display aspect ratio (4:3 or 16:9), easy-reader flag.
- MPD signalling uses an `Accessibility` descriptor on the AdaptationSet; all Representations must carry equivalent captions.

| Scheme `@schemeIdUri` | `@value` |
|---|---|
| `urn:scte:dash:cc:cea-608:2015` | optional; absent = a 608 service is present. Otherwise up to three `CCn=lang` pairs (CC1..CC4, `;` separated) or a bare list of 3-letter languages (discouraged). ABNF skeleton: `(channel *3 [";" channel]) / (language *3[";" language])`; `channel = CCn "=" language`. |
| `urn:scte:dash:cc:cea-708:2015` | service-to-language map, e.g. `1=lang:eng;2=lang:deu`; more keys (`war`, `er`) appear in the easy-reader example. DASH-IF gives no further 708 guidance (6.4.3.4). |

### IMSC1 and sidecar files (6.4.4-6.4.6, pp.177-179)

- Text track conforms to IMSC1; graphics-type samples are SAP 1; codec string per the W3C TTML profile registry. Table 23: without ISO BMFF -> MIME `application/ttml+xml` (DVB-DASH does not allow XML-only); with ISO BMFF -> `application/mp4`, encapsulation 14496-12 and 14496-30; `@codecs` per the W3C registry. (The table marks the first MIME with notes "(1,3)" but only note 1 is printed.)
- Side-loaded TTML/WebVTT (dash.js style): AdaptationSet `contentType="text"`, `mimeType="application/ttml+xml"` (WebVTT: `text/vtt`), `lang`, `Role` with scheme `urn:mpeg:dash:role:2011` value `subtitle`, one Representation with a `BaseURL` to the file. One file per full Period only (non-live); its time zero is the Period start; presentation time offset should be absent / ignored by the client. Segmented subtitles (live) must be ISOBMFF TTML/WebVTT with timing per 14496-30, i.e. against the media timeline.
- Subtitle annotation: use Role, Accessibility, Essential/Supplemental Property and the DASH role scheme; guidance in DVB-DASH 7.1.2 and SCTE 214-1 7.2.
