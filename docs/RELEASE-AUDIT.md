# Release audit playbook

The full battery of tests and checks run before every release tag. This is the
**"how to audit"** companion to:
- [`AUDIT-LEDGER.md`](AUDIT-LEDGER.md) — *what* PDF→md fidelity audits have been done (skip re-verifying unchanged doc sets).
- [`RELEASE-DOCS.md`](RELEASE-DOCS.md) — the documentation *deliverables* per release (CHANGELOG, release note, README, crate-root, docs.rs metadata).
- [`DESCRIPTOR-ACCEPTANCE.md`](DESCRIPTOR-ACCEPTANCE.md) — the acceptance gate for a *new descriptor*.

Rule: **Claude runs every gate itself and reads every audit; a release is never tagged on a subagent's say-so or on CI alone.** Run gates in an isolated `CARGO_TARGET_DIR` (e.g. `target-rel`) to avoid rust-analyzer lock contention; never two `cargo` invocations on one target dir at once.

---

## 1. Gate suite (mirror CI exactly — run yourself, `RUSTFLAGS="-D warnings"`)

All six must be green on a fresh run before staging a release:

1. `cargo fmt --all --check` — clean. (Local rustfmt can skew vs CI; if CI fmt fails, copy CI's expected wrap — don't trust local fmt.)
2. `cargo build --workspace --all-features --locked`
3. `cargo test --workspace --all-features --locked`
4. `cargo build --workspace --no-default-features --locked` (host; some leaf/bin crates may need `--exclude` per CI)
5. `cargo clippy --workspace --all-features --all-targets --locked -- -D warnings`
6. `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked`

Bare-metal `no_std` (`thumbv7em-none-eabi`) per-lib loop is the CI no_std job — covered on each merge PR; re-confirm if any lib changed since.

Defensive: the recurring dvb-si "hang" = concurrent `cargo test` on one crate + a rustdoc doctest deadlock. Use a single target dir per run and don't parallelize same-crate test runs.

---

## 2. Release-readiness audit (version + publish safety)

