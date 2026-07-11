# Spec-fidelity audit ledger

Record of adversarial **PDF→md fidelity audits** already performed, so a later
audit run does **not** re-verify doc sets that were checked clean against an
unchanged source. Each row = one independent auditor pass (RFC ASCII / PDF render
as the oracle, full field coverage).

**How to use before a new audit run:**
1. `git log --oneline -- <doc path>` — if the doc set is unchanged since the
   "audited at commit" below, it is already verified → **skip it**.
2. Only re-audit a doc set whose files changed after its ledger entry, or a doc
   set not listed here at all.
3. After any new audit, **append a row** (doc set · source · checks · verdict ·
   nits · commit) and bump the totals line.

Conventions: "checks" = distinct fields/tables/figures verified. "substantive
discrepancy" = a wrong wire value/width/order (parser-affecting). "nit" =
citation/wording/cosmetic, no parser impact. All nits below were **fixed** in the
cited commit.

---

## Session 2026-06-21 — new-crate + extension doc sets

| Doc set | Source spec(s) | Oracle | Checks | Verdict | Nits (all fixed) | Audited at commit |
|---|---|---|---|---|---|---|
| `st291/docs/` (st_2038.md, anc_packet_291.md) | SMPTE ST 2038:2021; RFC 8331 (ANC) | PDF render + RFC ASCII | 34 | ✅ 0 substantive | — | `7963ed32` |
| `dvb-vbi/docs/vbi.md` + `dvb-emsg/docs/emsg.md` | ETSI EN 301 775; DASH-IF IOP Part 10 | PDF render | 44 | ✅ 0 substantive | — | `7963ed32` |
| `dvb-simulcrypt/docs/` (message-framing, ecmg-scs, emmg-pdg-mux, parameter-types) | ETSI TS 103 197 V1.5.1 | PDF render | 30 | ✅ 0 substantive | — | `7963ed32` |
| `dvb-ule/docs/` (sndu, ts-mapping, ext-headers) | RFC 4326; RFC 5163 | RFC ASCII | 37 | ✅ 0 substantive | — | `7963ed32` |
| `dvb-scte35/docs/dvb_ta/` (das-descriptor, scte35-profiling, …) | ETSI TS 103 752-1 V1.2.1 | PDF render (pp.17,26-28) | 16 | ✅ 0 substantive | das-descriptor.md: `equivalent_segmentation_type` mnemonic `uimsbf`→`bslbf` per Table 1 (spec quirk); note de-swapped | `2916d94f` |
| `dvb-flute/docs/` (lct, alc, flute, norm) | RFC 5651/5775/6726/5740 | RFC ASCII | 38 | ✅ 37 PASS, 1 citation | norm.md: NORM Header-Ext-Types IANA registry §8.5→§8.1.1 (values were correct) | `2916d94f` |
| `dvb-cc/docs/decode/` (cea708-decode, cea608-decode, cea708-conformance) | ANSI/CTA-708-E S-2023; CTA-608-E S-2019; 47 CFR §79.102 | PDF render + CFR | 56 | ✅ 53 PASS, 2 wording | conformance.md ¶(i) minimum window/pen style 1 not 1–7; ¶(p) edge-type names from CEA-708-E not the CFR text | `2916d94f` |

**Session totals: 255 checks across 8 doc sets · 0 substantive wire discrepancies · 4 nits found, all fixed.**

Notes:
- Both CTA-708 worked examples reproduce byte-for-byte: DefineWindow
  `9A 38 4A D1 8B 0F 11`, SWA `97 64 53 88 22` → border type 5.
- dvb-ta Table 8 mis-registration (values drift down ~1 row in the PDF render)
  was independently re-confirmed via pdftotext line-geometry; the md's
  reconstructed widths (unique_program_id 16, avail_num/avails_expected 8,
  DAS_descriptor_flag 1, equivalent_segmentation_type 4, E_CRC_32 32) are correct.

---

## Session 2026-07-11 — RFC 8331 ANC-over-RTP transport (issue #648)

