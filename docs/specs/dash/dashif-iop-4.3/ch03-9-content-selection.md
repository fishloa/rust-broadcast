---
source: private/specs/dashif_iop_v4.3_guidelines.pdf
clauses: 3.9 Annotation and Client Model for Content Selection (3.9.1-3.9.5, Table 4)
pdf_pages: 54-72
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch03-9-content-selection.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# DASH-IF IOP v4.3 — 3.9 Content selection annotation and client model (notes)

PDF p.N = printed p.N-21. Verbatim: private file above.

## 3.9.1-3.9.2 Purpose (pp.54-55)

A client normally plays at most one Adaptation Set per media type, chosen from MPD labels plus client capabilities, client/user preferences and optional interaction; exceptions: dependent Representations across sets and application-driven multi-set playback. Sets may differ in codec, source format (SDR/HDR master), language or camera view. Table 4 is the restricted label set DASH-IF clients are expected to recognise; other organisations may add more.

## Table 4 — labels and usage (pp.55-62)

Use codes as printed (M mandatory, O optional, OD optional with default, "0..N" occurrence). Refs "T5"/"T9" = 23009-1 Table 5 (5.3.3.2) / Table 9 (5.3.7.2).

Any media type:
| Item | Use | Gloss |
|---|---|---|
| `@profiles` | O | inherits from MPD/Period; at least one Table 1/2 value must be present or inherited |
| `@group` | O | shall differ at least across media types; if present > 0; same group = same group value; only one Representation per group presented at a time, several groups of one type allowed for simultaneous presentation |
| `@selectionPriority` | OD, default 1 | disambiguates sets within a group; expresses MPD author preference (e.g. more efficient codec) |
| `ContentProtection` | 0..N | present means protected, absent means unprotected; see chapter 7 |
| `EssentialProperty` | 0..N | recognised scheme: `http://dashif.org/guidelines/trickmode` (3.2.9) |
| `SupplementalProperty` | (blank in source) | recognised schemes: `urn:mpeg:dash:adaptation-set-switching:2016` (3.8), the trickmode URL (3.2.9), `urn:mpeg:dash:period-continuity:2015` and `urn:mpeg:dash:period-connectivity:2015` (3.2.12); never used for differentiation, may improve operation |
| `Viewpoint` | 0..N | distinguishes media by viewpoint; absent = no differentiation |
| `Label` | 0..N | text for selection UI; must not be the only differentiator because start-up has no user interaction |

Video:
| Item | Use | Gloss |
|---|---|---|
| `@mimeType` | M | `video/mp4` |
| `@codecs` | M | codecs of Table 19 and Table 21 must be recognised |
| `@par` | O | shall be present if aspect ratio is a differentiator |
| `@maxWidth`, `@maxHeight` | O | should be present: max width/height in samples after cropping (should equal the maximum SPS sample count) |
| `@maxFrameRate` | O | should be present |
| `@scanType` | OD, default progressive | expected absent or `progressive` |
| `EssentialProperty` | 0..N | recognised: `urn:mpeg:mpegB:cicp:<Parameter>` (ISO/IEC 23001-8 [49]) with ColourPrimaries, TransferCharacteristics or MatrixCoefficients |
| `Accessibility` | 0..N | Role scheme (`urn:mpeg:dash:role:2011`) values `sign`, `captions`; or CEA-608 scheme `urn:scte:dash:cc:cea-608:2015` (6.4.3.3) |
| `Role` | 0..N | scheme above; values caption, subtitle, main, alternate, supplementary, sign, emergency; default main |
| `Rating` | 0..N | no scheme defined; sets using it may be ignored |

