---
source: private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
document: ETSI TS 103 285 V1.4.1 (2023-09)
clauses: 7 DASH specific aspects for subtitles (7.1-7.3)
pdf_pages: 57-61 (PDF page = printed page)
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/ch07-subtitles.md
verified_against_render: true
notes_original_wording: true
---

# TS 103 285 - clause 7 (notes): subtitles and downloadable fonts

Original-wording notes. TTML/EBU-TT-D parsing itself is out of scope here (EBU Tech 3380/3381, the `ttml-subtitle` crate).

## 7.1 Carriage (pp. 57-59)
- Player rule: an AdaptationSet with `@contentType="text"` whose `@codecs` does not start with `stpp` should be ignored.
- Subtitles shall be EBU-TT-D (Tech 3380) wrapped in ISO BMFF per Tech 3381 (with 14496-12 and 14496-30). Fonts needed
  or possibly needed shall be supplied as in 7.2. Because the XML rides in sample data, HTTP compression is recommended
  and clause 10.11 requires gzip support. Standalone EBU-TT-D delivered by an application is not excluded but out of scope.
- Table 18: MIME `application/mp4`; `@contentType` shall be `text`; `@codecs` shall start with `stpp`, should name the
  W3C TTML profile registry profile; bare `stpp` means `stpp.ttml.etd1` (backwards compatibility). Examples:
  `stpp.ttml.etd1`; `stpp.ttml.etd1|im1t` or `stpp.ttml.im1t|etd1` (usable by EBU-TT-D or IMSC1 renderers).
- Rationale given: fragment wrapping lets live subtitles use the normal DASH path, reuse the ISO BMFF timeline for
  A/V sync, and provides the presentation interval ("external context").
- 7.1.2 Roles: the subtitle purpose shall be identifiable via Role, Accessibility and `@lang` on the AdaptationSet:

| Kind of subtitle | `@lang` | Role | Accessibility (AudioPurposeCS) |
|---|---|---|---|
| hard of hearing, same language as the programme | audio language | `main` | 2 |
| translation of the main audio | subtitle language | `alternate` | none required |
| extra commentary | subtitle language | `commentary` | none required |
| translating a foreign soundtrack/parts | subtitle language | `main` | none required |

  Table 20 examples use `stpp` with three-letter `@lang` (`eng`/`fra`/`deu`) whereas audio tables use two-letter codes.

## 7.2 Downloadable fonts (pp. 59-61)
- Signal with an EssentialProperty or SupplementalProperty that is a direct child of the subtitle AdaptationSet only
  (elsewhere meaning is undefined). Scheme `urn:dvb:dash:fontdownload:2014`, `@value` = 1. Choose Essential when the
  signalled font must be used instead of the player's default, Supplemental otherwise. A non-supporting player ignores
  the whole set (Essential) or just the descriptor (Supplemental).
- Mandatory extra attributes (namespace `urn:dvb:dash:dash-extensions:2014-1`; declare it in the MPD; schema only in
  the unavailable attachment):

| Attribute | Type | Meaning |
|---|---|---|
| `url` | xs:anyURI | font location; resolved like a media URL (BaseURL applies) |
| `fontFamily` | xs:string | family name used in the EBU-TT-D documents |
| `mimeType` | xs:string | MIME type of the font resource |

- Formats (Table 21): OFF (ISO/IEC 14496-22:2009) = `application/font-sfnt`; WOFF 1.0 = `application/font-woff`.
  Vertical-script fonts shall include `vhea`. Line spacing should use hhea / OS/2 / vhea metrics, defaulting to the
  OS/2 `sTypo*` values; that is why only formats guaranteeing those metrics are allowed.
- Errors: failed download or unusable font -> Essential: do not present the set; Supplemental: present as if absent.
- Players should support font download in the listed formats; treat the scheme as unrecognised if mandatory attributes
  are missing or it sits outside a subtitle AdaptationSet.
- 7.3 informative example: a `stpp` English set (`mimeType="application/mp4"`, Role `main`, Accessibility 2) with a
  Supplemental font descriptor using `dvb:`-prefixed attributes (no `xmlns:dvb` shown in the excerpt) pointing at a
  `.woff` file with MIME `application/font-woff` and family `easyread`; SegmentTemplate with timescale 1000, duration
  10000, `startNumber` 1, Representation bandwidth 20000.