The v7.7.0 **partial-publish incident** (a stale `"7.4.0"` inter-crate ref didn't match the version being published; publish failed after 4 crates were already live) is why this section exists.

- **Version table** — every crate's `Cargo.toml` `version`. The 6 lockstep core crates (`dvb-common`, `dvb-si`, `dvb-t2mi`, `dvb-bbframe`, `dvb-conformance`, `dvb-tools`) must be identical. Independents (`dvb-stream`, `scte35-splice`, `mp4-emsg`, `mpeg-pes`, `dvb-subtitle`, `mpeg-ps`, `scte104`, `cc-data`, `st291`, `ule`, `dvb-ci`) version on their own cadence.
- **Inter-crate dep-ref matrix** — for EVERY crate, list each workspace-crate dependency and its pinned version req. **Bump EVERY inter-crate ref in lockstep**, not just the immediately-prior-version refs. A `^X.Y` range that still covers the new version won't *fail* publish, but bump it anyway for hygiene and to avoid a future major-bump trap. Flag any ref pinned **higher** than what is published (the real trap) or any hard `=` pin.
- **Which crates need a bump** — a non-empty `[Unreleased]`/`## Unreleased` CHANGELOG section ⇒ needs a bump. Semantics: patch = fixes only; minor = additive API; major = breaking. Lockstep crates with no change still bump in sync (with a "lockstep release; no functional changes" entry).
- **Release lanes** — each independent/new crate has its own `release-<crate>.yml` (tag `*-v*`); the lockstep `release.yml` (tag `v*`) publishes in **dependency order** (dvb-common first) and `--exclude`s the independents/new crates. Confirm every new crate has a lane.
- **Cargo.lock** — no conflict markers (`<<<<<<<`/`=======`/`>>>>>>>`); all crates present at expected versions. (A merge can leave a lock marker a failing `cargo build` won't refresh — take `origin/main`'s lock + rebuild.)
- **Verify published versions against crates.io ground truth** — `curl https://crates.io/api/v1/crates/<name>` → `.crate.max_version`. **Do NOT infer "unreleased" from CHANGELOG heading structure** — that heuristic is wrong (it mislabeled already-live dvb-subtitle/scte104). crates.io is the only authority for what's live.

---

## 3. Extensibility / code-quality audit (the deep dimension)

Run an adversarial, read-only auditor per new crate AND per new module added to an existing crate (skeptical framing: a PASS must be earned with file:line evidence). These dimensions have caught a real bug nearly every release. Per [[never-leave-found-bugs-unfixed]], fix everything surfaced in the same pass.

- **A. Round-trip symmetry** — every `Parse` has a symmetric `Serialize` that rebuilds from **typed fields**, never echoes a stored `raw`/`bytes` slice (a passthrough serialize is gameable and silently wrong). Round-trip test must *bite*: construct-from-fields → hand-computed wire bytes, mutate a field → assert the bytes change, ≥2-element boundary, plus a **real-fixture** parse + byte-exact round-trip. A plain round-trip test alone is gameable by raw passthrough.
  - **One-way decoders** (e.g. CEA-608/708 caption decode) are exempt from round-trip — instead require **known-spec-vector decode tests** that bite (a worked example from the spec) AND a `no_panic_on_arbitrary_input` test feeding truncated/random bytes.
- **B. No raw-byte public API** — structured wire data the spec gives a layout to must be fully typed (typed structs/enums/`Vec`/accessors); never `pub field: &[u8]` "decode it yourself". Only genuinely opaque/reserved/private/crypto/FEC payloads may stay `&[u8]`.
- **C. Decode-completeness** — any coded value the spec maps to a name/meaning must be a typed enum or have a decode accessor. Clients must never re-implement a spec lookup table. Flag raw integers the spec assigns named values.
- **D. Spec-fidelity** — every field in the spec syntax appears in the struct; module/crate doc cites spec + section; **no magic numbers** (spec-identity hex/decimal constants) outside `#[cfg(test)]` — every one named. Inline bitmask/range-dispatch hex (`& 0x7F`, `(0x20..=0x7F)`, control-byte comparisons) is the established convention and is NOT a violation.
- **E. Enum label convention (#204)** — EVERY public spec/field enum exposes `pub fn name(&self) -> &'static str` (hand-written match arms; spec token per variant, `"reserved"` for the catch-all) AND `dvb_common::impl_spec_display!(...)` for `Display`. Labels live in source, never in the macro. **Each crate must have a `tests/label_coverage.rs` drift-guard** that scans `src/` and fails CI if any public enum lacks `Display` (minus a documented SKIP list: errors, `Any*`/tag-dispatch enums, section-kind discriminants, data-carrying ADTs). A new crate with public enums and no `label_coverage.rs` is a convention violation.
- **F. `#[non_exhaustive]`** on every public enum (forward-compat).
- **G. Panic-class safety** — no unchecked indexing / `.unwrap()` / `.expect()` / `panic!` / `unreachable!` outside `#[cfg(test)]`; bounds checked before slicing; serializers check the output buffer size first. Fuzz coverage exists for every parser crate (CI nightly fuzz-build gate).
- **H. Extensibility / dispatch** — any dispatch set (table_ids, descriptor tags, message/packet/parameter types) is generated from a single declarative list via the `declare_*!` macro / `*Def` trait, with a **drift test** pinning each literal to the type's trait const. Adding a type is one line. Flag any hand-maintained match that can silently drift.

---

## 3.5 Documentation-accuracy audit (docs match the CODE, not just build clean)

The gate suite proves docs *compile* (examples build, doctests run, rustdoc is warning-free) — it does **not** prove they are *accurate*. A README coverage table can be green-building and still silently omit a shipped feature (real incident: transmux 0.12.0 shipped ProgressiveDemux / streaming TS-HLS / `source_pid` while the README listed none of them). For **every crate with a non-empty `[Unreleased]`**, verify — reading the source as ground truth, not the prose:

- **README coverage/feature tables ⇄ actual public API.** Cross-check against `lib.rs` `pub use`/`pub mod` + the crate-root `//!`. Every new public spoke / codec / transform / CLI flag added since the last release appears; nothing listed is absent or renamed. No stale "planned/⬜" row for something now implemented.
- **CHANGELOG `[Unreleased]` ⇄ the real diff.** Every user-visible change since the last tag (`git diff <last-tag>..HEAD` per crate) has an entry; every entry describes something that actually happened; breaking changes are under a `### Breaking` heading. Issue/PR refs present.
- **Examples exercise the CURRENT API.** Not just "compiles" — the example must use present-day signatures/constructors (e.g. after a `#[non_exhaustive]` + constructor migration, examples use the constructor, not a stale literal) and demonstrate a real path; fixture-driven examples actually run.
- **Crate-root `//!` overview + quickstart** reflect the current surface (docs.rs lands here).
- **Spec citations** in new/changed module docs point to the correct spec + section (spot-check the modules touched this release).
- **Cargo.toml metadata** (`description`/`keywords`/`categories`) still accurate for what the crate now does.

Fix everything found in the staging PR (per [[never-leave-found-bugs-unfixed]]) — a stale README is a release defect, not a nicety.

---

## 4. Per-release checklist (the staging steps after audits are green)

Per [`RELEASE-DOCS.md`](RELEASE-DOCS.md): version bumps (lockstep together; bump every inter-crate ref) → CHANGELOG `[Unreleased]` cut to a dated heading → `docs/release-notes/vX.Y.Z.md` → README coverage → crate-root `//!` (incl. CLI `--help` text for new tool features) → Cargo.toml + `[package.metadata.docs.rs]` sweep → branch → PR (`Closes #n`) → **all CI checks SUCCESS (not just mergeStateStatus)** → merge → version-tag push (release CI gates + publishes) → post-publish verify docs.rs built green + every crate live on crates.io.

**Never `cargo publish` / push a release tag from a workstation, and never tag without the owner's explicit per-release sign-off.** Releases are tag-driven, CI-only.
