---
source: private/specs/dashif_iop_v4.3_guidelines.pdf
clauses: 3 Usage of MPEG-DASH, 3.1 Scope
pdf_pages: 33
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch03-1-scope.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# DASH-IF IOP v4.3 — 3.1 Scope (notes)

PDF p.33 = printed p.12. Verbatim: private file above.

- The IOPs use ISO BMFF encapsulation and form a superset of the ISO BMFF On-Demand, Live and Common profiles of ISO/IEC 23009-1 (clauses 8.3, 8.4, 8.10).
- Each Representation is offered either as one Segment with aligned Subsegments across the Adaptation Set (mainly on-demand) or as a sequence of Segments addressed by a templated URL (live/dynamic).
- In both cases every (Sub)Segment starts with a SAP of type 1 or 2 (IDR for video), and (Sub)Segment boundaries inside an Adaptation Set are aligned without gaps/overlap, so a client can play the "come-from" Representation and then switch to the "go-to" Representation with no overlap in download/decode/presentation (seamless switching). The printed words are "go-" / "to" split at a line end; the hyphen is a judgement.
- Extra constraints exist for bitstream switching = true and special cases such as trick modes.
