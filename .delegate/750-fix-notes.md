# 750-fix-notes — Fidelity audit corrections for ATSC3 spec docs

Date: 2026-07-30
Branch: `prep/750-atsc3-spec`

## Fix 1: Reference [2] was misreported as unvendored

A/321's normative reference [2] is **ATSC A/331:2026-04**, named explicitly in A/321's
bibliography §2.1. The `ea_wake_up_1`/`ea_wake_up_2` semantics are defined in A/331 Annex G.2.

Both A/331:2025-06 (already transcribed) and A/331:2026-04 (verified) contain the same Annex
G.2 content: the two bits form a 2-bit Wake-up Field, with Table G.2.1 mapping values to
emergency-alert states.

Correction: identified ref [2] correctly, transcribed the Wake-up Field table and semantics
into `a321-bootstrap.md` §7. Removed the false gap entry from README's "could not establish."

## Fix 2: A/331 revision currency

A/321:2026-06 cites A/331:2026-04. The PR transcribed A/331:2025-06.

A/331:2026-04 was fetched (`https://www.atsc.org/wp-content/uploads/2026/04/A331-2026-04-...`)
and converted via markitdown. Diffed against 2025-06 across every clause transcribed:

- Annex A ROUTE tables (field constraints, extension headers, FEC Payload IDs, Codepoint,
  SrcFlow, RepairFlow, delivery-object format IDs)
- §6 LLS/SLT tables (LLS_table() syntax, SLT XML, code-value tables)
- §7.1 ROUTE/DASH SLS (USBD, S-TSID, APD, HELD, DWD)
- §7.2 MMT (mmt_atsc3_message(), MMTP USBD)
- Annex H media-type registry
- Annex G.2 Wake-up Field

Result: **no value, bit-width, or semantic differs in any transcribed clause.** The 2026-04
revision history lists: three amendments rolled into 2025-10, one into 2026-02, and 2026-04 is
"references to ATSC documents updated" (bibliographic update).

The 2026-04 version does add a new Table 6.5 "Values for kind" (a `kind` attribute on
`SLT.Service`) that was not transcribed — but this is new content, not a change to existing
content.

Decision: keep the 2025-06 transcription; state both revisions and the verification result in
README. The wire format is identical.

## Fix 3: EXT_TOL cross-reference was wrong

`a331-route.md` §2.2 cited the "should be present" rule to "the section A.3.5.1 area."
The actual location is §A.3.8 / §A.3.8.1 (Extension Headers > EXT_TOL Header).

Corrected.

## Fix 4: Missing ROUTE-vs-FLUTE delta — File Template EXT_TOL/EXT_FTI shall

§A.3.3.2.7 (File Template) upgrades EXT_TOL/EXT_FTI presence from the general "should" of
§A.3.8.1 to "shall," with a last-packet timing requirement (the general rule is a
recommendation; File Template mode makes it a conformance requirement).

This was present in BOTH 2025-06 and 2026-04 — it was simply overlooked in the original pass.

Added as a bullet under File Mode in `a331-route.md` §5.

## Files modified

- `atsc3/docs/README.md`: updated source table, A/331 revision note, removed false gap,
  added adversarial audit section
- `atsc3/docs/a321-bootstrap.md`: fixed ref [2] citations, added §7 (Wake-up Field table
  + semantics from A/331 Annex G.2)
- `atsc3/docs/a331-route.md`: fixed EXT_TOL cross-reference (§A.3.8.1), added File Template
  EXT_TOL/EXT_FTI delta

## Audit scope

Every numeric/bit-syntax table was independently verified (see README audit section).
No fabricated values found anywhere. The three textual/structural issues listed above
were the only defects.