Audio:
| Item | Use | Gloss |
|---|---|---|
| `@mimeType` | M | `audio/mp4` |
| `@codecs` | M | Tables 22, 25-29 |
| `@lang` | O | should be present; absent = unknown |
| `@audioSamplingRate` | O | may be present for output devices limited to some rates |
| `AudioChannelConfiguration` | 0..N | recognised schemes: `urn:mpeg:dash:23003:3:audio_channel_configuration:2011`, `urn:mpeg:mpegB:cicp:ChannelConfiguration`, `tag:dolby.com,2014:dash:audio_channel_configuration:2011` |
| `EssentialProperty` | 0..N | recognised: `urn:mpeg:dash:audio-receiver-mix:2014` (23009-1 5.8.5.7) |
| `Accessibility` | 0..N | Role-scheme values `description`, `enhanced-audio-intelligibility` |
| `Role` | 0..N | main, alternate, supplementary, commentary, dub, emergency; default main |
| `Rating` | 0..N | as video |

Subtitle:
| Item | Use | Gloss |
|---|---|---|
| `@mimeType` | M | `application/mp4` or `application/ttml+xml` |
| `@codecs` | O | Table 23 |
| `@lang` | O | should be present |
| `Accessibility` | 0..N | Role-scheme values caption, sign |
| `Role` | 0..N | main, alternate, subtitle, supplementary, commentary, dub, description, emergency; default main |

> **Notes (Table 4):** `SupplementalProperty` has an empty use cell; the video `@maxFrameRate` text has a stray trailing fragment and the `@maxHeight` text speaks of a "horizontal" sample count (copy slip); audio and subtitle `Role` rows say "together with the Accessibility descriptor"; "Adaption Set" misspelling; video `Accessibility` cites T9 while other rows cite T5; the video Accessibility sublist items carry stray "o" bullets.

## 3.9.3 Content model (pp.62-63)

Figure 3 layers, top to bottom: Content (asset description) -> media types (video, audio, subtitle, application) -> alternative content per type (main, alternative 1, 2; chosen by an application or user) -> target versions = Adaptation Sets (chosen automatically by system capabilities/preferences) -> encoded Representations (dynamic switching). Ideas: a client does not auto-select alternatives (camera views) but needs a default, the *main* content; alternatives are every non-main content; *associated* content links different media types (e.g. commentary for camera 1). Target versions differ by preparation (downmix, resolution, language, trick-mode suitability), client codec/capability preferences or user preferences; a client selects at most one target version per group; authors can give priorities (e.g. prefer HEVC, native over dubbed language); each target version is one Adaptation Set (scalable codecs excepted); switching among Representations is independent of media type and target version.

> **Notes:** the text says "Figure 1" but the figure is Figure 3; it refers to 3.2.12 for media-type signalling although that is 3.2.13.

## 3.9.4 Signalling rules (pp.64-68)

- Alternatives (3.9.4.2): when a Period has alternative content, alternatives should be distinguishable and one should be main. Main = Role `main`; alternative = Role `alternative` (sic); no Role = main. If main and alternative both exist, alternatives shall carry at least one of: a `ViewPoint` (same `@schemeIdUri` across the alternatives of a type, differing `@value`), or a `Label` (same `@id`, differing text). ViewPoint suits a target application that understands the scheme; Label suits clients with a UI.
- Associated content (3.9.4.3): media types belonging together share the same ViewPoint scheme and value; a client that does not understand the scheme still obeys the association (e.g. pick video by label, play audio by ViewPoint).
- Within one media type and alternative, sets shall differ in at least one of: `@profiles`; `ContentProtection`; `EssentialProperty`; or the per-type labels below. Sets with `EssentialProperty` outside the permitted values should not occur. `@selectionPriority` should differ; otherwise expect random selection (3.9.4.4).
- Video (3.9.4.5): `@codecs` (including profile/level), `@maxWidth`/`@maxHeight`, `@maxFrameRate`, `EssentialProperty`, or Accessibility (Role `sign`/`caption`/`subtitle`, where caption/subtitle on video means open burned-in text; or the CEA-608 scheme meaning SEI-carried 608). Sets with `Rating`, `FramePacking` or non-progressive `@scanType` should not be present. Use `@selectionPriority`, in particular to rank burned-in-caption sets below others.
- Audio (3.9.4.6): `@codecs`, `@lang` (dominant language), `@audioSamplingRate` (maximum), `AudioChannelConfiguration` (should be present; unknown if absent; schemes depend on codec table: Tables 22/27/28/29 -> the MPEG schemes; ec-3/ac-4 of Table 25 -> the Dolby tag scheme, see 9.2.1.2; Table 26 -> DTS spec [39]), `EssentialProperty`, Accessibility (`description`, `enhanced-audio-intelligibility`). `Rating` may be ignored by clients.
- Subtitle (3.9.4.7): `@codecs`, `@lang`, `EssentialProperty`, Accessibility (`description` or `caption`).
- Auxiliary data (3.9.4.8): Role values caption, subtitle, main, alternate, supplementary, sign, emergency, dub; SupplementalProperty pairs for trickmode (value = id of the main set), period continuity, period connectivity, set switching (comma list of ids).

