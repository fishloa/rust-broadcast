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

- **Version table** — every crate's `Cargo.toml` `version`. The 5 lockstep crates (`dvb-si`, `dvb-t2mi`, `dvb-bbframe`, `dvb-conformance`, `dvb-tools`) must be identical. `broadcast-common` is **not** in the lockstep: it versions on its own API changes and ships from its own `broadcast-common-v*` tag (`release-broadcast-common.yml`). Independents (`dvb-stream`, `scte35-splice`, `mp4-emsg`, `mpeg-pes`, `dvb-subtitle`, `mpeg-ps`, `scte104`, `cc-data`, `st291`, `ule`, `dvb-ci`) version on their own cadence.
- **Inter-crate dep-ref matrix** — for EVERY crate, list each workspace-crate dependency and its pinned version req. **Bump EVERY inter-crate ref in lockstep**, not just the immediately-prior-version refs. A `^X.Y` range that still covers the new version won't *fail* publish, but bump it anyway for hygiene and to avoid a future major-bump trap. Flag any ref pinned **higher** than what is published (the real trap) or any hard `=` pin.
- **Which crates need a bump** — a non-empty `[Unreleased]`/`## Unreleased` CHANGELOG section ⇒ needs a bump. Semantics: patch = fixes only; minor = additive API; major = breaking. Lockstep crates with no change still bump in sync (with a "lockstep release; no functional changes" entry).
- **Release lanes** — each independent/new crate has its own `release-<crate>.yml` (tag `*-v*`); the lockstep `release.yml` (tag `v*`) publishes the five lockstep crates in **dependency order** (`dvb-bbframe` first) and `--exclude`s the independents/new crates. When `broadcast-common` moves, its `broadcast-common-v*` tag goes first and must be live before the lockstep `v*` tag. Confirm every new crate has a lane.
- **Cargo.lock** — no conflict markers (`<<<<<<<`/`=======`/`>>>>>>>`); all crates present at expected versions. (A merge can leave a lock marker a failing `cargo build` won't refresh — take `origin/main`'s lock + rebuild.)
- **Verify published versions against crates.io ground truth** — `curl https://crates.io/api/v1/crates/<name>` → `.crate.max_version`. **Do NOT infer "unreleased" from CHANGELOG heading structure** — that heuristic is wrong (it mislabeled already-live dvb-subtitle/scte104). crates.io is the only authority for what's live.
- **Epoch-pure compat buckets** — changing which caret-epoch of a workspace sibling a crate builds against is a **major-class change** in that crate (minor for 0.x, major for >=1.0). Every published compat bucket must stay epoch-pure: all published versions inside the same bucket must agree on every workspace-sibling epoch. Violating this — as mpeg-ts 0.3.1 did when it bumped broadcast-common ^8→^9 inside the 0.3 bucket — creates a "two majors of one crate" resolution for downstream consumers, because a prior version in the same bucket still pulls in the old epoch transitively (issue #858). `tools/check-published-dep-consistency.py` machine-checks this (checks 1–2), so the rule is enforced, not just documented.

---

## 3. Extensibility / code-quality audit (the deep dimension)

Run an adversarial, read-only auditor per new crate AND per new module added to an existing crate (skeptical framing: a PASS must be earned with file:line evidence). These dimensions have caught a real bug nearly every release. Per [[never-leave-found-bugs-unfixed]], fix everything surfaced in the same pass.

- **A00. Fixture homes** — no test depends on an untracked/gitignored fixture (`grep -rn "local-only\|archive-gated" --include='*.rs' --include='*.md'` is empty outside history files); every capture is public-committed or in `private/fixtures/<topic>/` with PROVENANCE, resolved through `test-fixtures` (CRATE-ACCEPTANCE section 3).
- **A0. Test-integrity gates (T3/T11)** — round-trip compares against the ORIGINAL fixture slice byte-exactly (never parsed-AST equality or re-canonicalised bytes); an independent oracle (libsrt, ffmpeg/GPAC, `mediastreamvalidator`, libebur128/EBU Tech 3341/3342, libzvbi, TSDuck) backs each protocol and self-produced expected values do not count; a fixture exists per spec branch (v0/v1, each flag combo) and >= 2 entities where a uniqueness invariant exists; reserved-bit/trailing-byte policy followed (CRATE-ACCEPTANCE section 1). See also 3.6 Bug classes.
- **A. Round-trip symmetry** — every `Parse` has a symmetric `Serialize` that rebuilds from **typed fields**, never echoes a stored `raw`/`bytes` slice (a passthrough serialize is gameable and silently wrong). Round-trip test must *bite*: construct-from-fields → hand-computed wire bytes, mutate a field → assert the bytes change, ≥2-element boundary, plus a **real-fixture** parse + byte-exact round-trip. A plain round-trip test alone is gameable by raw passthrough.
  - **One-way decoders** (e.g. CEA-608/708 caption decode) are exempt from round-trip — instead require **known-spec-vector decode tests** that bite (a worked example from the spec) AND a `no_panic_on_arbitrary_input` test feeding truncated/random bytes.
- **B. No raw-byte public API** — structured wire data the spec gives a layout to must be fully typed (typed structs/enums/`Vec`/accessors); never `pub field: &[u8]` "decode it yourself". Only genuinely opaque/reserved/private/crypto/FEC payloads may stay `&[u8]`.
- **C. Decode-completeness** — any coded value the spec maps to a name/meaning must be a typed enum or have a decode accessor. Clients must never re-implement a spec lookup table. Flag raw integers the spec assigns named values.
- **D. Spec-fidelity** — every field in the spec syntax appears in the struct; module/crate doc cites spec + section; **no magic numbers** (spec-identity hex/decimal constants) outside `#[cfg(test)]` — every one named. Inline bitmask/range-dispatch hex (`& 0x7F`, `(0x20..=0x7F)`, control-byte comparisons) is the established convention and is NOT a violation.
- **E. Enum label convention (#204)** — EVERY public spec/field enum exposes `pub fn name(&self) -> &'static str` (hand-written match arms; spec token per variant, `"reserved"` for the catch-all) AND `broadcast_common::impl_spec_display!(...)` for `Display`. Labels live in source, never in the macro. **Each crate must have a `tests/label_coverage.rs` drift-guard** that scans `src/` and fails CI if any public enum lacks `Display` (minus a documented SKIP list: errors, `Any*`/tag-dispatch enums, section-kind discriminants, data-carrying ADTs). A new crate with public enums and no `label_coverage.rs` is a convention violation.
- **F. `#[non_exhaustive]`** on every public enum (forward-compat).
- **G. Panic-class safety** — no unchecked indexing / `.unwrap()` / `.expect()` / `panic!` / `unreachable!` outside `#[cfg(test)]`; bounds checked before slicing; serializers check the output buffer size first. Fuzz coverage exists for every parser crate (CI nightly fuzz-build gate).
- **H. Extensibility / dispatch** — any dispatch set (table_ids, descriptor tags, message/packet/parameter types) is generated from a single declarative list via the `declare_*!` macro / `*Def` trait, with a **drift test** pinning each literal to the type's trait const. Adding a type is one line. Flag any hand-maintained match that can silently drift.
- **I. No-hand-roll tripwires re-run** — re-run every `tests/*guard*.rs` tripwire (`cargo test -p <crate> --test no_handroll_guard` / `--test dehandroll_guard`) and re-run the **section-3 inventory greps** of the de-hand-roll spec (`docs/superpowers/specs/2026-10-03-protocol-runtime-dehandroll-design.md` §3 is the checked-in inventory; regenerate the per-category greps from it): HTTP/RTSP framing, URL, SDP/ICE, base64/hex/dates, and the runtime/concurrency patterns, over `src/**/*.rs` outside `#[cfg(test)]`, comparing hits against the reasoned allowlists. Every new hit is either a bug, a **documented exception** (spec §9), or a fresh reasoned allow entry — never an unexplained survivor. A tripwire is lexical; the grep is the second, independent pass.

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

## 3.6 Bug classes seen (sibling-grep register)

Every fix greps the whole workspace for siblings of its class and adds a regression test per sibling ([`CRATE-ACCEPTANCE.md`](CRATE-ACCEPTANCE.md) section 3). Classes come from the 2026-09-24 full-read audit (`audit(systemic)` issues). The last column is the guard/fix pattern that exists in this repo today; "none yet" means the class is recorded but has no repo-wide guard, so the audit greps for it by hand.

| ID | Bug class | Guard / fix pattern in this repo |
|---|---|---|
| T1 | Serializers narrow a length/count field before the range check (`as u16` wraps) | `broadcast_common::len` `fit_bits`/`fit_u8`/`fit_u16`/`fit_u24`/`fit_u32`; `descriptors::write_descriptor_header` |
| T2 | Allocation sizes taken from untrusted wire data | Capped accumulators (`broadcast_common::pusi::DEFAULT_MAX_UNIT_SIZE`, used by `mpeg-pes`/`mpeg-ts`); cap before `with_capacity` |
| T3 | Tests that cannot fail | Byte-exact-vs-original-fixture, independent-oracle and per-branch gates ([`CRATE-ACCEPTANCE.md`](CRATE-ACCEPTANCE.md) section 3); stub-the-impl biting rule |
| T4 | Timelines rebuilt from zero | none yet (audit by hand; see `timed-metadata` `Timeline`) |
| T5 | Reconnect mints new state | none yet (audit `multimux` supervised reconnect by hand) |
| T6 | Mutex poisoning cascades | `parking_lot` locks (`multimux`, `media-plane`) |
| T7 | Blocking in async | De-hand-roll tripwires flag `thread::sleep`/`sleep(` in non-test async code (`tests/no_handroll_guard.rs`) |
| T8 | Fix applied to one copy | The sibling-grep rule above; shared helpers in `broadcast-common` |
| T9 | 33-bit clock wrap | `broadcast_common::clock33` (`add`, `add_signed`, `signed_distance`, `unwrap_delta`) + per-crate `tests/clock33_guard.rs` |
| T10 | Invalid states representable | Typed enums/fields, `#[non_exhaustive]` coverage guard (`tests/non_exhaustive_coverage.rs`) |
| T11 | Reserved-bit / trailing-byte policy | Workspace policy in `broadcast-common` crate docs and [`CRATE-ACCEPTANCE.md`](CRATE-ACCEPTANCE.md) section 1 |
| T12 | Attribute tokenizer duplicated | Consolidate on one shared tokenizer; the de-hand-roll tripwires catch re-introduced hand-rolled parsing |
| T13 | Drifted duplicate implementations | Hoist into `broadcast-common` (`pusi`, `seq`, `clock33`, `ts_dup`) so there is one copy |
| T15 | Panicking `to_bytes` on fallible serializers | `Serialize::try_to_bytes` (use for anything derived from untrusted input) |
| T16 | One bad packet aborts the stream | Resync/skip paths in `mpeg-ts` (`resync`) and `dvb-stream`; in-band `Lagged` in `media-plane` instead of an error |

## 4. Per-release checklist (the staging steps after audits are green)

Per [`RELEASE-DOCS.md`](RELEASE-DOCS.md): version bumps (lockstep together; bump every inter-crate ref) → CHANGELOG `[Unreleased]` cut to a dated heading → `docs/release-notes/vX.Y.Z.md` → README coverage → crate-root `//!` (incl. CLI `--help` text for new tool features) → Cargo.toml + `[package.metadata.docs.rs]` sweep → branch → PR (`Closes #n`) → **all CI checks SUCCESS (not just mergeStateStatus)** → merge → version-tag push (release CI gates + publishes) → post-publish verify docs.rs built green + every crate live on crates.io.

**Never `cargo publish` / push a release tag from a workstation, and never tag without the owner's explicit per-release sign-off.** Releases are tag-driven, CI-only.
