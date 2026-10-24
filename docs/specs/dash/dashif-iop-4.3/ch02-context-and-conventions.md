---
source: private/specs/dashif_iop_v4.3_guidelines.pdf
clauses: 2 Context and Conventions (2.1-2.4.3)
pdf_pages: 26-33
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch02-context-and-conventions.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# DASH-IF IOP v4.3 — 2 Context and conventions (notes)

PDF p.26 = printed p.5. Verbatim: private file above.

## 2.1 Relation to MPEG-DASH (p.26)

- Base spec is ISO/IEC 23009-1 second edition (2014) including Cor.3 and Amd.3 [4]; v1/v2 of this document were built on the first edition. Draft Amd.4 and Cor.3 are named as relevant.
- Backward compatibility across MPEG editions is MPEG's business, not this document's.
- Written in coordination with DVB-DASH [42]; differences are highlighted where practical, but the coverage is explicitly not claimed to be complete (see the ETSI TS 103 285 notes).

## 2.2 Compatibility and version history (pp.26-30)

Lists of changes per version (3, 3.1, 3.2, 3.3, 4.0, 4.1, 4.2, 4.3). Client-relevant items, with the clause that now holds them:

| Version | Item | Clause |
|---|---|---|
| 3 | time-code wrap-around forbidden for DASH-264/AVC; trick-mode constraints; Adaptation Set constraints incl. bitstream switching; HTTP details; CEA-608/708 in SEI; simple vs main live (main = segment parsing) | 3.2, 3.2.9, 3.2.10, 3.4, 6.4.3, 4 |
| 3.1 | transforming proxies; live-edge operation; availabilityTimeOffset; on-request xlink excluded for dynamic MPDs | 3.4.4, 4.3.6, 4.3, 5 |
| 3.2 | timing-model clarifications; switching across Adaptation Sets; HTTPS; key rotation | 3.2.7, 3.8, 7.2, 7 |
| 3.3 | identifiers table; live-to-on-demand conversion; content type via mime type/codecs/Role rather than `@contentType`; sideloaded subtitle timing; period continuity, callback event, MPD anchors aligned to Amd.3 | 1, 4.6.2, 3.2.13, 6.4.4, 3.2.12 |
| 4.0 | `Period@start` recommendation for multi-period; period connectivity; early-terminated Period; time-sync clarifications; trick modes for live | 4.3.3.3.1, 3.2.12, 4.8.3, 4.7, 4.10 |
| 4.1 | robust live services; last-segment-number signalling; seek preview/thumbnails; annotation and client selection model; remote referencing and query parameters for ad insertion; VP9 | 4.11, 4.4.3.5-6, 3.2.9/6.2.6, 3.9, 5.3.2.1.3/5.3.5, 11 |
| 4.2 | HTTPS text (RFC 5246); initialization-range clarification; permitted MPD updates; reference resolution; live-to-on-demand with the same MPD URL; SegmentTemplate substitution | 7.2, 3.2.1, 4.4.3.3, 3.2.15, 4.6.4, 4.3.2.2.8 |
| 4.3 | clause 3 renamed; default parameters for ISO BMFF; XML duration restrictions; MPD location/reference resolution; on-demand services incl. mixed; availability-time-offset processing model; "change lead time"; bitstream-switching and segmented-subtitle restrictions; USAC; HDR dynamic metadata | 3.2.1, 3.2.7.4, 3.2.15, 3.10, 4.11.4.1, 6.2.5.2, 6.4.5, 9.2.6, 10 |

Backward compatibility with v2 (2.2.2): content can serve v2 and v3 clients if authored in the restricted way, accepting that v2 clients may ignore some Representations/Adaptation Sets, or by publishing two MPDs over the same segments. New restrictions vs v2: no time-code wrap, DRM/`ContentProtection` rules, trick-mode and HTTP constraints, Adaptation Set constraints. Relaxations: extra CEA-608/708 subtitling, relaxed HE-AACv2 encoding, `lmsg` brand optional for last-segment signalling, bitstream switching may be true, remote elements with xlink allowed.

## 2.3 Key words (pp.30-31)

- Follows RFC 2119 [44] semantics for SHALL / SHALL NOT / SHOULD / SHOULD NOT / MAY, intended to be used consistently but only in lower case in this document.
- For content-authoring statements: SHALL/SHALL NOT are checked by the conformance software as errors; SHOULD/SHOULD NOT as warnings; SHOULD/MAY only documented as features.
- For client statements: SHALL means test content exists and the reference client implements it; SHALL NOT means the reference client does not; SHOULD means test content exists and the reference client implements it unless justified; SHOULD NOT the opposite; MAY means test content exists and the reference client implements it only if justified.

## 2.4 Interoperability points and profiles (pp.31-33)

- ISO/IEC 23009-1 clause 8 profiles may be defined by third parties; this document recommends calling such definitions interoperability points rather than profiles; an IOP is signalled in `@profiles` once it has a URI whose owner defines the semantics. The document can be read as content conformance points and as client capability points.
- Profile-specific MPD derivation for a profile ProfA listed in `MPD@profiles` (2.4.2), in order: (1) `@profiles` reduced to ProfA only; (2) drop AdaptationSets whose (inferred) `@profiles` lacks ProfA; (3) same for Representations; (4) drop elements/attributes ProfA excludes, or extension-namespace ones it does not include; (5) drop those ProfA says "may be ignored".
- An MPD conforms to ProfA if ProfA is listed in `@profiles`, the derived MPD is valid per ISO/IEC 23009-1 and respects ProfA's restrictions. A Media Presentation conforms if its MPD does, every Period of the derived MPD has at least one Representation, and the Segments obey ProfA.
- 2.4.3: IOPs give a basic full A/V experience; Extensions improve one dimension. Examples of `@profiles` signalling: Annex A.
