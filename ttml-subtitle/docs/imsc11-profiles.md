# IMSC 1.1 profile constraints reference

Source: **W3C Recommendation, "TTML Profiles for Internet Media Subtitles and
Captions 1.1" (IMSC 1.1)**, 08 November 2018, edited in place 27 April 2020.
This version: `https://www.w3.org/TR/2018/REC-ttml-imsc1.1-20181108/`. Latest:
`https://www.w3.org/TR/ttml-imsc1.1/`. Fetched 2026-07-30 (see `README.md` in
this directory for provenance). Section numbers below are this document's own
numbering (not TTML2's).

IMSC 1.1 defines two profiles of TTML2 (`ttml2-syntax.md` in this directory):
the **Text Profile** and the **Image Profile**. Both are built from the same
three sections of this spec: §6 (Supported Features and Extensions) + §7
(Common Provisions) + either §8 (Text Profile Provisions) or §9 (Image Profile
Provisions). This is the profile that actually ships in broadcast/streaming —
be precise about what it forbids relative to bare TTML2.

## 1. Conformance (§4, near-verbatim)

- A **Document Instance** conforming to a profile herein: SHALL satisfy all
  normative provisions of the profile; MAY include vocabulary/syntax/attribute
  values whose Feature/Extension disposition (§2 below) is `permitted` or
  `optional`; SHALL NOT include any whose disposition is `prohibited`.
  (It is, by definition, also a conforming TTML1 Document Instance per
  TTML2 §3.1.)
- A **presentation processor**: SHALL meet TTML2's Generic Processor
  Conformance (TTML2 §3.2.1); SHALL satisfy all normative provisions of the
  profile; SHALL implement presentation-semantic support for every
  Feature/Extension designated `permitted` or `permitted-deprecated`
  (subject to the profile's additional constraints on each); MAY implement
  support for `optional` ones.
- A **transformation processor**: same as above but "transformation-semantic
  support" instead of presentation.
- `permitted-deprecated` is intended to become `optional` or `prohibited` in
  a future version of this spec — an implementer should not build new
  reliance on it.
- A processor conforming to one profile (Text or Image) is **not** required
  to conform to the other. This spec does not define behavior when processing
  a non-conformant Document Instance.
- The `permitted`/`prohibited` dispositions here are a distinct concept from
  a `ttp:feature`/`ttp:extension` element's own `value="permitted"` /
  `"prohibited"` (that's TTML2's inline per-document profile mechanism,
  `ttml2-syntax.md` §3.1) — do not conflate the two.

## 2. Profiles overview (§5)

- **Text Profile** (§5.2): timed text expressed using Unicode text
  exclusively. Comprises §6 + §7 + §8.
- **Image Profile** (§5.3): timed text expressed using bitmap images
  exclusively. Comprises §6 + §7 + §9.
- It is (with narrow exceptions — e.g. a Document Instance with no `p`,
  `span`, `br`, `image` element and no `smpte:backgroundImage` attribute)
  **not generally possible** for one Document Instance to conform to both
  profiles simultaneously, and never possible for it to present both text
  data and image data at once. Applications needing both forms SHOULD offer
  two separate Document Instances, one per profile, associated so assistive
  technology can find the text form when image content is shown.
- **Profile Resolution / Override (§5.4)**: TTML2's own Profile Semantics
  apply (see `ttml2-syntax.md` §3.1). If any `ebuttm:conformsToStandard`
  element ([EBU-TT-M]) is present with one of the following values, it
  **overrides** all other content-profile determination:

  | Designator value | Resolves to |
  |---|---|
  | IMSC 1.0.1 Text Profile Designator | Text Profile |
  | IMSC 1.0.1 Image Profile Designator | Image Profile |
  | This spec's Image Profile Designator (§9.1) | Image Profile |
  | This spec's Text Profile Designator (§8.1) | Text Profile |
  | `urn:ebu:tt:distribution:2014-01` ([EBU-TT-D]) | Text Profile |
  | `urn:ebu:tt:distribution:2018-04` ([EBU-TT-D]) | Text Profile |

- The Text Profile *processor* profile includes all features required by
  each of: [ttml10-sdp-us], [EBU-TT-D], [ttml-imsc1.0.1 Text Profile]. The
  Image Profile processor profile includes all features required by
  [ttml-imsc1.0.1 Image Profile].
- Example (informative, §5.4.1): `mimeType="application/mp4"` +
  `codecs="stpp.ttml.im1t"` in a DASH Manifest signals a track of
  [ttml-imsc1.0.1] Text Profile documents.

## 3. Namespaces (§7.3, Table, verbatim)

| Name | Prefix | Value | Defining spec |
|---|---|---|---|
| XML | `xml` | `http://www.w3.org/XML/1998/namespace` | [xml-names] |
| TT | `tt` | `http://www.w3.org/ns/ttml` | [ttml2] |
| TT Parameter | `ttp` | `http://www.w3.org/ns/ttml#parameter` | [ttml2] |
| TT Styling | `tts` | `http://www.w3.org/ns/ttml#styling` | [ttml2] |
| TT Feature | (none) | `http://www.w3.org/ns/ttml/feature/` | [ttml2] |
| SMPTE-TT Extension | `smpte` | `http://www.smpte-ra.org/schemas/2052-1/2010/smpte-tt` | [SMPTE2052-1] |
| EBU-TT Styling | `ebutts` | `urn:ebu:tt:style` | [EBU-TT-D] |
| EBU-TT Metadata | `ebuttm` | `urn:ebu:tt:metadata` | [EBU-TT-D] |
| **IMSC Styling** | `itts` | `http://www.w3.org/ns/ttml/profile/imsc1#styling` | this spec |
| **IMSC Parameter** | `ittp` | `http://www.w3.org/ns/ttml/profile/imsc1#parameter` | this spec |
| **IMSC Metadata** | `ittm` | `http://www.w3.org/ns/ttml/profile/imsc1#metadata` | this spec |
| **IMSC Extension** | (none) | `http://www.w3.org/ns/ttml/profile/imsc1/extension/` | this spec |
| IMSC 1.1 Text Profile Designator | (none) | see §8.1 below | this spec |
| IMSC 1.1 Image Profile Designator | (none) | see §9.1 below | this spec |

Prefixes are conventions only — a Document Instance may bind any prefix to
these namespace URIs. All namespaces defined by this spec are **mutable**:
undefined names within them are reserved for future W3C standardization.

## 4. Profile designators (§8.1, §9.1)

| Profile | Designator (absolute URI) |
|---|---|
| IMSC 1.1 Text | `http://www.w3.org/ns/ttml/profile/imsc1.1/text` |
| IMSC 1.1 Image | `http://www.w3.org/ns/ttml/profile/imsc1.1/image` |

(The older IMSC 1.0/1.0.1 designators — `.../imsc1/text`, `.../imsc1/image`
— appear in examples throughout this spec for backward-compatibility
signalling; they are a *different*, earlier-version profile, not aliases of
the 1.1 designators above. See §L "Summary of substantive changes" for the
1.0.1→1.1 diff, not transcribed in full here — not read in detail, listed
under "could not establish" in `README.md`.)

## 5. Feature/Extension disposition table (§6, Table, verbatim, all 159 rows)

Legend: `permitted` = a conforming Document Instance MAY use it, a conforming
processor MUST support it. `permitted-deprecated` = permitted now but
expected to become `optional` or `prohibited` in a future version — treat as
"supported for compatibility, do not rely on for new content". `prohibited`
= **a conforming Document Instance MUST NOT use it; a conforming parser that
sees it in a Document Instance claiming this profile should treat the
document as non-conformant.** "Partially supported via X" = the feature
itself is not directly assigned a disposition; only the narrower
sub-feature(s) named are. No processor requirement given = informative only
(does not change parse/reject behaviour).

**Baseline rule that governs every row below**: *"All features specified in
[TTML2] are prohibited unless specified otherwise below."* — i.e. anything
not listed in this table, or in the TTML2 feature catalog at all, is
prohibited by default in both profiles. This is the single most
implementation-relevant sentence in this document: a conformant IMSC 1.1
parser/validator must reject any TTML2 feature use that is not explicitly
`permitted`/`permitted-deprecated`/`optional` for the claimed profile.

### 5.1 Relative to the TT Feature namespace

| Feature | Text Profile | Image Profile |
|---|---|---|
| `#animation` | permitted | permitted |
| `#animation-version-2` | Partially supported via `#animation`. | Partially supported via `#animation`. |
| `#background` | Partially supported via `#backgroundColor`. | Partially supported via `#backgroundColor`. |
| `#backgroundColor` | permitted | Partially supported via `#backgroundColor-region` and `#backgroundColor-block`. |
| `#backgroundColor-block` | permitted | permitted |
| `#backgroundColor-inline` | permitted | prohibited |
| `#backgroundColor-region` | permitted | permitted |
| `#base` | permitted | permitted |
| `#base-version-2` | Partially supported via `#base`. | Partially supported via `#base`. |
| `#bidi` | permitted | prohibited |
| `#bidi-version-2` | Partially supported via `#bidi` and `#unicodeBidi-version-2`. | prohibited |
| `#cellResolution` | permitted | permitted |
| `#color` | permitted — §8.4.1 additional constraints | prohibited |
| `#content` | permitted | permitted |
| `#contentProfiles` | permitted | permitted |
| `#core` | permitted | permitted |
| `#direction` | permitted | prohibited |
| `#disparity` | permitted | permitted |
| `#display` | permitted | permitted |
| `#display-block` | permitted | permitted |
| `#display-inline` | permitted | permitted |
| `#display-region` | permitted | permitted |
| `#display-version-2` | Partially supported via `#display`, `#display-block`, `#display-inline`, `#display-region`. | (same) |
| `#displayAlign` | permitted | prohibited |
| `#displayAlign-region` | permitted | prohibited |
| `#displayAlign-relative` | permitted | prohibited |
| `#displayAlign-version-2` | Partially supported via `#displayAlign-region`, `#displayAlign-relative`. | prohibited |
| `#displayAspectRatio` | permitted — §7.12.5 additional constraints | (same) |
| `#extent` | permitted | permitted |
| `#extent-full-version-2` | Partially supported via `#extent-version-2`. | (same) |
| `#extent-image` | **prohibited** | permitted — §9.4.4 additional constraints |
| `#extent-length` | permitted | permitted |
| `#extent-length-version-2` | permitted | see disposition of `#length-version-2` |
| `#extent-region` | Partially supported via `#extent-length` — §8.4.2 additional constraints | Partially supported via `#extent-length` — §9.4.2 additional constraints |
| `#extent-region-version-2` | Partially supported via `#extent-region`. | (same) |
| `#extent-root` | Partially supported via `#extent-length` — §7.12.6 additional constraints | (same) |
| `#extent-root-version-2` | Partially supported via `#extent-root`. | (same) |
| `#extent-version-2` | Partially supported via `#extent`, `#extent-region-version-2`. | Partially supported via `#extent`, `#extent-image`, `#extent-region-version-2`. |
| `#fontFamily` | permitted — §8.4.4 additional constraints | **prohibited** |
| `#fontFamily-generic` | permitted — §8.4.3 additional constraints | **prohibited** |
| `#fontFamily-non-generic` | permitted | **prohibited** |
| `#fontSize` | Partially supported via `#fontSize-isomorphic`. | **prohibited** |
| `#fontSize-isomorphic` | permitted | **prohibited** |
| `#fontStyle` | permitted | **prohibited** |
| `#fontStyle-italic` | permitted | **prohibited** |
| `#fontStyle-oblique` | permitted | **prohibited** |
| `#fontWeight` | permitted | **prohibited** |
| `#fontWeight-bold` | permitted | **prohibited** |
| `#frameRate` | permitted — §7.12.7 additional constraints | (same) |
| `#frameRateMultiplier` | permitted | permitted |
| `#image` | **prohibited** | permitted — §9.4.4 additional constraints |
| `#image-png` | **prohibited** | permitted — §9.3 additional constraints |
| `#initial` | permitted | **prohibited** |
| `#layout` | permitted — §7.12.1 additional constraints | (same) |
| `#length` | permitted | Partially supported via `#length-integer`, `#length-real`, `#length-positive`, `#length-negative`, `#length-cell`, `#length-percentage`, `#length-pixel`. |
| `#length-cell` | permitted — §7.12.8 additional constraints | (same) |
| `#length-em` | permitted | **prohibited** |
| `#length-integer` | permitted | permitted |
| `#length-negative` | permitted — §8.4.5 additional constraints | permitted — §9.4.3 additional constraints |
| `#length-percentage` | permitted | permitted |
| `#length-pixel` | permitted | permitted |
| `#length-positive` | permitted | permitted |
| `#length-real` | permitted | permitted |
| `#length-root-container-relative` | permitted — §7.12.9 additional constraints | **prohibited** |
| `#length-version-2` | permitted | Partially supported via `#length`. |
| `#lineBreak-uax14` | processor SHALL implement | no processor requirement specified |
| `#lineHeight` | permitted — §8.4.6 additional constraints | **prohibited** |
| `#luminanceGain` | permitted | permitted |
| `#metadata` | permitted | permitted |
| `#metadata-item` | permitted — §7.12.2 additional constraints | (same) |
| `#metadata-version-2` | permitted | permitted |
| `#nested-div` | permitted | **prohibited** |
| `#nested-span` | permitted | **prohibited** |
| `#opacity` | permitted | permitted |
| `#opacity-region` | permitted | permitted |
| `#opacity-version-2` | Partially supported via `#opacity`. | (same) |
| `#origin` | permitted — §8.4.7 additional constraints | permitted |
| `#overflow` | permitted | permitted |
| `#overflow-visible` | permitted | permitted |
| `#padding` | permitted | **prohibited** |
| `#padding-1` | permitted | **prohibited** |
| `#padding-2` | permitted | **prohibited** |
| `#padding-3` | permitted | **prohibited** |
| `#padding-4` | permitted | **prohibited** |
| `#padding-region` | permitted | **prohibited** |
| `#padding-version-2` | Partially supported via `#padding`, `#padding-1..4`, `#padding-region`. | **prohibited** |
| `#position` | permitted — §8.4.8 additional constraints | **prohibited** |
| `#presentation` | permitted | permitted |
| `#presentation-version-2` | Partially supported via `#presentation`, `#profile-version-2`. | (same) |
| `#profile` | permitted | permitted |
| `#profile-full-version-2` | Partially supported via `#contentProfiles`, `#profile`, `#profile-version-2`. | (same) |
| `#profile-version-2` | Partially supported via `#contentProfiles`, `#profile`. | (same) |
| `#region-timing` | permitted | permitted |
| `#ruby` | permitted | **prohibited** |
| `#ruby-full` | Partially supported via `#ruby`, `#rubyAlign`, `#rubyPosition`, `#rubyReserve`. | **prohibited** |
| `#rubyAlign` | Partially supported via `#rubyAlign-minimal`. | **prohibited** |
| `#rubyAlign-minimal` | permitted — §8.4.9 additional constraints | **prohibited** |
| `#rubyPosition` | permitted | **prohibited** |
| `#rubyReserve` | permitted | **prohibited** |
| `#set` | permitted | permitted |
| `#shear` | permitted | **prohibited** |
| `#showBackground` | permitted | permitted |
| `#structure` | permitted | permitted |
| `#styling` | permitted | permitted |
| `#styling-chained` | permitted | permitted |
| `#styling-inheritance-content` | permitted | permitted |
| `#styling-inheritance-region` | permitted | permitted |
| `#styling-inline` | permitted | permitted |
| `#styling-nested` | permitted | permitted |
| `#styling-referential` | permitted | permitted |
| `#textAlign` | permitted | **prohibited** |
| `#textAlign-absolute` | permitted | **prohibited** |
| `#textAlign-relative` | permitted | **prohibited** |
| `#textAlign-version-2` | Partially supported via `#textAlign`, `#textAlign-relative`, `#textAlign-absolute`. | **prohibited** |
| `#textCombine` | permitted | **prohibited** |
| `#textDecoration` | permitted | **prohibited** |
| `#textDecoration-over` | permitted | **prohibited** |
| `#textDecoration-through` | permitted | **prohibited** |
| `#textDecoration-under` | permitted | **prohibited** |
| `#textEmphasis` | Partially supported via `#textEmphasis-minimal`. | **prohibited** |
| `#textEmphasis-minimal` | permitted | **prohibited** |
| `#textOutline` | Partially supported via `#textOutline-unblurred`. | **prohibited** |
| `#textOutline-unblurred` | permitted — §8.4.10 additional constraints | **prohibited** |
| `#textShadow` | permitted — §8.4.11 additional constraints | **prohibited** |
| `#tickRate` | permitted — §7.12.10 additional constraints | (same) |
| `#timeBase-media` | permitted | permitted |
| `#timeContainer` | permitted | permitted |
| `#time-clock` | permitted | permitted |
| `#time-clock-with-frames` | permitted | permitted |
| `#time-offset` | permitted | permitted |
| `#time-offset-with-frames` | permitted | permitted |
| `#time-offset-with-ticks` | permitted | permitted |
| `#timing` | permitted — §7.12.13 additional constraints | (same) |
| `#transformation` | permitted | permitted |
| `#transformation-version-2` | Partially supported via `#transformation`, `#profile-version-2`. | (same) |
| `#unicodeBidi` | permitted | **prohibited** |
| `#unicodeBidi-version-2` | Partially supported via `#unicodeBidi`. | **prohibited** |
| `#visibility` | permitted | Partially supported via `#visibility-block`, `#visibility-region`. |
| `#visibility-block` | permitted | permitted |
| `#visibility-image` | **prohibited** | permitted |
| `#visibility-inline` | permitted | **prohibited** |
| `#visibility-region` | permitted | permitted |
| `#visibility-version-2` | Partially supported via `#visibility`. | Partially supported via `#visibility`, `#visibility-image`. |
| `#wrapOption` | permitted | **prohibited** |
| `#writingMode` | permitted | Partially supported via `#writingMode-horizontal`. |
| `#writingMode-horizontal` | permitted | Partially supported via `#writingMode-horizontal-lr`, `#writingMode-horizontal-rl`. |
| `#writingMode-horizontal-lr` | permitted | permitted |
| `#writingMode-horizontal-rl` | permitted | permitted-deprecated |
| `#writingMode-vertical` | permitted | **prohibited** |
| `#zIndex` | permitted-deprecated | permitted-deprecated |

### 5.2 Relative to the SMPTE-TT Extension Namespace

| Feature | Text Profile | Image Profile |
|---|---|---|
| `#image` | **prohibited** | permitted-deprecated — §9.4.5 additional constraints |

### 5.3 Relative to the IMSC Extension namespace

(Relative URIs below are resolved against the IMSC Extension Namespace base:
`http://www.w3.org/ns/ttml/profile/imsc1/extension/`.)

| Extension | Text Profile | Image Profile |
|---|---|---|
| `#activeArea` | permitted | permitted |
| `#altText` | permitted-deprecated — §7.12.3 additional constraints | (same) |
| `#aspectRatio` | permitted-deprecated — §7.12.4 additional constraints | (same) |
| `#fillLineGap` | permitted | **prohibited** |
| `#forcedDisplay` | permitted | permitted |
| `#linePadding` | permitted — §8.4.12 additional constraints | permitted-deprecated |
| `#multiRowAlign` | permitted — §8.4.13 additional constraints | permitted-deprecated |
| `#progressivelyDecodable` | permitted-deprecated | permitted-deprecated |

## 6. Common Provisions constraints a conforming processor must enforce (§7)

Applies to **both** profiles.

- **§7.1 Document Encoding**: SHALL be well-formed XML 1.0, UTF-8 encoded.
  SHOULD NOT contain entity declarations or entity references other than to
  predefined entities (intended to become prohibited in a future version).
  Character references and predefined-entity references are fine.
- **§7.2 Foreign Elements/Attributes**: MAY be present if neither
  specifically permitted nor forbidden by the profile. A transformation
  processor SHOULD preserve them. (TTML2's own content-conformance algorithm
  prunes elements/attributes in namespaces other than any TT namespace
  *before* conformance is evaluated — so truly foreign-namespace vocabulary
  does not itself make a document non-conformant.)
- **§7.4 Overflow**: author SHOULD assume strict clipping regardless of the
  computed `tts:overflow` value (per TTML2, `tts:overflow` doesn't affect a
  region's extent for drawing-area purposes).
- **§7.6 Synchronization**: when mapping a media time expression M to a
  frame F of a Related Video Object, the presentation processor SHALL map M
  to the frame with presentation time closest to, but not less than, M.
- **§7.7 Root Container Region mapping**: the Root Container Region SHALL be
  mapped to a rectangular area within each video frame such that: the
  width:height ratio equals the RCR's display aspect ratio (TTML2 Appendix
  H); the rectangle is centered on the frame center; it lies entirely within
  the frame; and it has height or width equal to the frame's.
- **§7.9 Profile Signaling**: `ttp:contentProfiles` SHOULD be present on
  `tt`, with exactly one value equal to the Text or Image profile designator
  (§4 above). (Prohibited under some profiles this doc references, e.g.
  EBU-TT-D.) When using `ebuttm:conformsToStandard`, the designators of the
  Text/Image Profile SHALL be used for the corresponding conformance claim.
- **§7.10 Hypothetical Render Model**: it SHALL be possible to apply §10
  Hypothetical Render Model to any sequence of consecutive intermediate
  synchronic documents without error.
- **§7.11 Style Resolution**: `itts:fillLineGap`, `itts:forcedDisplay`,
  `ebutts:linePadding`, `ebutts:multiRowAlign` are SHALL be subject to
  TTML2's §10.4 Style Resolution procedures (i.e. they behave like ordinary
  inheritable style properties, including via the `initial` element).

### 6.1 §7.12 Constraints — the "must reject" list

These are hard, structural constraints, independent of which named feature
they're filed under. **A conformant validator/parser must be able to detect
violations of every one of these** — they are exactly the kind of thing an
implementer is likely to under-enforce:

- **§7.12.1.1 Presented region** — a temporally-active region counts as
  "presented" only if: computed `tts:opacity` ≠ `"0.0"`; AND computed
  `tts:display` ≠ `"none"`; AND computed `tts:visibility` ≠ `"hidden"`; AND
  either (a) content is selected into it, or (b) computed `tts:showBackground`
  = `"always"` and computed `tts:backgroundColor` has non-transparent alpha.
- **§7.12.1.2 Dimensions and position** — **all regions SHALL NOT extend
  beyond the Root Container Region** (every region coordinate ⊆ RCR
  coordinates). **No two presented regions in a given intermediate synchronic
  document SHALL overlap** (coordinate-set intersection must be empty).
- **§7.12.1.3 Maximum number** — **the number of presented regions in a
  given intermediate synchronic document SHALL NOT be greater than 4.**
- **§7.12.2** — an `altText` named metadata item SHALL NOT be present if any
  `ittm:altText` element is also present (mutually exclusive equivalent
  mechanisms).
- **§7.12.3 `#altText`** — `ittm:altText` SHOULD NOT be present unless
  IMSC 1.0.1 compatibility is desired; SHALL NOT be present if an `altText`
  named metadata item is also present.
- **§7.12.4 `#aspectRatio`** — `ittp:aspectRatio` SHOULD NOT be present
  unless IMSC 1.0.1 compatibility is desired; **SHALL NOT be present if
  `ttp:displayAspectRatio` is also present** (mutually exclusive).
- **§7.12.5 `#displayAspectRatio`** — symmetric: `ttp:displayAspectRatio`
  SHALL NOT be present if `ittp:aspectRatio` is also present.
- **§7.12.6 `#extent-root`** — if the document includes any length value
  using `px` units, `tts:extent` SHALL be present on `tt`.
- **§7.12.7 `#frameRate`** — if the document includes any clock-time
  expression using the frames term, or any offset-time expression using the
  `f` metric, `ttp:frameRate` SHALL be present on `tt`.
- **§7.12.8 `#length-cell`** — `c` units SHALL NOT appear anywhere except as
  the value of `ebutts:linePadding`.
- **§7.12.9 `#length-root-container-relative`** — on `tts:extent` or
  `tts:position`: `rh` units SHALL NOT be used for horizontal components;
  `rw` units SHALL NOT be used for vertical components (keeps region
  dimension/overlap validation independent of RCR aspect ratio).
- **§7.12.10 `#tickRate`** — `ttp:tickRate` SHALL be present on `tt` if the
  document contains any time expression using the `t` metric.
- **§7.12.13 `#timing`** — for any content element containing `br` elements,
  text nodes, or a `smpte:backgroundImage` attribute, both `begin` and one of
  `end`/`dur` SHOULD be specified on that element or an ancestor.

## 7. Text Profile Provisions (§8)

- **§8.1 Designator**: `http://www.w3.org/ns/ttml/profile/imsc1.1/text`.
- **§8.2 Recommended Character Sets**: SHOULD use characters from Annex B
  Common Character Sets (not transcribed here — see README "could not
  establish" list).
- **§8.3 Reference Fonts**: when rendering codepoints matching Annex A's
  combinations of computed font family + codepoint, a processor SHALL use a
  font producing a substantially-identical glyph-sequence dimension to one of
  the specified reference fonts. Only covers a subset of Latin, Greek,
  Cyrillic, Hebrew, Arabic scripts.
- **§8.4 Constraints** (all normative, verbatim):
  - **§8.4.1 `#color`** — the *initial* value of `tts:color` SHALL be
    `"white"` (overriding TTML2's own unspecified/black-ish default — "see
    prose" in the base spec's property table).
  - **§8.4.2 `#extent-region`** — `tts:extent` SHALL be present on every
    `region` element, and SHALL use `px` units, percentage values, or
    root-container-relative units (i.e. never `em`/`c`).
  - **§8.4.3 `#fontFamily-generic`** — if computed `tts:fontFamily` is
    `"default"`, the *used* value SHALL be `"monospaceSerif"`.
  - **§8.4.4 `#fontFamily`** — linear whitespace SHOULD NOT appear between
    components of `tts:fontFamily`'s value.
  - **§8.4.5 `#length-negative`** — strictly negative length expressions
    SHALL NOT be used with any attribute other than `tts:disparity` and
    `tts:textShadow`. (Does not apply to `tts:shear`, a percentage that
    legitimately accepts negatives.)
  - **§8.4.6 `#lineHeight`** — the specified value SHOULD be such that every
    `p` element's style set contains a `tts:lineHeight` whose value is not
    `normal` (uneven `normal` implementation across processors).
  - **§8.4.7 `#origin`** — `tts:origin` SHALL use `px` units or percentage
    values; SHALL NOT be present if any `tts:position` is also present
    (mutually exclusive).
  - **§8.4.8 `#position`** — `tts:position` SHALL use `px`, percentage, or
    root-container-relative units; SHALL NOT be present if any `tts:origin`
    is also present.
  - **§8.4.9 `#rubyAlign`** — computed value SHALL be `center` or
    `spaceAround` only (narrower than TTML2's full 6-value set).
  - **§8.4.10 `#textOutline-unblurred`** — computed `tts:textOutline` on any
    `span` SHALL be ≤ 10% of the computed `tts:fontSize` on the same
    element.
  - **§8.4.11 `#textShadow`** — `tts:textShadow` SHALL NOT have more than 4
    `<shadow>` values.
  - **§8.4.12 `ebutts:linePadding`** — MAY be specified on `region`, `body`,
    `div`, `p`, `initial` (in addition to `style`); processor SHALL apply it
    to `p` only, and SHALL treat it as inheritable. Only supports `c` length
    units (see §7.12.8 above). [EBU-TT-D] itself only allows it on `style`
    — this is a documented divergence.
  - **§8.4.13 `ebutts:multiRowAlign`** — same placement/inheritance rule as
    `linePadding` above; also a documented divergence from [EBU-TT-D].

## 8. Image Profile Provisions (§9)

- **§9.1 Designator**: `http://www.w3.org/ns/ttml/profile/imsc1.1/image`.
- **§9.2.1 "Presented image" definition** — a `div` element with a
  `smpte:backgroundImage` attribute OR a child `image` element, that flows
  into a presented region.
- **§9.2.2 Constraint** — in a given intermediate synchronic document, each
  presented region SHALL contain **at most one** `div` element, which SHALL
  be a presented image.
- **§9.2.3** — for ISD construction purposes, a `div` with
  `smpte:backgroundImage` SHALL NOT be considered empty.
- **§9.3 Image Resources** — an image resource is a PNG datastream ([PNG]).
  If a `pHYs` chunk is present, it SHALL indicate square pixels (PNG's own
  default when absent).
- **§9.4 Constraints**:
  - **§9.4.1 `#content`** — `p`, `span`, `br` elements SHALL NOT be present
    at all (this is the single biggest structural difference from the Text
    Profile — an Image Profile document has no run-text content model).
  - **§9.4.2 `#extent-region`** — `tts:extent` SHALL be present on every
    `region`, and SHALL use `px` units only (stricter than Text Profile,
    which also allows percentage/root-relative).
  - **§9.4.3 `#length-negative`** — strictly negative lengths SHALL NOT be
    used with any attribute other than `tts:disparity` (note: Text Profile
    also permits `tts:textShadow`; Image Profile does not, consistent with
    `tts:textShadow` itself being prohibited in Image Profile per §5.1).
  - **§9.4.4 TTML `#image` Feature**:
    - `image` SHALL only be used in an image presentation context.
    - `image` SHALL be a child of a `div` that does **not** have a
      `smpte:backgroundImage` attribute.
    - A `div` SHALL have zero or one child `image` element.
    - A `div` with a child `image` SHOULD contain a `metadata` containing an
      `altText` named metadata item that is a Text Alternative of the image.
    - `image` SHALL specify `src` (referencing a §9.3-conformant resource),
      `type`, and `tts:extent` — the latter equal to *both* the region's
      `tts:extent` and the image resource's own pixel dimensions.
  - **§9.4.5 SMPTE `#image` Extension**:
    - **§9.4.5.1 `smpte:backgroundImage`** — MAY be used per [SMPTE2052-1]
      §5.5.2 semantics. If applied to a `div`: the referenced image's
      width/height (px) SHALL equal the `px`-unit `tts:extent` of the region
      the `div` presents in; the `div` SHOULD contain a `metadata` with an
      `ittm:altText` Text Alternative; the image SHALL conform to §9.3; and
      the `div` SHALL NOT have any `image` element as a descendant (the two
      mechanisms — `smpte:backgroundImage` vs. child `image` — are mutually
      exclusive per `div`).
    - **§9.4.5.2** `smpte:backgroundImageHorizontal` and
      `smpte:backgroundImageVertical` **SHALL NOT be used.**
    - **§9.4.5.3** `smpte:image` **SHALL NOT be used.**

## 9. Extension vocabulary (§7.8) — full element/attribute definitions

All are in the IMSC Styling (`itts:`), IMSC Parameter (`ittp:`), or IMSC
Metadata (`ittm:`) namespaces (§3 above). Designator URIs are relative to the
IMSC Extension Namespace base `http://www.w3.org/ns/ttml/profile/imsc1/extension/`.

### 9.1 `ittp:aspectRatio` (§7.8.1, attribute on `tt` only)
```
ittp:aspectRatio : numerator denominator   // int(numerator) != 0, int(denominator) != 0; NO whitespace between the two digit runs
numerator | denominator : <digit>+
```
Deprecated in favor of `ttp:displayAspectRatio` (equivalent semantics; the
two are mutually exclusive per §7.12.4/§7.12.5 above). If absent, aspect
ratio falls back to TTML2 Appendix H's derivation.

### 9.2 `ittp:progressivelyDecodable` (§7.8.2, attribute on `tt` only)
```
ittp:progressivelyDecodable : "true" | "false"
```
Default `"false"` if absent. A **progressively decodable** Document Instance
(all four conditions must hold): (1) no TTML timing vocabulary attribute or
element appears within `head`; (2) for any two intermediate synchronic
documents A (start TA) and B (start TB), if A contains a `p` lexically
preceding any `p` in B, then TA ≤ TB; (3) no TTML timing attribute on any
descendant of `p`; (4) no element E1 references another element E2 whose
opening tag is lexically *after* E1's. A value of `"true"` asserts this;
`"false"` asserts neither way.

### 9.3 `itts:forcedDisplay` (§7.8.3, style property)
```
Values: false | true
Initial: false
Applies to: body, div, p, region, span
Inherited: yes
Percentages: N/A
Animatable: discrete
```
Used with an out-of-band boolean processor parameter `displayForcedOnlyMode`
(default `"false"`, settable by the embedding application, not by the
document). **If and only if** `displayForcedOnlyMode` is `"true"`, a content
element with computed `itts:forcedDisplay` = `"false"` SHALL NOT produce any
visible rendering, regardless of computed `tts:visibility`. Has no effect on
layout/composition, only visibility. Note: a region's *background* can
remain visible under `showBackground="whenActive"`-adjacent rules even when
all its forced-display-false content is hidden — an author must give content
and its region matching `itts:forcedDisplay` values to avoid this.

### 9.4 `ittm:altText` (§7.8.4, element, child of `metadata` only)
```
<ittm:altText
  xml:id = ID
  xml:lang = string
  xml:space = (default|preserve)
  {any attribute not in the default namespace, any TT namespace, or any IMSC namespace}
  Content: #PCDATA
</ittm:altText>
```
Provides a text-equivalent string for an element (typically an image), for
indexing/QA — NOT intended to be displayed in place of the element (unlike
HTML `alt`), though assistive technology may read it. §9 (Image Profile)
specifies its use with images specifically.

### 9.5 `ittp:activeArea` (§7.8.5, attribute on `tt` only)
```
ittp:activeArea : leftOffset topOffset width height
leftOffset | topOffset | width | height : <percentage>   // non-negative, <= 100%
```
`width`/`height` are relative to the Root Container Region's width/height and
give the Active Area's size. `leftOffset`/`topOffset` specify an alignment
point; the Active Area's top-left origin `{x, y}` is computed as:
```
x = leftOffset * (1 - width/100)
y = topOffset * (1 - height/100)
```
so the Active Area can never extend outside the Root Container Region in any
dimension. If absent, the Active Area **is** the Root Container Region.
Analogous to broadcast Active Format Description (AFD) metadata — lets a
system avoid cropping subtitle-critical area when the video is cropped.

### 9.6 `itts:fillLineGap` (§7.8.6, style property, applies to `p` only)
```
Values: false | true
Initial: false
Applies to: p
Inherited: yes
Percentages: N/A
Animatable: discrete
```
If `"true"`, the background of every inline area generated by descendant
`span`s SHALL extend to the before-edge/after-edge of its containing line
area (eliminating visible gaps between successive lines/paragraphs sharing a
region).

## 10. Extension designations (Appendix F) — feature-support definitions

These restate, for each IMSC-defined extension, what "a transformation/
presentation processor supports the feature" means (used by the disposition
table in §5 above): `#progressivelyDecodable`, `#aspectRatio`,
`#forcedDisplay`, `#altText`, `#linePadding`, `#multiRowAlign`,
`#activeArea`, `#fillLineGap` — each is supported by a processor if it
recognizes/transforms (transformation) or presentation-implements
(presentation) the corresponding attribute/element named in §9 above (or, for
`#linePadding`/`#multiRowAlign`, the `ebutts:` attributes from [EBU-TT-D]).
No new normative constraints beyond §9/§6 — this appendix is the "what does
supporting X mean" definition, not additional restrictions.

## 11. Hypothetical Render Model (§10) — not transcribed

The paint/compositing algorithm (regions → images → text, §10.1–10.5) is a
*presentation*-processor concern (how to rasterize), not a parse/serialize
concern, and is not transcribed here — listed in README's "could not
establish" / "not needed" list.

## 12. Not transcribed here

- Annex A Reference Fonts (specific font names/metrics tables) — presentation
  concern; see README.
- Annex B Common Character Sets — recommended-input-charset guidance, not a
  hard constraint (§8.2 above already states the SHOULD).
- Annex C Forced content (worked example of `itts:forcedDisplay` combined
  with hard-of-hearing + translation subtitle tracks) — illustrative only,
  already summarized in §9.3 above.
- Annex D WCAG/MAUR Considerations, Annex G XML Schema Definitions, Annex H
  Extensibility Objectives, Annex J Acknowledgements, Annex K Privacy and
  Security Considerations — non-normative/administrative.
- Annex I Compatibility with other TTML-based specifications (EBU-TT-D,
  SDP-US, SMPTE-TT, CFF-TT, IMSC1/1.0.1) — cross-spec compatibility notes,
  not IMSC 1.1's own constraints; relevant only if this crate later needs to
  interoperate with those specific sibling profiles.
- Annex L Summary of substantive changes (IMSC1.0.1 → 1.1 diff) — historical,
  not needed for a from-scratch 1.1 implementation.
