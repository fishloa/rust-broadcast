---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Annex B (normative) - MPD schema"
pdf_pages: "242"
printed_pages: "234"
kind: implementer notes in original wording (not a transcription)
verbatim_source: private/specs-verified/dash/23009-1/annex-b-schema.md
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION-C1.md
---

# Annex B - MPD schema (implementer notes)

## What the annex contains (p. 234)

Annex B is a single page. It makes the MPD XML Schema normative but does **not** embed the schema text: it only names a download location,
`https://standards.iso.org/iso-iec/23009/-1/ed-5/en`, for the file `DASH-MPD.xsd`. The Ed.5 PDF therefore carries **no schema text**; the
conformance rule in 8.1 (an MPD must be valid against the Annex B schema) refers to that external file.

## How it relates to the XSDs in the repository

- The Ed.6 schema bundle is in the private submodule at `private/specs/iso_iec_23009-1_dash_2026_ed6_schema/`. Files: `DASH-MPD.xsd` (main
  schema), `DASH-MPD-UP.xsd` and `DASH-MPD-PATCH.xsd` (companions), `DASH.abnf`, `LICENSE`, `README.md`, and the spec example documents
  `example_G*.mpd` / `example_H*.mpd` (these correspond to Annex G / Annex H examples).
- Per project rule the XSD is **not** duplicated here. Treat it as the machine-readable authority for element/attribute names, types and
  cardinality. How Ed.6 differs from the Ed.5 clause text is recorded by the sibling PR in `ED6-SCHEMA-DIFF.md` (PR #1516).
- An Ed.5-exact XSD was not fetched in this task; if a validator must match Ed.5 precisely, download `ed-5/en/DASH-MPD.xsd` from the URL above
  into the private tree and diff it against the Ed.6 file first (gap recorded in VERIFICATION-C1.md).

## Implementer take-aways

- Validate parsed MPDs structurally against the XSD for sanity, but keep the semantic rules (clauses 5 and 8) in code: the XSD cannot express
  the profile-specific "may be ignored" filtering of 8.1.
- Extension namespaces are permitted by the schema; the profile-specific MPD derivation in 8.1 step 4 drops those not admitted by the profile
  (see [08-profiles.md](08-profiles.md)).
