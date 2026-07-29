# Fixture provenance

All fixtures in this directory are unmodified files taken verbatim from the
**W3C `imsc-tests` repository** (`https://github.com/w3c/imsc-tests`), the
official conformance test suite for the IMSC family of specifications
(referenced from `https://github.com/w3c/imsc/`). Fetched from the `main`
branch at commit `08f10c5d5c36ab1105202ed964aee8c4ae939ed5` (2026-04-20), on
2026-07-30.

**Licence**: dual-licensed under the W3C Test Suite License and the W3C
3-clause BSD License (licensee's choice) — see
`https://github.com/w3c/imsc-tests/blob/main/LICENSE.md`. Both permit
copying and redistribution "in any medium for any purpose and without fee or
royalty" provided a link/URL to the original document and the W3C copyright
notice are retained. That attribution is given here. No right to create
*modifications* of the test files is granted by this licence — accordingly
every file below is committed byte-for-byte as fetched; none has been edited.

These are real, hand-authored-by-spec-editors conformance test documents
(several originally contributed by Institut für Rundfunktechnik, per their
own file headers, under Apache 2.0, itself compatible with redistribution
here), not synthetic happy-path samples — they carry the real oddities
(mixed profile designator styles across TTML1/TTML2 syntax eras, EBU-TT-D
dual-conformance metadata, negative shadow offsets, multi-byte Japanese
text with ruby annotations) that a hand-written fixture would not.

## Files

| File | Source path (in `w3c/imsc-tests`) | What it exercises |
|---|---|---|
| `imsc1-document-example-822.ttml` | `imsc1/ttml/document/DocumentExample822.ttml` | Baseline Text-Profile document: `tt:tt`-prefixed root, `p`/`span`/`br`, `tts:backgroundColor`, `tts:textAlign`, `ttm:title`/`ttm:desc`/`ttm:copyright` metadata. Derived from the DFXP 1.0 §8.2.2 spec example. |
| `imsc1-time-expressions-001.ttml` | `imsc1/ttml/timing/TimeExpressions001.ttml` | Exhaustive `<time-expression>` syntax coverage: `s`/`m`/`h`/`f`/`t` metrics, plain clock-time, fractional clock-time, clock-time-with-frames, `ttp:frameRate`+`ttp:frameRateMultiplier`+`ttp:tickRate` all set together, `seq` time container. |
| `imsc1-animation-001.ttml` | `imsc1/ttml/animation/Animation001.ttml` | `<set>` animation element changing `tts:backgroundColor` mid-interval. |
| `imsc1-backgroundcolor-rgba-001.ttml` | `imsc1/ttml/backgroundColor/backgroundcolor-rgba-001.ttml` | `#rrggbbaa` color syntax, `ttp:cellResolution`, referential `style` chaining (`style` element + `style=IDREFS`), dual EBU-TT-D/IMSC1 conformance via `ebuttm:conformsToStandard`. IRT-contributed (Apache-2.0 origin, see file header). |
| `imsc1.1-ruby-001.ttml` | `imsc1_1/ttml/ruby/ruby001.ttml` | `tts:ruby` (`container`/`base`/`text`) — a TTML2 feature only reachable through the IMSC 1.1 (not 1.0.1) Text Profile designator; real Japanese ruby text. |
| `imsc1.1-textemphasis-001.ttml` | `imsc1_1/ttml/textEmphasis/textEmphasis001.ttml` | `tts:textEmphasis` (`filled`/`open` × `circle`/`dot`/`sesame`), Japanese text, IMSC 1.1 designator. |
| `imsc1.1-textshadow-001.ttml` | `imsc1_1/ttml/textShadow/textShadow001.ttml` | `tts:textShadow` with **negative** offset components (`10% -20% 5% lime`) — exercises `<length>`'s signed-value path. |
| `imsc1.1-displayaspectratio-001.ttml` | `imsc1_1/ttml/displayAspectRatio/displayAspectRatio001.ttml` | `ttp:displayAspectRatio` on `tt`, IMSC 1.1 `ttp:contentProfiles` designator. |
| `imsc1-activearea-001.ttml` | `imsc1/ttml/activeArea/ActiveArea001.ttml` | `ittp:activeArea` (IMSC Extension namespace) with 3 regions, one of them outside the active area. **It does NOT exercise the §7.12.1.3 max-4-presented-regions cap (only 3 regions are present) nor the §7.12.1.2 no-overlap rule (the three y-extents — 10-20%, 80-90%, 92-98% — are disjoint).** An earlier revision of this row claimed it exercised both; that was a coverage overclaim, caught by the fidelity audit. No fixture in this set covers either constraint — see the gaps note below. |
| `imsc1-alttext-smpte-backgroundimage-001.ttml` | `imsc1/ttml/altText/altText1.ttml` | Image Profile via the older `smpte:backgroundImage` attribute mechanism (as opposed to the `image001` fixture's `<image>` element mechanism) + `ittm:altText` metadata element. Note: this fixture references an external `altText1-img.png` resource that was **not** fetched (only the `image001` pair below includes its image resource) — the `.ttml` is still useful standalone for parsing `smpte:backgroundImage`/`ittm:altText` syntax; a decoder exercising the referenced-resource path would need to fetch `imsc1/ttml/altText/altText1-img.png` separately. |
| `imsc1.1-image-profile-001.ttml` + `imsc1.1-image-profile-001-img.png` | `imsc1_1/ttml/image/image001.ttml` + `imsc1_1/ttml/image/image001-img.png` | Image Profile via the TTML2 `<image>` element (`src`/`type`/`tts:extent` all required per IMSC 1.1 §9.4.4) — a matched TTML+PNG pair, so this is the one fixture where the referenced image resource is actually present and byte-verifiable (640×120 8-bit RGBA PNG, verified with `file`). |

## What was deliberately excluded (and why)

- The bulk of the `imsc1`/`imsc1_1`/`imsc1_2`/`imsc1_3` test trees (321 TTML
  files + hundreds of reference PNGs total) — same licence, so redistribution
  would have been permitted, but committing the whole suite is unnecessary
  weight for a spec-prep task; the 11-file selection above was chosen to
  cover the syntax breadth documented in `docs/ttml2-syntax.md` and
  `docs/imsc11-profiles.md` (Text Profile, Image Profile via both
  mechanisms, animation, ruby/textEmphasis/textShadow as IMSC-1.1-only
  reachable TTML2 features, the full time-expression grammar, an IMSC
  extension attribute, and referential styling).
- `imsc1/ttml/altText/altText1-img.png` (the image resource referenced by
  `imsc1-alttext-smpte-backgroundimage-001.ttml`) — not fetched; noted above.
- DASH-IF test assets (`testassets.dashif.org`, `livesim2.dashif.org`) and
  the `Dash-Industry-Forum` GitHub org's `Test-Vectors` repo — both endpoints
  are reachable (checked), but a shallow pass (repo listing, `Test-Vectors`
  git metadata) found no obviously-indexed bare `.ttml`/IMSC XML document
  comparable to the W3C `imsc-tests` suite; DASH-IF's TTML-related test
  content is more likely muxed inside CMAF/fMP4 segments (a `transmux`
  concern, not this crate's), not exposed as standalone XML. This was not an
  exhaustive search of DASH-IF's holdings — the W3C `imsc-tests` suite above
  was already an authoritative, sufficient, and clearly-licensed source, so
  the search stopped there rather than being pursued further.
- No TTML/DFXP files pre-existed anywhere else in this repository (checked
  via `find`/`grep` across the whole workspace before fetching externally).

## Coverage gaps in this fixture set (established by the fidelity audit)

The 11 documents were checked against the spec for what they actually exercise, not what
they were assumed to. Nothing here covers:

- **The IMSC 1.1 §7.12.1.3 four-presented-region cap.** No fixture presents more than 3
  regions.
- **The §7.12.1.2 region-no-overlap rule.** Every fixture's regions have disjoint extents.
- **`itts:forcedDisplay`** and **`itts:fillLineGap`.**

These are all "must reject" structural constraints, so a validator implementing them has no
fixture to prove it bites. Either pull the relevant documents from the rest of the
`w3c/imsc-tests` suite (321 files; only 11 were taken here) or hand-construct violating
documents — and if hand-constructed, say so at the test site, because a hand-made document
only exercises the violation its author imagined.
