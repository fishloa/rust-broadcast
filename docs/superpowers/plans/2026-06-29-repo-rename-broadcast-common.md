# Repo rename → rust-broadcast + dvb-common → broadcast-common — cutover plan

> **For agentic workers:** mechanical sweep, delegate-driven, gated by the orchestrator. Non-breaking via shim.

**Goal:** Rename the GitHub repo `rust-dvb` → `rust-broadcast` and the shared base crate `dvb-common` → `broadcast-common`, without breaking any existing consumer.

**Approach:** `broadcast-common` is a straight content move of `dvb-common`; `dvb-common` becomes a deprecated re-export shim (the pattern already used 6× for scte35-splice/mp4-emsg/etc). Full in-tree sweep repoints every crate to `broadcast-common`. Release in waves to respect the documented release-tag traps.

## Global constraints (verbatim)

- No publish without explicit per-release OK; owner reviews code before merge/publish.
- No `cargo publish` from a workstation — tags + CI only.
- No magic numbers / spec-citation discipline unchanged.
- broadcast-common content is byte-identical logic to dvb-common — pure rename, no behavior change.
- Type identity preserved: shim is `pub use broadcast_common::*` so `dvb_common::Parse` ≡ `broadcast_common::Parse` (no public `pub use dvb_common` re-exports exist, confirmed).
- Sweep size: 432 `.rs` files reference `dvb_common`; ~21 crates depend on `dvb-common`.

## Decisions locked

- New name: **broadcast-common** (owner pick 2026-06-29).
- Repo: **rust-broadcast** (roadmap-locked; slug free).
- Timing: one cutover PR now.
- Version: broadcast-common + lockstep → **8.1.0** (minor; rename is additive/non-breaking via shim). dvb-common shim also 8.1.0. mpeg-ts → **0.1.1** (patch; internal dep rename only).

## Decision (resolved 2026-06-29): republish EVERYTHING now

Owner chose full consistency: every dvb-common dependent is bumped + tagged + published on broadcast-common in this cutover. ~23 crates. Mechanical sweep by deterministic script; structural bits by ds-flash; releases gated by orchestrator + per-release owner OK.

**Independent crates to bump + republish (current → new, all patch/minor, dep-rename only):**
- mpeg-ts 0.1.0 → 0.1.1
- mpeg-ps 0.1.2 → 0.1.3
- mpeg-pes 0.1.1 → 0.1.2
- scte35-splice 1.0.0 → 1.0.1
- smpte2038 0.1.0 → 0.1.1
- ule 0.1.0 → 0.1.1
- cc-data 0.2.0 → 0.2.1
- scte104 0.1.0 → 0.1.1
- mp4-emsg 0.1.0 → 0.1.1
- timed-metadata 0.1.0 → 0.1.1
- dvb-subtitle 0.1.1 → 0.1.2
- dvb-ci 0.5.0 → 0.5.1
- dvb-ci-runtime 0.10.0 → 0.10.1
- dvb-simulcrypt 0.2.0 → 0.2.1
- dvb-vbi 0.1.0 → 0.1.1
- dvb-flute 0.1.0 → 0.1.1
- dvb-stream 0.2.1 → 0.2.2

(Deprecated shims dvb-cc/dvb-pes/dvb-scte35/dvb-emsg/dvb-smpte2038/dvb-ule do NOT dep dvb-common — untouched. dvb-demo/fuzz are non-published — repoint in-tree only.)

---

## Task 1: GitHub repo rename + repository fields

**Files:** every `Cargo.toml` with `repository = "https://github.com/fishloa/rust-dvb"`.

- [ ] Rename repo on GitHub: `rust-dvb` → `rust-broadcast` (Settings, or `gh repo rename`). GitHub auto-redirects old URLs.
- [ ] Update local remote: `git remote set-url origin git@github.com:fishloa/rust-broadcast.git`.
- [ ] Sweep `repository =` field in all manifests → `.../rust-broadcast`.
- [ ] Update README badge URLs / any hardcoded `rust-dvb` repo links in docs.
- [ ] Verify: `grep -rn "rust-dvb" --include=Cargo.toml .` empty (except intentional historical mentions in CHANGELOGs/release-notes, which stay).

## Task 2: Create broadcast-common crate

**Files:** Create `broadcast-common/` (src copied from `dvb-common/src/`), `broadcast-common/Cargo.toml`, `broadcast-common/README.md`, `broadcast-common/CHANGELOG.md`, `broadcast-common/examples/`.

- [ ] `git mv dvb-common/src broadcast-common/src` content (traits, crc32_mpeg2, bcd, bits, time, lib, impl_spec_display macro) — identical logic.
- [ ] Crate name `broadcast-common`, version `8.1.0`, same features (std/chrono), same deps (chrono, libm).
- [ ] Crate-root doc: drop the "dvb_si/dvb_t2mi/dvb_bbframe family" phrasing → "shared wire primitives for the broadcast/MPEG parser family".
- [ ] Move the two examples; keep the `#![doc = include_str!]` example embedding.
- [ ] Add to workspace members.
- [ ] Gate: `cargo test -p broadcast-common --all-features --locked`, `--no-default-features`, thumbv7em no_std, doc -D warnings.

