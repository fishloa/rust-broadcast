# dvb-ci-runtime CAS-layer Implementation Plan (#763)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Raise `dvb-ci-runtime` to a single-slot CI-CAM CAS orchestration layer — PMT-in `ca_pmt`, CAT-driven EMM-PID feed, periodic entitlement re-query with edge-triggered events, typed `CaEnable` — additive over the existing raw API.

**Architecture:** Two additive layers. Layer 1 = sans-IO `ManagedCa` state on `Driver` (parsed dvb-si structs in; builds `ca_pmt` via the existing `dvb-ci::builder`, drives the existing multi-programme API, re-queries via `Resource::tick`, emits events through the existing `Notification` framework). Layer 2 = turnkey `CaDescrambler` owning a `CiDataDevice`. Typed structs in/out, no raw bytes.

**Tech Stack:** Rust, `dvb-ci` (ca_pmt objects + `builder::build_ca_pmt`), `dvb-si` (`PmtSection`/`CatSection`/`CaDescriptor`), the crate's `Notification`/`Driver`/`Resource::tick`/`CaDevice`/`CiDataDevice`.

**Design spec:** `docs/superpowers/specs/2026-07-24-dvb-ci-cas-layer-design.md` (read it first).

## Global Constraints
- MSRV 1.86, edition 2024; build/test `--locked`. `cargo nextest`, never two cargo cmds at once.
- Additive / non-breaking. `Notification` is `#[non_exhaustive]`. Raw `Driver::send_ca_pmt(&[u8])` + existing notifications stay.
- No raw-byte public API for the new surface — parsed `PmtSection`/`CatSection` in; typed `CaEnable`/PID slices out.
- No magic numbers outside `#[cfg(test)]` (named consts, incl. `REQUERY_DEFAULT`). #204 label convention for any new label enum (`CaEnable` already has it).
- Every new/changed public item cites its spec (EN 50221 §8.4.3.5 (ca_pmt_reply, Table 26); EN 300 468 §6.2.16 CA descriptor; ISO/IEC 13818-1 §2.4.4.5 CAT). Grounding: `dvb-ci/docs`, `dvb-si/docs`.
- Version: dvb-ci-runtime 0.13.0 → 0.14.0 (final task).
- **Preflight (before Task 1):** read `dvb-ci-runtime/src/{event.rs,driver.rs,resource.rs,dataplane.rs}`, `dvb-ci/src/builder.rs` + `objects/ca_pmt*.rs` (exact `build_ca_pmt`/`CaEnable`/`CaPmtReply`-object signatures), `dvb-si/src/tables/{pmt.rs,cat.rs}` + `descriptors/ca.rs`, and how `Notification::CaInfo` exposes the CAM CAIDs. Use those exact signatures — do not guess.

---

### Task 1: Expose typed `CaEnable` on `CaPmtReply`

**Files:** Modify `dvb-ci-runtime/src/event.rs` (`Notification::CaPmtReply`), `dvb-ci-runtime/src/resource.rs` or `driver.rs` (wherever `CaPmtReply` is emitted from the parsed `ca_pmt_reply`). Test: in-module.

**Interfaces — Produces:** `Notification::CaPmtReply { program_number: u16, ca_enable: Option<CaEnable>, descrambling_ok: bool }` (adds `ca_enable`; `dvb_ci::objects::ca_pmt_reply::CaEnable`). `None` = programme `CA_enable_flag` clear; do NOT invent a sentinel.

