# Release documentation standard

The authoritative standard for what documentation every `rust-dvb` release must
produce, and where. Docs are a product surface; the spec-citation discipline is
this family's differentiator and must surface on docs.rs and crates.io.

The per-release checklist below is enforced as part of the release flow (see
CLAUDE.md → "Releases are tag-driven"). One-time setup was completed in v7.4.0.

## The three surfaces

### 1. docs.rs — the API reference (highest leverage)
- **Crate-root `//!` doc** that orients: one-sentence what-it-is, the spec it
  implements **with citation** (e.g. "ETSI EN 300 468 V1.19.1"), a **doctested**
  quickstart, a feature-flag table, MSRV + `no_std` note, intra-doc links to
  sibling crates.
- **`[package.metadata.docs.rs]` in every library Cargo.toml** — without it
  docs.rs builds default-features only and feature-gated API (`ts`/`serde`/
  `chrono`) vanishes:
  ```toml
  [package.metadata.docs.rs]
  all-features = true
  rustdoc-args = ["--cfg", "docsrs"]
  ```
  plus `#![cfg_attr(docsrs, feature(doc_cfg))]` in `lib.rs`, and
  `#[cfg_attr(docsrs, doc(cfg(feature = "…")))]` on feature-gated public items so
  docs.rs shows the gating pills.
- Every public item documented; `RUSTDOCFLAGS=-D warnings` gated in CI.

### 2. crates.io — the landing page (discovery + first impression)
- **Per-crate `README.md`** (all crates, not just the workspace one) — crates.io
  renders this: badges, one-paragraph pitch, coverage table, quickstart, links.
- **Cargo.toml discoverability metadata**, audited each release: `description`
  (keyword-rich — it's the search-result line), `keywords` (≤5),
  `categories` (crates.io fixed list: `parser-implementations`, `encoding`,
  `no-std`, `multimedia::encoding`), `repository`, `documentation`, `license`,
  `readme`, `rust-version`.

### 3. GitHub — source of truth + release record
- **Workspace `README.md`**: architecture map, the crate table, the spec-grounding
  story and the `docs/` transcription tree.
- **Per-crate `CHANGELOG.md`** in keep-a-changelog format.
- **A GitHub Release per tag**, body = the `docs/release-notes/vX.Y.Z.md` narrative.

## Per-release checklist (run every tag)

1. **CHANGELOG** — `[Unreleased]` → `## X.Y.Z — DATE` in every changed crate;
   lockstep crates with no functional change get a one-line "Lockstep release"
   entry.
2. **Release note** — `docs/release-notes/vX.Y.Z.md` (highlights, breaking
   changes + migration, PR/issue refs).
3. **README coverage tables** — update each crate's coverage/feature table.
4. **Crate-root `//!`** — refresh overview + quickstart if the public surface
   changed, so docs.rs lands current.
5. **Cargo.toml metadata sweep** — description/keywords/categories accurate;
   `[package.metadata.docs.rs]` present.
6. **GitHub Release** — created from the tag, body = the release note.
7. **Post-publish verification** — confirm **docs.rs built green** (it can fail
   independently of a successful `cargo publish`), version badges resolve, the
   crates.io page renders the README.

## Also targeted

- **lib.rs** — auto-ingests from crates.io; driven by `categories`/`keywords`
  (no separate action).
- **SECURITY.md** — supported versions, private reporting, hardening posture
  (parser family + cargo-fuzz).
- **`examples/`** — small per-crate runnable examples double as docs and are
  CI-compilable; surface on docs.rs. Every library crate ships two (a basic
  quickstart + an advanced/real-capture walkthrough) as of #253; the CI
  `cargo build --workspace --examples --all-features` step keeps them building.
- **GitHub Pages** — hosts the WASM demo (fishloa.github.io/rust-dvb); optional,
  not per-release.
- **Blog / r/rust / This Week in Rust** — only for major releases, skipped for
  routine minors.