## Task 3: dvb-common → deprecated shim

**Files:** `dvb-common/src/lib.rs` (replace with shim), `dvb-common/Cargo.toml`.

- [ ] `dvb-common/src/lib.rs` → `#![no_std] #![allow(deprecated)] pub use broadcast_common::*;` + deprecated module doc pointing at broadcast-common (mirror dvb-scte35 shim).
- [ ] Cargo.toml: version `8.1.0`, description "DEPRECATED: renamed to `broadcast-common`.", dep `broadcast-common = { path = "../broadcast-common", version = "8.1" }`, mirror std/chrono features → `broadcast-common/...`.
- [ ] Delete dvb-common's own src modules (now in broadcast-common) — keep only the shim lib.rs.
- [ ] Gate: `cargo build -p dvb-common --all-features --locked` (re-exports resolve).

## Task 4: Repoint all dependents (the 432-file sweep)

**Files:** every crate's `Cargo.toml` (dep `dvb-common` → `broadcast-common`) + every `.rs` (`dvb_common` → `broadcast_common`), EXCEPT the dvb-common shim itself.

- [ ] Cargo.toml: replace `dvb-common = { ... }` → `broadcast-common = { path = "../broadcast-common", version = "8.1", ... }` in all dependents (lockstep + mpeg-ts + independents + fuzz + bindings/python).
- [ ] Source: `dvb_common::` → `broadcast_common::`, `use dvb_common` → `use broadcast_common`, feature refs `dvb-common/std` → `broadcast-common/std` etc.
- [ ] Leave CHANGELOG/release-note historical mentions of dvb-common intact.
- [ ] Gate: full workspace build + test + clippy + fmt + doc + no-default + thumbv7em all green.

## Task 5: Version bumps + changelogs

- [ ] Lockstep (dvb-si, dvb-t2mi, dvb-bbframe, dvb-conformance, dvb-tools) → 8.1.0; CHANGELOG [8.1.0] note "depend on broadcast-common (was dvb-common)".
- [ ] mpeg-ts → 0.1.1; CHANGELOG note the dep rename.
- [ ] broadcast-common CHANGELOG [8.1.0] "Initial release — renamed from dvb-common."
- [ ] dvb-common CHANGELOG [8.1.0] "Deprecated — now a re-export shim for broadcast-common."
- [ ] Independent crates repointed in-tree (Task 4) but versions unchanged until their next release (per open decision).
- [ ] `docs/release-notes/v8.1.0.md` + RELEASE-DOCS checklist.

## Task 6: Release workflow for broadcast-common + lockstep wiring

- [ ] release.yml publish list: add `broadcast-common` FIRST (before dvb-common shim and the rest). New order: broadcast-common → dvb-common(shim) → dvb-bbframe → dvb-si → dvb-conformance → dvb-t2mi → dvb-tools. (dvb-common shim now depends on broadcast-common, so broadcast-common first.)
- [ ] Add `broadcast-common` to the lockstep version-gate loop in release.yml.
- [ ] Confirm dependency order: broadcast-common (no workspace deps) → everything else.

## Release sequence (after merge, on explicit OK)

1. **Tag `v8.1.0`** → release.yml publishes broadcast-common (NEW, no workspace deps → clean, no partial-publish dance) → dvb-common shim → bbframe → si → conformance → t2mi → tools. After this, broadcast-common 8.1.0 is live (prerequisite for every independent).
2. **Independent tags, pushed ONE AT A TIME** (memory: >3 tags in one push fires NO workflows): mpeg-ts-v0.1.1, mpeg-ps-v0.1.3, mpeg-pes-v0.1.2, scte35-splice-v1.0.1, smpte2038-v0.1.1, ule-v0.1.1, cc-data-v0.2.1, scte104-v0.1.1, mp4-emsg-v0.1.1, timed-metadata-v0.1.1, dvb-subtitle-v0.1.2, dvb-ci-v0.5.1, dvb-ci-runtime-v0.10.1, dvb-simulcrypt-v0.2.1, dvb-vbi-v0.1.1, dvb-flute-v0.1.1, dvb-stream-v0.2.2. Each deps broadcast-common 8.1 (now live) — no cross-tag race as long as v8.1.0 went first.
3. **Verify** every crate live on the **sparse index** (NOT the rate-limited JSON API); docs.rs green.
4. broadcast-common is only NEW first-publish (1) → low 429 risk; the rest are re-publishes (not new-crate-limited).

## Verification

- Full gate suite (orchestrator-run): build/test/clippy/fmt/doc/no-default/thumbv7em, all-features + MSRV 1.81.
- `cargo build -p dvb-common` proves the shim re-export resolves (type identity).
- A consumer pinning `dvb-common = "8"` still builds against the 8.1.0 shim (back-compat check).
