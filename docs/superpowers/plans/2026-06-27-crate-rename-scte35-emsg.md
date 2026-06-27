# Crate Rename: scte35-splice + mp4-emsg (with shims) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the two mis-prefixed crates — `dvb-scte35`→`scte35-splice`, `dvb-emsg`→`mp4-emsg` — preserving git history, with non-yanked re-export shims on the old names, so downstream builds keep working and `timed-metadata` (Plan B) can depend on the correct names from day one.

**Architecture:** `git mv` the source dirs to new names (history preserved); bump package name + version; recreate the old dirs as thin shim crates (`pub use <new>::*` + `#[deprecated]`); update workspace members + fuzz refs; full gate; release. Both renamed crates become independently versioned (like `scte104`); `scte35-splice` thereby leaves the 7-crate DVB lockstep.

**Tech Stack:** Rust workspace, Cargo, `cargo-nextest`/`cargo test`, clippy, rustfmt, rustdoc. crates.io publish is CI-only and owner-gated.

## Global Constraints

- MSRV **1.81** (`workspace.rust-version`); always build/test `--locked`.
- CI runs with `RUSTFLAGS="-D warnings"`; the 6 gates must pass: build (all-features + no-default), test, clippy `-D warnings`, `cargo fmt --all --check`, `RUSTDOCFLAGS="-D warnings" cargo doc`.
- No magic numbers outside `#[cfg(test)]`; every public spec/field enum keeps `name()` + `impl_spec_display!` (label_coverage drift-guard).
- Aligned tables keep `#[rustfmt::skip]`; Cargo.toml manual column alignment preserved.
- **Never `cargo publish` from a workstation.** Releases are tag-driven, CI-only, and require explicit owner sign-off per release.
- Shims are **not yanked** — existing `dvb-scte35`/`dvb-emsg` Cargo.toml references must keep resolving.
- Version map (locked): `scte35-splice = 1.0.0`, `mp4-emsg = 0.1.0`, shim `dvb-scte35 = 7.9.1`, shim `dvb-emsg = 0.1.1`.

---

### Task 1: Create `scte35-splice` from `dvb-scte35` (history-preserving move)

**Files:**
- Move: `dvb-scte35/` → `scte35-splice/` (via `git mv`)
- Modify: `scte35-splice/Cargo.toml` (package name + version + leave lockstep)
- Modify: `Cargo.toml` (workspace `members`: replace `"dvb-scte35"` with `"scte35-splice"` for now; the shim is re-added in Task 3)

**Interfaces:**
- Produces: crate `scte35-splice` v1.0.0, same public API as `dvb-scte35` 7.9.0 (`SpliceInfoSection`, `ClearPayload`, `commands`, `descriptors`, `time`, `traits`, `error`). Crate lib name becomes `scte35_splice`.

- [ ] **Step 1: Move the directory preserving history**

```bash
git mv dvb-scte35 scte35-splice
git mv scte35-splice/src scte35-splice/src   # no-op; dir already moved
```

- [ ] **Step 2: Rename the package + set independent version**

Edit `scte35-splice/Cargo.toml`:
```toml
[package]
name         = "scte35-splice"
version      = "1.0.0"
description  = "ANSI/SCTE 35 splice information (DPI cueing) — parser + serializer, no_std."
# keep authors/edition/license/repository/rust-version from workspace as before
```
(Update the `description` to drop any "DVB" framing. Keep all other fields.)

- [ ] **Step 3: Update workspace members**

Edit root `Cargo.toml` `members`: replace `"dvb-scte35"` with `"scte35-splice"`.

- [ ] **Step 4: Update the crate-root doc + any internal self-references**

In `scte35-splice/src/lib.rs`, update the `//!` crate doc: keep the SCTE 35 spec citation, drop "DVB" framing if present. Search for any internal references to the old name:
```bash
grep -rn "dvb_scte35\|dvb-scte35" scte35-splice/
```
Fix any (doc links, `crate` paths are unaffected; only external-style `dvb_scte35::` refs in docs/tests need updating to `scte35_splice::`).

- [ ] **Step 5: Build + test the renamed crate**

