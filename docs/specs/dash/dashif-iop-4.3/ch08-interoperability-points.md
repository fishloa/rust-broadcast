---
source: DASH-IF Interoperability Points v4.3 (15 Nov 2018), private/specs/dashif_iop_v4.3_guidelines.pdf
clauses: "8 DASH-IF Interoperability Points (8.1-8.5)"
pdf_pages: "197-200"
printed_pages: "176-179"
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch08-interoperability-points.md
verified_against_render: true
notes_original_wording: true
---

# IOP v4.3 clause 8 - Interoperability point definitions (notes)

Own-words notes; exact wording in the private file. A client claims an IOP by supporting at least the listed features; content may only claim an IOP if such a client plays it. Authors are encouraged to use the HTTP-URL construction of the DASH-IF implementation guidelines [6] 5.1.4. "p." = PDF page.

| IOP | `@profiles` identifier | Client must support | Cite |
|---|---|---|---|
| DASH-AVC/264 main | `http://dashif.org/guidelines/dash264main` | all DASH features of clause 3; simple live operation (4.9.2); server-based ad insertion (5.6.1); AVC Progressive High L4.0 (6.2); HE-AAC v2 level 2 (DRC not expected); subtitles/CC: SMPTE-TT (on-demand: single-file download is enough; live or key rotation: ISO BMFF needed) and CEA-608/708 (6.4.3); CENC with key rotation, MPD-based and movie-box-based parsing of DRM parameters (7) | 8.2, pp.197-198 |
| DASH-AVC/264 high | `http://dashif.org/guidelines/dash264high` | everything in main plus main live operation (4.9.3); content authoring also follows app-based ad insertion (5.6.2) | 8.3, p.198 |
| DASH-IF IOP simple | `http://dashif.org/guidelines/dash-if-simple` | as main, plus HEVC Main Tier Main L4.1 (8-bit HD up to 1080p) | 8.4, pp.198-199 |
| DASH-IF IOP main | `http://dashif.org/guidelines/dash-if-main` | everything in AVC/264 high (8.3), HEVC Main L4.1 and HEVC Main10 L4.1; adds real-time segment parsing and 10-bit HEVC | 8.5, pp.199-200 |

Extra rule for IOP main (p.200): content that is also AVC/264-high conformant must additionally list the AVC/264-high identifier; without it the content may be treated as HEVC-only.