> **Notes (disagreements/mis-references):** 3.9.4.4 points at "3.10.4.5-7" for the per-type labels; they are 3.9.4.5-3.9.4.7. 3.9.4.4 contains the editorial remark about the content-protection options and a task force. The Role value is `alternative` in 3.9.4.2 and 3.9.5.2 but `alternate` in Table 4 and 3.9.4.8. 3.9.4.8 says the period-continuity/connectivity value is an *Adaptation Set* id, 3.2.12 says a *Period* id (p.68 vs p.47). "Aa @schemeIdUri", "stll", "the the" are typos.

## 3.9.5 Client processing reference model (pp.68-72)

Platform inputs assumed: per codec string yes/no decode; per DRM string yes/no; max resolution and frame rate; audio channel configuration and sampling rate; preferred language; accessibility settings (captions, subtitles, audio description, enhanced intelligibility); other preferences. If something is unknown the client degrades (e.g. uses `@selectionPriority` when it has no language preference). Purpose: default behaviour and a reference for authors to test annotation.

Algorithm at the joined Period (first Period for on-demand, live edge for live):
1. Exclude sets with Role `alternative` (main content first; an upfront alternatives review changes the sequence but not the rest).
2. Exclude sets failing codec, DRM or rendering capability.
3. If CEA-608 is not rendered, drop the 608 Accessibility descriptors (keep the set for ordinary video).
4. Accessibility preferences:
   - captions requested: take video sets with Accessibility (Role `caption` or the 608 scheme) and subtitle sets with Accessibility Role `caption`; if several, drop those not in the preferred language (when a language is set and some set matches; sets without language are dropped if any set has one), then highest `@selectionPriority`, then random. Not requested: caption Accessibility may be dropped.
   - sign language requested: video sets with Role-scheme Accessibility `sign`; else drop that signalling.
   - audio description requested: sets with Accessibility `description` (printed as "video Adaptation Sets"; presumably audio, kept as printed); else drop.
   - enhanced intelligibility requested: audio sets with that value; else drop.
5. Video: exclude sets with an unrecognised Essential descriptor and, from the *initial* selection, trickmode sets; then highest `@selectionPriority`; then free choice (random allowed). A selection may span several sets if switching is signalled, but only one is selected.
6. Audio: exclude unrecognised Essential; language filter as above; priority; free choice; set switching or receiver mix may widen the selection.
7. Subtitles: exclude unrecognised Essential; language filter; priority; free choice.
8. A client able to switch to alternative content may do so via `Label` or `ViewPoint`, dynamically; after the choice, steps 2 onward apply again.
9. At a Period boundary look for period continuity/connectivity (a set continuing the current one); if none, restart at step 1.

> **Notes:** the step-4 sub-bullets for subtitle sets and "makes those available" are indented one level deeper than the video bullet (p.70); caption literals lose a closing quote and gain a space after the opening quote; the step-5 trickmode exclusion has no counterpart for audio/subtitles.