Run:
```bash
cargo build -p scte35-splice --all-features --locked
cargo test  -p scte35-splice --all-features --locked
```
Expected: PASS (identical to prior dvb-scte35 results; same source).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor(scte35-splice): rename dvb-scte35 -> scte35-splice (history-preserving), v1.0.0, leaves lockstep"
```

---

### Task 2: Create `mp4-emsg` from `dvb-emsg` (history-preserving move)

**Files:**
- Move: `dvb-emsg/` → `mp4-emsg/` (via `git mv`)
- Modify: `mp4-emsg/Cargo.toml` (package name; version stays `0.1.0`)
- Modify: `Cargo.toml` (workspace members: `"dvb-emsg"` → `"mp4-emsg"`)

**Interfaces:**
- Produces: crate `mp4-emsg` v0.1.0, same API as `dvb-emsg` 0.1.0 (`EventMessageBox` + the `emsg::*` re-exports, `EmsgVersion`/`VERSION_0`/`VERSION_1`, `error`). Lib name `mp4_emsg`.

- [ ] **Step 1: Move the directory**

```bash
git mv dvb-emsg mp4-emsg
```

- [ ] **Step 2: Rename the package**

Edit `mp4-emsg/Cargo.toml`:
```toml
[package]
name        = "mp4-emsg"
version     = "0.1.0"
description = "ISO BMFF / DASH Event Message Box (emsg, ISO/IEC 23009-1) — parser + serializer, no_std."
```

- [ ] **Step 3: Update workspace members**

Root `Cargo.toml`: replace `"dvb-emsg"` with `"mp4-emsg"`.

- [ ] **Step 4: Fix internal references + crate doc**

```bash
grep -rn "dvb_emsg\|dvb-emsg" mp4-emsg/
```
Update the `//!` crate doc (drop DVB framing; keep ISO BMFF / DASH citation) and any `dvb_emsg::` doc/test refs → `mp4_emsg::`.

- [ ] **Step 5: Build + test**

Run:
```bash
cargo build -p mp4-emsg --all-features --locked
cargo test  -p mp4-emsg --all-features --locked
```
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor(mp4-emsg): rename dvb-emsg -> mp4-emsg (history-preserving), v0.1.0"
```

---

### Task 3: Recreate `dvb-scte35` as a deprecated re-export shim

**Files:**
- Create: `dvb-scte35/Cargo.toml`
- Create: `dvb-scte35/src/lib.rs`
- Create: `dvb-scte35/CHANGELOG.md` (append shim entry)
- Modify: root `Cargo.toml` (re-add `"dvb-scte35"` to members)

**Interfaces:**
- Consumes: `scte35-splice = "1.0"` (Task 1).
- Produces: `dvb-scte35` v7.9.1 — a shim whose entire API is `pub use scte35_splice::*`.

- [ ] **Step 1: Write the shim Cargo.toml**

Create `dvb-scte35/Cargo.toml`:
```toml
[package]
name         = "dvb-scte35"
version      = "7.9.1"
edition      = "2021"
license      = "MIT OR Apache-2.0"
repository   = "https://github.com/fishloa/rust-dvb"
rust-version = "1.81"
description  = "DEPRECATED: renamed to `scte35-splice`. This crate re-exports it."

[dependencies]
scte35-splice = { path = "../scte35-splice", version = "1.0" }

[features]
default = ["std", "serde", "chrono"]
std    = ["scte35-splice/std"]
serde  = ["scte35-splice/serde"]
chrono = ["scte35-splice/chrono"]
```
(Mirror the exact feature names that `scte35-splice` exposes — verify with `grep -A20 '\[features\]' scte35-splice/Cargo.toml` and match them.)

- [ ] **Step 2: Write the shim lib.rs**

Create `dvb-scte35/src/lib.rs`:
```rust
//! **DEPRECATED — renamed to [`scte35_splice`](https://crates.io/crates/scte35-splice).**
//!
//! This crate is a thin re-export shim kept so existing `dvb-scte35` dependencies
//! keep building. New code should depend on `scte35-splice` directly. No further
//! feature work lands here.
#![no_std]
#![allow(deprecated)]

pub use scte35_splice::*;
```

- [ ] **Step 3: Re-add to workspace members + write CHANGELOG**

Root `Cargo.toml` members: add `"dvb-scte35"` back (alongside `"scte35-splice"`).

Create/prepend `dvb-scte35/CHANGELOG.md`:
```markdown
# Changelog

## 7.9.1 — 2026-06-27
- **DEPRECATED.** Crate renamed to `scte35-splice`. This version is a re-export
  shim (`pub use scte35_splice::*`) of `scte35-splice 1.0.0`, which contains the
  exact code of `dvb-scte35 7.9.0`. Migrate to `scte35-splice`.