- [ ] Step 1: Test — feed a scripted `ca_pmt_reply` APDU whose `CA_enable` = `0x03` via `MockCaDevice`; assert the emitted `Notification::CaPmtReply.ca_enable == Some(<the 0x03 variant name from ca_pmt_reply.rs>)` and `descrambling_ok == true`. Add a second test: flag-clear reply → `ca_enable == None`, `descrambling_ok == false`. Run → FAIL (field absent).
- [ ] Step 2: Add `ca_enable: Option<CaEnable>` to the variant; at the emit site, plumb the dvb-ci `CaPmtReply` object's own `Option<CaEnable>` straight through (preserve `None`); derive `descrambling_ok` (`Some(possible*) => true`, else `false`). Cite EN 50221 §8.4.3.5 (ca_pmt_reply, Table 26).
- [ ] Step 2a: Fix the ONE exhaustive match site: `dvb-ci-runtime/src/bin/ci-probe.rs` (~L192) destructures `Notification::CaPmtReply { program_number, descrambling_ok }` with no `..` — it's inside a `#[cfg(all(feature="linux", target_os="linux"))]` module, so a macOS build silently skips it. Add the `ca_enable` field (or `..`). **Verify on the Linux target**: `cargo check -p dvb-ci-runtime --features linux --locked --target aarch64-unknown-linux-gnu --bin ci-probe` (do NOT trust the macOS build for this).
- [ ] Step 3: Run → PASS. Full gate for `-p dvb-ci-runtime`.
- [ ] Step 4: Commit `feat(dvb-ci-runtime): expose typed CaEnable on CaPmtReply notification (#763)`.

### Task 2: `Notification::Entitlement` variant (edge event, not yet emitted)

**Files:** Modify `dvb-ci-runtime/src/event.rs`. Test: in-module (construction/round-trip; emission tested in Task 5).

**Interfaces — Produces:** `Notification::Entitlement { program_number: u16, ca_enable: CaEnable, descrambling_ok: bool }`.

- [ ] Step 1: Test — construct the variant, assert its fields; if the crate has a `label_coverage`/serde round-trip test that scans `Notification`, ensure it still passes (data variant, no `name()` needed; `CaEnable` already `Display`). Run → FAIL (variant absent).
- [ ] Step 2: Add the `#[non_exhaustive]`-compatible variant with doc citing #763 + that it is edge-triggered per program by the re-query.
- [ ] Step 3: Run → PASS + `RUSTDOCFLAGS="-D warnings" cargo doc -p dvb-ci-runtime`.
- [ ] Step 4: Commit `feat(dvb-ci-runtime): add edge-triggered Notification::Entitlement variant (#763)`.

### Task 3: `ManagedCa` state + `add_service(&PmtSection)`

**Files:** Create `dvb-ci-runtime/src/managed.rs` (the `ManagedCa` owned state: active-service map `program_number → owned {es_pids, ca_pids, cmd, last_ca_enable}`, requery interval, `since`). Modify `dvb-ci-runtime/src/driver.rs` (hold a `ManagedCa`; add `pub fn add_service`), `lib.rs` (`pub mod managed;`). Test: `dvb-ci-runtime/tests/managed_ca.rs` (or in-module).

**Interfaces — Consumes:** `dvb_ci::builder::build_ca_pmt` (exact sig from preflight), the existing multi-programme send path. **Produces:** `Driver::add_service(&mut self, pmt: &dvb_si::tables::pmt::PmtSection<'_>) -> Result<(), CaError>` (new `CaError` in a new `error` arm or module); records the service in `ManagedCa`.

- [ ] Step 1: Fixture prep — obtain a real scrambled-service PMT: parse it from a committed capture (`fixtures/dvb-si/tnt-5w-12732v-isi6-10s.ts` or another with CA descriptors) via `dvb-si` in the test, OR add a small committed PMT section fixture with program + ES CA descriptors under `dvb-ci-runtime/tests/fixtures/`. Document provenance.
- [ ] Step 2: Test — `add_service(&pmt)` then assert the `ca_pmt` bytes sent to `MockCaDevice` byte-equal `dvb_ci::builder::build_ca_pmt(&pmt, first/only, ok_descrambling)`'s `to_bytes()` (oracle). Run → FAIL.
- [ ] Step 3: Implement `ManagedCa` + `add_service`: build via `build_ca_pmt`, send through the existing path, record `program_number` + ES/CA PIDs + `CaError::NoCaDescriptor{program_number}` when the PMT has no CA descriptor at program or ES level.
- [ ] Step 4: Run → PASS + gate. Commit `feat(dvb-ci-runtime): ManagedCa + add_service(&PmtSection) building ca_pmt (#763)`.

