---
source: DASH-IF Interoperability Points v4.3 (15 Nov 2018), private/specs/dashif_iop_v4.3_guidelines.pdf
clauses: "Annex A Examples for Profile Signalling"
pdf_pages: "229"
printed_pages: "annex p.1"
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/annex-a-profile-signalling.md
verified_against_render: true
notes_original_wording: true
---

# IOP v4.3 Annex A - Profile signalling and pruning (notes)

Own-words notes; the exact example lists are in the private file. "p." = PDF page.

Mechanism shown: `MPD@profiles` lists every profile the document claims; each AdaptationSet may list its own `@profiles`. To check one IOP, prune the MPD to the elements that claim it (and the MPD-level list reduces to that profile); the pruned MPD must then conform to that IOP.

| Example | MPD claims | AdaptationSets | Pruned for `http://dashif.org/guidelines/dash264` |
|---|---|---|---|
| 1 | `urn:mpeg:dash:profile:isoff-on-demand:2011` and `http://dashif.org/guidelines/dash264` | (a) both ids; (b) dash264 only; (c) isoff-on-demand only (non-conforming set) | sets (a) and (b) remain, each listing only dash264 |
| 2 | the two above plus virtual IOP `http://dashif.org/guidelines/dashif#extension-example` | id 1: isoff-on-demand + dash264; id 2: dash264; id 3: isoff-on-demand + extension-example | sets 1 and 2 remain; pruning for the extension id leaves only set 3, which must conform to the example extension set |

Printed oddity: the last identifier is printed `...dashif# extension-example` with a space in one list (p.229). The annex footer reads "v4.1".
