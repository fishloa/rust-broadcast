# mpeg-ts Scope-B Rework Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Rework the mpeg-ts extraction from re-export/scope-A to **clean-break + scope-B**: move the 4 generic PSI tables (PAT/PMT/CAT/TSDT) into mpeg-ts too, drop the dvb-si re-export shims, and ship as a breaking **8.0.0**.

**Architecture:** Build on commit `efb4c039` (green re-export checkpoint: framing already in mpeg-ts, dvb-si re-exporting). This rework: (1) delete the `dvb_si::{ts,section,resync,mux,pid}` re-export modules; repoint dvb-si internals + the 3 workspace consumers to `mpeg_ts::` directly; (2) move `pat/pmt/cat/tsdt` table modules into mpeg-ts with a minimal `mpeg_ts::DescriptorLoop(&[u8])` (raw bytes — mpeg-ts has zero descriptor typing); (3) rewire dvb-si's `AnyTableSection`/`declare_tables!` to source those 4 table types from `mpeg_ts`; (4) bump lockstep 8.0.0.

**Tech Stack:** dvb-common, thiserror, bytes; spec ITU-T H.222.0 (08/2023) = ISO/IEC 13818-1 (vendored + transcribed in mpeg-ts/docs/).

## Global Constraints

- MSRV 1.81; build/test `--locked`. CI `RUSTFLAGS="-D warnings"`. 6 gates + MSRV + bare-metal `thumbv7em-none-eabi` (mpeg-ts + dvb-si).
- `#![no_std]` + alloc; `#![forbid(unsafe_code)]`; serde feature-gated.
- **CLEAN BREAK:** after this, `dvb_si::{ts,section,resync,mux,pid}` and `dvb_si::tables::{pat,pmt,cat,tsdt}` do NOT exist. No re-export shims. Consumers use `mpeg_ts::…`.
- **mpeg-ts has NO descriptor typing.** Generic tables hold `mpeg_ts::DescriptorLoop(&'a [u8])` (raw, serialize-verbatim). Typed descriptor parsing stays entirely in dvb-si (`parse_loop`/`AnyDescriptor`). Descriptors stay in dvb-si **permanently** (decided).
- Round-trip invariant travels with all moved code (parse↔serialize byte-identical tests).
- `AnyTableSection` stays in dvb-si; its `declare_tables!` references `mpeg_ts::{Pat,Pmt,Cat,Tsdt}Section` for table_ids 0x00–0x03; drift test pins tag→`TABLE_ID` across crates.
- Lockstep **8.0.0** (all 6: dvb-common, dvb-si, dvb-t2mi, dvb-bbframe, dvb-conformance, dvb-tools) + dvb-tools clap CLI (#344). mpeg-ts independent 0.1.0.
- **DO NOT PUBLISH.** Build → gate → audit → PR → STOP for owner code review.

---

### Task R1: Clean-break the framing (drop dvb-si re-exports → mpeg_ts direct)

**Files:** `dvb-si/src/lib.rs` (remove the 5 re-export modules), `dvb-si/src/{demux.rs,collect/*.rs,tot.rs}` (use `mpeg_ts::` directly), `dvb-si/src/error.rs` (keep `From<mpeg_ts::Error>` — SiDemux still needs it), `dvb-tools/src/{util.rs,pids.rs}`, `dvb-stream/src/resync.rs`, `dvb-conformance/src/lib.rs`, `demo/src/lib.rs`, their `Cargo.toml`s (add `mpeg-ts` dep).

- [ ] **Step 1: Remove the re-export modules** from `dvb-si/src/lib.rs` (the `pub mod ts { pub use mpeg_ts::ts::*; }` blocks for ts/section/resync/mux/pid). dvb-si no longer re-exports them.
- [ ] **Step 2: Repoint dvb-si internals** — in `demux.rs`/`collect/*.rs`/`tot.rs`, change `use crate::ts::…`/`crate::section::Section`/`crate::pid::…`/`crate::resync::…`/`crate::mux::…` → `use mpeg_ts::ts::…` etc. (`grep -rn 'crate::\(ts\|section\|resync\|mux\|pid\)::' dvb-si/src`). Keep `dvb-si`'s `mpeg-ts` dep + `From<mpeg_ts::Error>`.
- [ ] **Step 3: Repoint the workspace consumers** — dvb-tools/dvb-stream/dvb-conformance/demo: every `dvb_si::{ts,section,resync,mux,pid}::X` → `mpeg_ts::…::X`; add `mpeg-ts = { path="../mpeg-ts", version="0.1", default-features=false }` (+ std passthrough) to each Cargo.toml that needs it (`grep -rn 'dvb_si::\(ts\|section\|resync\|mux\|pid\)::' --include=*.rs . | grep -v dvb-si/`).
- [ ] **Step 4: Build the whole workspace** — `cargo build --workspace --all-features --locked` PASS; `cargo test --workspace --all-features --locked` PASS (the fixture/SiDemux tests prove the direct-`mpeg_ts` wiring works). Confirm NO `dvb_si::ts`/`crate::ts` references remain (`grep -rn 'dvb_si::ts\b\|dvb_si::section\b\|crate::ts::\|crate::section::' --include=*.rs . | grep -v mpeg-ts/`).
- [ ] **Step 5: fmt + commit** — `git commit -m "refactor: clean break — drop dvb_si TS re-exports, consumers use mpeg_ts directly"`

---

### Task R2: Move the 4 generic PSI tables into mpeg-ts (raw descriptor bytes)

**Files:** `git mv dvb-si/src/tables/{pat,pmt,cat,tsdt}.rs mpeg-ts/src/tables/`; Create `mpeg-ts/src/tables/mod.rs`, `mpeg-ts/src/descriptor_loop.rs` (the raw `DescriptorLoop`); Modify `mpeg-ts/src/lib.rs`.

- [ ] **Step 1: Minimal `mpeg_ts::DescriptorLoop`** — create `mpeg-ts/src/descriptor_loop.rs`: `pub struct DescriptorLoop<'a>(pub &'a [u8]);` with `From<&[u8]>`, `Deref<Target=[u8]>`, serialize-verbatim (write the bytes), serde-as-bytes, `Debug`. NO typed iteration (that's dvb-si). Export at `mpeg_ts::DescriptorLoop`.
- [ ] **Step 2: `git mv` the 4 table files** into `mpeg-ts/src/tables/`; create `mpeg-ts/src/tables/mod.rs` declaring `pub mod {pat,cat,pmt,tsdt};` + re-export the section types. Wire `pub mod tables;` in mpeg-ts/src/lib.rs.
- [ ] **Step 3: Repoint the moved tables** — in the 4 files, `crate::` rebinds to mpeg_ts (error/ts/pid resolve). Replace the dvb-si `DescriptorLoop` import (`use crate::descriptors::any::DescriptorLoop` or similar) with `crate::DescriptorLoop` (mpeg-ts's raw one). PMT's `program_info`/`es_info` + CAT/TSDT loops now type as `mpeg_ts::DescriptorLoop` (raw bytes). Verify no `crate::descriptors::` (typed) refs remain: `grep -nE 'crate::descriptors|AnyDescriptor|parse_loop' mpeg-ts/src/tables/*.rs` → must be NONE (STOP-gate; if a table needs typed descriptors, the design is wrong — report).
- [ ] **Step 4: Build + test mpeg-ts standalone** — `cargo test -p mpeg-ts --all-features --locked` PASS (moved table round-trip tests travel with the files); `cargo build -p mpeg-ts --no-default-features --locked` PASS. Workspace will be broken (dvb-si still references the moved tables) — expected, fixed in R3. Build ONLY `-p mpeg-ts`.
- [ ] **Step 5: commit** — `git commit -m "refactor(mpeg-ts): move PAT/PMT/CAT/TSDT generic PSI tables from dvb-si (raw descriptor bytes); dvb-si dispatch rewired next"`

---

### Task R3: Rewire dvb-si AnyTableSection/declare_tables to source generic tables from mpeg-ts

**Files:** `dvb-si/src/tables/any.rs` (declare_tables! + AnyTableSection + drift test), `dvb-si/src/tables/mod.rs` (drop the 4 moved mod decls; re-export or reference mpeg_ts types), `dvb-si/src/{collect,demux}` (PMT/PAT users → mpeg_ts types), any dvb-si code using `crate::tables::{pat,pmt,cat,tsdt}`.

- [ ] **Step 1: Drop the 4 moved table modules** from `dvb-si/src/tables/mod.rs`. dvb-si no longer defines pat/pmt/cat/tsdt.
- [ ] **Step 2: Rewire `declare_tables!`** in `tables/any.rs` — the 4 generic entries (PROGRAM_ASSOCIATION/PROGRAM_MAP/CA/TS_DESCRIPTION) now list `mpeg_ts::tables::pat::PatSection` etc. as their type path. `AnyTableSection` variants for 0x00–0x03 wrap the mpeg-ts types. The drift test must read `TABLE_ID` from the mpeg-ts types (ensure mpeg-ts's table types expose the same `TABLE_ID`/`TableDef` const the macro/test expect — if `TableDef` trait lives in dvb-si, either move the relevant const exposure or have the macro read mpeg_ts types' inherent `TABLE_ID`).
- [ ] **Step 3: Repoint dvb-si PMT/PAT consumers** — `collect/`, `demux.rs`, anything using `crate::tables::pat::PatSection`/`pmt::PmtSection` → `mpeg_ts::tables::…`. Where dvb-si code walked PMT descriptors via the old typed `DescriptorLoop`, it now calls `dvb_si::descriptors::parse_loop(pmt.es_info.0)` on the raw bytes (mpeg_ts::DescriptorLoop derefs to `&[u8]`).
- [ ] **Step 4: Build + test dvb-si + workspace** — `cargo build --workspace --all-features --locked`, `cargo test --workspace --all-features --locked` PASS. The `AnyTableSection` dispatch + drift test + SiDemux fixture tests (PAT→PMT discovery over real .ts) are the integration proof.
- [ ] **Step 5: commit** — `git commit -m "refactor(dvb-si): AnyTableSection sources PAT/PMT/CAT/TSDT from mpeg-ts; PMT descriptors via parse_loop on raw bytes"`

---

### Task R4: Lockstep 8.0.0 + mpeg-ts docs/release-infra

**Files:** 6 lockstep `Cargo.toml` (8.0.0) + CHANGELOGs; `mpeg-ts/{README.md,CHANGELOG.md,examples/}`; `docs/release-notes/{v8.0.0.md,mpeg-ts-v0.1.0.md}`; `.github/workflows/release-mpeg-ts.yml`; `release.yml` (ordering comment); `CLAUDE.md` + `README.md` inventory.

- [ ] **Step 1: Bump all 6 lockstep crates** 7.9.0/7.10.0 → **8.0.0**; CHANGELOG `[8.0.0]` entries. dvb-si entry: "BREAKING — TS framing + PAT/PMT/CAT/TSDT moved to `mpeg-ts`; use `mpeg_ts::…`; descriptors stay in dvb-si." dvb-tools entry: clap CLI (#344).
- [ ] **Step 2: mpeg-ts README + CHANGELOG (0.1.0) + example** per docs/RELEASE-DOCS.md (demux a .ts → SectionReassembler → PAT/PMT; cite H.222.0). docs.rs metadata present.
- [ ] **Step 3: release-mpeg-ts.yml** (model on release-mpeg-pes.yml; tag `mpeg-ts-v*`; gates test/clippy/no_std-thumbv7em/tag-match; publish `-p mpeg-ts`).
- [ ] **Step 4: release-notes** `v8.0.0.md` (lockstep + the mpeg-ts extraction + migration note) + `mpeg-ts-v0.1.0.md`. CLAUDE.md/README crate inventory: add mpeg-ts; note TS/PSI-tables now live there.
- [ ] **Step 5: commit** — `git commit -m "docs+release: lockstep v8.0.0 + mpeg-ts 0.1.0 (README/CHANGELOG/example/workflow)"`

---

### Task R5: Full gate + audit prep (controller-run)

- [ ] **Step 1: Run all gates** (workspace whole): fmt, build all-features, build no-default (excl dvb-tools/conformance/stream), clippy `-D warnings`, test, doc `-D warnings`, MSRV 1.81, bare-metal thumbv7em (mpeg-ts + dvb-si). ALL PASS. Add `mpeg-ts/tests/label_coverage.rs` (cover public enums; SKIP Error/Any*).
- [ ] **Step 2: commit** any fmt/label fixes.
- [ ] **Step 3: (controller) final whole-branch review + RELEASE-AUDIT battery**, then STOP — owner reviews code before any tag.

---

## Self-Review

- **Spec coverage (revised design):** clean break (R1), move 4 tables + raw DescriptorLoop (R2), AnyTableSection cross-crate rewire (R3), 8.0.0 + docs/infra (R4), gate+audit (R5). descriptors-stay-dvb-si enforced by R2 STOP-gate (no `crate::descriptors` in moved tables). Covered.
- **Key risks flagged:** R3 declare_tables cross-crate `TABLE_ID`/`TableDef` exposure (the hard part) — if mpeg-ts table types can't expose what the drift test reads, R3 reports for a design tweak. R2 STOP-gate catches any typed-descriptor dependency that would contradict "raw bytes."
- **No publish.** R5 ends at owner code review.
- **Builds on** the green checkpoint `efb4c039`.

> **UPDATE 2026-06-27: Scope A chosen — Tasks R2 and R3 (table move) are DROPPED. Only R1 (clean-break) + R4 (8.0.0 docs) + R5 (gate) apply.**