### Task 4: `set_cat(&CatSection)` + `emm_pids()` / `descramble_pids()`

**Files:** Modify `dvb-ci-runtime/src/managed.rs`, `driver.rs`. Test: `tests/managed_ca.rs`.

**Interfaces — Consumes:** `Notification::CaInfo` CAIDs (the CAM's advertised set; from preflight), `dvb_si::tables::cat::CatSection` + its CA descriptors. **Produces:** `Driver::set_cat(&mut self, cat: &CatSection<'_>) -> Result<(), CaError>`, `Driver::emm_pids(&self) -> &[u16]`, `Driver::descramble_pids(&self) -> &[u16]`.

- [ ] Step 1: Test — script `ca_info` with CAIDs `{0x0648, 0x0100}`; `set_cat` a CAT whose CA descriptors map `0x0648→PID 0x1FF0`, `0x0500→PID 0x1FF1`; assert `emm_pids() == [0x1FF0]` (0x0500 excluded — not advertised). Assert `descramble_pids()` = the ES PIDs from the added service(s). Also assert `set_cat` before any `ca_info` → `emm_pids()` empty, then recomputes once `ca_info` arrives. Run → FAIL.
- [ ] Step 2: Implement: parse the CAT's CA descriptors (CAID→emm_pid), intersect with the CAM CAIDs from the last `CaInfo`; store the EMM set; `descramble_pids` = union of active services' ES PIDs. Cite EN 300 468 §6.2.16 + ISO/IEC 13818-1 §2.4.4.5.
- [ ] Step 3: Run → PASS + gate. Commit `feat(dvb-ci-runtime): CAT-driven EMM-PID feed (CAT ∩ ca_info CAIDs) (#763)`.

### Task 5: Re-query timer + edge-triggered `Entitlement`

**Files:** Modify `dvb-ci-runtime/src/managed.rs` (requery interval + per-program `last_ca_enable`), the CA resource / driver `tick` path (`resource.rs`/`driver.rs`). Test: `tests/managed_ca.rs`.

**Interfaces — Produces:** `Driver::set_requery_interval(&mut self, interval: core::time::Duration)` (const `REQUERY_DEFAULT = Duration::from_secs(10)`; `Duration::ZERO` disables). Emits `Notification::Entitlement` on a per-program `(ca_enable/descrambling_ok)` transition.

- [ ] Step 1: Test — add a service; script the CAM to reply `CaEnable`≈not-entitled (descrambling_ok=false) first, then after advancing the clock past the interval (drive `pump`/`tick`), reply descrambling-possible (true). Assert exactly ONE `Notification::Entitlement{program_number, ca_enable, descrambling_ok:true}`. Negative control: unchanged status across re-queries → NO `Entitlement`. Run → FAIL.
- [ ] Step 2: Implement: reuse the date-time resource's `tick(elapsed)` pattern — accumulate `since`, when `≥ interval` re-send the active `ca_pmt`(s) via the existing multi-programme path; on each `CaPmtReply`, diff `ca_enable` vs stored `last_ca_enable[program_number]`; on change, push `Notification::Entitlement` + update. Respect `SetTimer`/`next_timer`.
- [ ] Step 3: Run → PASS + gate. Commit `feat(dvb-ci-runtime): periodic ca_pmt re-query + edge-triggered Entitlement events (#763)`.

### Task 6: `remove_service`

**Files:** Modify `managed.rs`, `driver.rs`. Test: `tests/managed_ca.rs`.

**Interfaces — Produces:** `Driver::remove_service(&mut self, program_number: u16) -> Result<(), CaError>`.

- [ ] Step 1: Test — add two services, `remove_service(pn1)`; assert the sent `ca_pmt` uses list-management `update`/`not_selected` for the dropped program, `descramble_pids()` no longer contains its ES PIDs, and its `last_ca_enable` entry is gone. Run → FAIL.
- [ ] Step 2: Implement via the existing `remove_program` path + state cleanup.
- [ ] Step 3: Run → PASS + gate. Commit `feat(dvb-ci-runtime): remove_service (#763)`.

### Task 7: Turnkey `CaDescrambler` over `CiDataDevice`

**Files:** Create `dvb-ci-runtime/src/descrambler.rs`; `lib.rs` (`pub mod descrambler;` + re-export). Test: `tests/descrambler.rs` using `MockCiDataDevice`.

**Interfaces — Consumes:** `Driver` (Layer 1), `CiDataDevice`. **Produces:** `pub struct CaDescrambler<D: CaDevice, C: CiDataDevice>` with `new(driver, ci_data)`, `add_service(&PmtSection)`, `set_cat(&CatSection)`, `feed_ts(&mut self, scrambled: &[u8]) -> io::Result<Vec<u8>>`, `take_notifications() -> Vec<Notification>`.

- [ ] Step 1: Test — build over `MockCaDevice`+`MockCiDataDevice`; `add_service`+`set_cat` from fixtures; `feed_ts(scrambled)` → assert the PIDs written into the mock `CiDataDevice` are `emm_pids() ∪ descramble_pids()` (packets on other PIDs dropped/passed per the documented policy) and the returned bytes are the mock's descrambled output; entitlement events surface via `take_notifications()`. Run → FAIL.
- [ ] Step 2: Implement the wrapper: delegate CA control to `Driver`; in `feed_ts`, filter/route TS packets on `emm_pids() ∪ descramble_pids()` into `CiDataDevice::write`, read descrambled out. Document the pass/drop policy for non-CA PIDs.
- [ ] Step 3: Run → PASS + workspace gate (dvb-ci bin/ci-probe consumers). Commit `feat(dvb-ci-runtime): turnkey CaDescrambler owning ci0 data plane (#763)`.

### Task 8: Docs + release prep (0.14.0)

**Files:** `dvb-ci-runtime/src/lib.rs` (`//!` — add the managed-CA section + point to the spec), `README.md`, `CHANGELOG.md` (`[Unreleased]`→`[0.14.0]`), `Cargo.toml` version 0.13.0→0.14.0. `docs/release-notes/dvb-ci-runtime-0.14.0.md`.

- [ ] Step 1: Update `//!`/README with the two-layer managed-CA API + the raw-API-still-there note; CHANGELOG entries (typed CaEnable, Entitlement, add_service/set_cat/emm_pids/descramble_pids, CaDescrambler); release note.
- [ ] Step 2: Bump Cargo.toml → 0.14.0; `cargo check -p dvb-ci-runtime` to refresh Cargo.lock.
- [ ] Step 3: Full gate: `cargo build --workspace --all-features --locked`, `--no-default-features`, `cargo nextest run -p dvb-ci-runtime --all-features --locked`, clippy `-D warnings`, fmt, doc `-D warnings`. Commit `release(dvb-ci-runtime): 0.14.0 prep — CI-CAM CAS orchestration layer (#763)`.

---

## Self-review

**Spec coverage:** PMT-in ca_pmt → T3; CAT EMM feed → T4; re-query + edge events → T5; raw CA_enable exposure → T1(+T2 event); turnkey ci0 ownership → T7; typed structs in/out → T1/T3/T4; additive + version → T8; non-goals (multi-slot/software-CAS) → not implemented (correct). All spec sections covered.

**Placeholders:** none — each task names exact files, the new signatures, and a concrete biting test; where an existing signature must be matched exactly, the Preflight + per-task "read X" pointer directs to it rather than guessing (deliberate: inventing existing-API signatures would be wrong).

**Type consistency:** `CaEnable` (dvb-ci) used identically T1/T2/T5; `Notification::Entitlement{program_number,ca_enable,descrambling_ok}` consistent T2/T5; `add_service`/`remove_service`/`set_cat`/`emm_pids`/`descramble_pids`/`set_requery_interval`/`CaDescrambler` names match the spec throughout.