```

- [ ] **Step 4: Verify the shim compiles + re-exports resolve**

Run:
```bash
cargo build -p dvb-scte35 --all-features --locked
```
Expected: PASS. Then verify a symbol resolves through the shim:
```bash
cargo doc -p dvb-scte35 --no-deps --locked 2>&1 | tail -3
```
Expected: builds; `SpliceInfoSection` etc. visible via `dvb_scte35::`.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(dvb-scte35): convert to deprecated re-export shim of scte35-splice (v7.9.1)"
```

---

### Task 4: Recreate `dvb-emsg` as a deprecated re-export shim

**Files:**
- Create: `dvb-emsg/Cargo.toml`, `dvb-emsg/src/lib.rs`, `dvb-emsg/CHANGELOG.md`
- Modify: root `Cargo.toml` (re-add `"dvb-emsg"`)

**Interfaces:**
- Consumes: `mp4-emsg = "0.1"` (Task 2).
- Produces: `dvb-emsg` v0.1.1 shim (`pub use mp4_emsg::*`).

- [ ] **Step 1: Write the shim Cargo.toml**

Create `dvb-emsg/Cargo.toml`:
```toml
[package]
name         = "dvb-emsg"
version      = "0.1.1"
edition      = "2021"
license      = "MIT OR Apache-2.0"
repository   = "https://github.com/fishloa/rust-dvb"
rust-version = "1.81"
description  = "DEPRECATED: renamed to `mp4-emsg`. This crate re-exports it."

[dependencies]
mp4-emsg = { path = "../mp4-emsg", version = "0.1" }

[features]
default = ["std", "serde"]
std   = ["mp4-emsg/std"]
serde = ["mp4-emsg/serde"]
```
(Match `mp4-emsg`'s real feature set — verify and mirror exactly.)

- [ ] **Step 2: Write the shim lib.rs**

Create `dvb-emsg/src/lib.rs`:
```rust
//! **DEPRECATED — renamed to [`mp4_emsg`](https://crates.io/crates/mp4-emsg).**
//!
//! Thin re-export shim kept so existing `dvb-emsg` dependencies keep building.
//! New code should depend on `mp4-emsg` directly.
#![no_std]
#![allow(deprecated)]

pub use mp4_emsg::*;
```

- [ ] **Step 3: Re-add to members + CHANGELOG**

Root `Cargo.toml`: add `"dvb-emsg"` back.

Create `dvb-emsg/CHANGELOG.md`:
```markdown
# Changelog

## 0.1.1 — 2026-06-27
- **DEPRECATED.** Renamed to `mp4-emsg`. This version re-exports `mp4-emsg 0.1.0`
  (the exact code of `dvb-emsg 0.1.0`). Migrate to `mp4-emsg`.
```

- [ ] **Step 4: Verify**

```bash
cargo build -p dvb-emsg --all-features --locked
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(dvb-emsg): convert to deprecated re-export shim of mp4-emsg (v0.1.1)"
```

---

### Task 5: Update fuzz harness + any remaining references

**Files:**
- Modify: `fuzz/Cargo.toml` (dep names)
- Modify: any `fuzz/fuzz_targets/*.rs` using `dvb_scte35::`/`dvb_emsg::`

**Interfaces:**
- Consumes: `scte35-splice`, `mp4-emsg`.

- [ ] **Step 1: Find fuzz references**

```bash
grep -rn "dvb-scte35\|dvb_scte35\|dvb-emsg\|dvb_emsg" fuzz/
```

- [ ] **Step 2: Repoint fuzz deps to the new crates**

In `fuzz/Cargo.toml`, replace `dvb-scte35 = {...}` → `scte35-splice = { path = "../scte35-splice" }` and `dvb-emsg = {...}` → `mp4-emsg = { path = "../mp4-emsg" }`. Update target source `use dvb_scte35::` → `use scte35_splice::` and `dvb_emsg` → `mp4_emsg`.

- [ ] **Step 3: Verify the fuzz crate builds**

Run:
```bash
cargo +nightly fuzz build 2>/dev/null || cargo build --manifest-path fuzz/Cargo.toml --locked
```
Expected: builds (or, if nightly fuzz unavailable locally, `cargo check --manifest-path fuzz/Cargo.toml` passes).

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "build(fuzz): repoint scte35/emsg fuzz targets to renamed crates"
```

---

### Task 6: Full workspace gate suite

**Files:** none (verification only).

- [ ] **Step 1: Run the six CI gates locally**

Run each; all must pass:
```bash
cargo build --workspace --all-features --locked
cargo build --workspace --no-default-features --locked
cargo test  --workspace --all-features --locked
cargo clippy --workspace --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
```
Expected: all PASS. (Note: leaf binary/runtime crates already `--exclude`'d from the no-default lane per CI config — match the CI invocation exactly; check `.github/workflows`.)

- [ ] **Step 2: Confirm both shims + both renamed crates are workspace members**

```bash
grep -A30 'members' Cargo.toml | grep -E 'scte35-splice|mp4-emsg|dvb-scte35|dvb-emsg'
```
Expected: all four present.

- [ ] **Step 3: Commit any fmt fixes**

```bash
git add -A && git commit -m "style: fmt after rename" || echo "nothing to commit"
```

---

### Task 7: Documentation — READMEs, CLAUDE.md lockstep list, release notes

**Files:**
- Create: `scte35-splice/README.md`, `mp4-emsg/README.md`
- Modify: `CLAUDE.md` (lockstep crate list + crate descriptions)
- Create: `docs/release-notes/scte35-splice-v1.0.0.md`, `docs/release-notes/mp4-emsg-v0.1.0.md`

**Interfaces:** none.

- [ ] **Step 1: Write per-crate READMEs**

Each README per `docs/RELEASE-DOCS.md` standard: one-line purpose, spec citation, install snippet (`scte35-splice = "1.0"` / `mp4-emsg = "0.1"`), a minimal parse example, feature list, license. State the rename lineage ("formerly `dvb-scte35`").

- [ ] **Step 2: Update CLAUDE.md**

- Change the lockstep description from "seven core crates (… `dvb-scte35` …)" to **six** (`dvb-common`, `dvb-si`, `dvb-t2mi`, `dvb-bbframe`, `dvb-conformance`, `dvb-tools`); note `scte35-splice` is now independently versioned (like `scte104`/`mp4-emsg`).
- Update the crate inventory bullets: rename `dvb-scte35`→`scte35-splice`, `dvb-emsg`→`mp4-emsg`; mark old names as deprecated shims.

- [ ] **Step 3: Write release notes** per the RELEASE-DOCS standard for both new crates (lineage + "= old code" mapping).

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "docs: READMEs + CLAUDE.md lockstep update + release notes for scte35-splice/mp4-emsg rename"
```

---

### Task 8: PR, merge, release (owner-gated)

**Files:** none (release process).

- [ ] **Step 1: Open the PR**

```bash
git push -u origin <branch>
gh pr create --title "Rename dvb-scte35 -> scte35-splice, dvb-emsg -> mp4-emsg (with shims)" \
  --body "De-prefixes two mis-named crates. New crates scte35-splice 1.0.0 / mp4-emsg 0.1.0; old names become non-yanked re-export shims (dvb-scte35 7.9.1 / dvb-emsg 0.1.1). No functional change. Prereq for the timed-metadata crate."
```

- [ ] **Step 2: Verify ALL CI checks are SUCCESS** (not just mergeStateStatus) before merge.

- [ ] **Step 3: STOP — get explicit owner sign-off before any publish.** Per the hard rule, never trigger a crates.io publish without per-release OK.

- [ ] **Step 4: On approval, tag-driven release.** Publish order respects deps: `scte35-splice` + `mp4-emsg` (new crates) first, then the shims `dvb-scte35`/`dvb-emsg` (they depend on the new crates). Use the project's per-crate tag mechanism (independent crates use their own `<crate>-v<version>` tags, like `scte104`). Heed the release-tag traps: push companion tags one-at-a-time; new-crate first-publish may hit the crates.io 429 rate limit — wait out cooldown and re-run the idempotent lane.

- [ ] **Step 5: Post-publish verify.** Confirm `scte35-splice 1.0.0`, `mp4-emsg 0.1.0`, `dvb-scte35 7.9.1`, `dvb-emsg 0.1.1` are all live on crates.io and docs.rs built green. Verify a fresh `cargo add scte35-splice` resolves.

---

## Self-Review

- **Spec coverage:** both renames (Tasks 1–4), shims non-yanked (Tasks 3–4), workspace+fuzz refs (Tasks 1–5), full gate (Task 6), docs incl. lockstep change (Task 7), owner-gated release with trap mitigations (Task 8). Covered.
- **Version map** consistent across tasks: scte35-splice 1.0.0, mp4-emsg 0.1.0, shims 7.9.1 / 0.1.1.
- **No placeholders:** feature blocks flagged "verify and mirror exactly" because the source crate's real feature set must be read at implementation time — that is a concrete instruction, not a TBD.
- **Naming:** lib names `scte35_splice` / `mp4_emsg` used consistently in shims, fuzz, docs.