| Doc set | Source spec(s) | Oracle | Checks | Verdict | Nits (all fixed) | Audited at commit |
|---|---|---|---|---|---|---|
| `st291/docs/anc_rtp_8331.md` | RFC 8331 §2/§2.1/§3.1/§4 | RFC ASCII (`specs/rfc8331_anc_rtp.txt`) | 13 (timestamp progressive/interlaced semantics, marker bit, Extended Sequence Number + RFC 4175 scope note, Length, ANC_Count, F's 4 values incl. 0b01 invalid, reserved, media type, subtype, clock rate, SDP rtpmap example) | ✅ 0 substantive | — | `401dcec1` |

Scope: only the RTP-transport-specific material curated here — the
per-ANC-packet fields + parity/checksum math were already curated (and
audited clean, 0 substantive discrepancies) in `anc_packet_291.md` per the
2026-06-21 session above and are reused unchanged, not re-audited.

---

## Extensibility / code-quality audits

Distinct from the PDF→md fidelity audits above: these are adversarial **code**
audits per `docs/RELEASE-AUDIT.md` §3 (round-trip symmetry / no `self.raw`
passthrough, no raw-byte public API, decode-completeness, spec-fidelity / no
magic numbers, the #204 `name()`+`impl_spec_display!` label convention + per-crate
`label_coverage` drift-guard, `#[non_exhaustive]`, panic-class safety,
`declare_*` dispatch). One adversarial auditor per crate/module.

### Session 2026-06-21 — new crates + new modules (audited at `f715ee07`, fixed in #313–#321)

8 targets audited. Found **6 real bugs** + a systematic missing-`label_coverage`
gap on every new crate, all fixed:

| Target | Key findings (all fixed) | Fix PR |
|---|---|---|
| dvb-scte35 `dvb_ta` | 🔴 `DvbDasDescriptor::serialize_into` u8 `descriptor_length` overflow (upid>248) → silent corruption; + TOML drift for `EquivalentSegmentationType`/`TimelineType` | #318 |
| dvb-ule | 🔴 reassembly missing RFC §7.2.1 PP check (corrupt partial); 🔴 `Sndu.type_field` ignored by serialize; +non_exhaustive, label_coverage, typed H-Type, magic-number consts | #321 |
| dvb-vbi | 🔴 `MonochromeDataField` accepted `n_pixels=0`; +label_coverage, 5 mutation bites, named masks, dispatch drift test | #314 |
| dvb-smpte2038 | 🔴 serialize silently zero-filled short `user_data_words`; +label_coverage, fixed-bit checks, more mutation bites | #319 |
| dvb-cc decode | 🔴 non-biting CC1 doctest (field2 routed CC3); +typed `Cea608Color`/`EdgeType`/`AnchorPoint`, label_coverage (was absent) | #317 |
| dvb-emsg | `EmsgVersion`/`PresentationTime` missing `#[non_exhaustive]`; `PresentationTime` missing `name()`; label_coverage; example panic | #313 |
| dvb-flute | NORM_INFO struct added (was unimplemented vs doc claim); label_coverage; named flag consts; `h_flag` `||`→`&&` | #315 |
| dvb-simulcrypt | label_coverage; exhaustive error-status drift tables; example index guard; serde smoke test | #316 |
| dvb-si `association_tag` | clean except missing `USE_DVB_RESERVED_DEFAULT` const | #320 |

Systematic gap: **7 crates lacked `tests/label_coverage.rs`** (enums had
name()+Display but no drift-guard) — now all have one. Every correctness bug got a
regression test **verified by revert** (fails pre-fix, passes post-fix). Re-audit a
crate only if its `src/` changed after the fix PR above.

---

## Pre-2026-06-21 (prior sessions)

The core dvb-si / dvb-t2mi / dvb-bbframe doc sets and ~48 enum↔TOML drift-guards
were audited under the **#158 doc-excellence** arc (Phase A + B + adversarial
PDF-fidelity pass), shipped in **v6.4.0**. Those are CI-guarded by the per-crate
`*_drift` / `label_coverage` tests and the `convention_audit` gate, so they are
continuously re-verified by CI rather than re-audited by hand. Treat them as
covered unless a drift-guard fails.
