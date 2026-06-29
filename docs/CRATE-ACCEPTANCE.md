# Crate acceptance standard (every crates.io publish)

The consolidated hard bar **every crates.io publish** clears � not just new crates. A **new crate** clears the entire bar; a **new version** of an existing crate keeps the whole crate compliant and holds any newly-added wire type to the same invariants. Gated per-release via [`RELEASE-AUDIT.md`](RELEASE-AUDIT.md).
Companion to [`DESCRIPTOR-ACCEPTANCE.md`](DESCRIPTOR-ACCEPTANCE.md) (descriptor-level),
[`RELEASE-DOCS.md`](RELEASE-DOCS.md) (per-release docs) and [`RELEASE-AUDIT.md`](RELEASE-AUDIT.md)
(pre-tag battery). Where those overlap, this doc is the index; they hold the detail.

> **This is a living document.** As we learn how to build these crates well, tighten it —
> every recurring failure mode becomes a line here so it is gated, not rediscovered. Improve
> code, docs, and quality all round, continuously. (CLAUDE.md "continuous improvement".)

## 1. Wire contract — the non-negotiable invariants
- **Symmetric `Parse` + `Serialize`** for every wire structure, with a **round-trip test that is byte-identical** — `parse(real_bytes) → serialize → ==` the input bytes, INCLUDING stuffing/padding/reserved bits. "Semantic" round-trip (re-parse → fields equal) is **NOT sufficient** — it is passed by garbage and by raw-passthrough serializers.
- **No `self.raw` passthrough** in serialize: lengths/offsets are COMPUTED from fields, never stored-and-echoed. `grep self.raw` inside serialize must be empty.
- **No raw-byte public API** for structured data: fully typed (typed `Vec`/enums). Only genuinely-opaque blobs (compressed/private/unknown) stay `&[u8]`, documented as such.
- **No magic numbers** outside `#[cfg(test)]`: every hex literal is a named const or enum.
- **`#[non_exhaustive]`** on every public enum/struct that may gain variants/fields.
- **Decode-completeness**: every coded spec field that maps to a name has a typed accessor — clients never re-implement a spec lookup table.

## 2. Spec grounding & sourcing discipline
- **Module docs cite the spec** (name, section, tag/box/table id). Transcribe the cited syntax into the crate's own `docs/` where it's a parser.
- **No implementation without a verified primary source.** Never encode a value/layout from memory or "well-known facts". If no source: mark GAP, don't fabricate.
- **Verify the SOURCE, not name-presence.** A spec/fixture that merely *mentions* a structure is not a syntax source (profiles reference; they don't reproduce). Check the actual field tables. (Session lesson: CFF/UltraViolet, ETSI ECI — all profile/reference docs, rejected.)
- **pdf2md gate**: transcribe spec tables with `pdf2md … --report`; require **exit 0** (value-verified). A **scanned/image PDF has no text layer → unverifiable** → cross-check source only, never a verified primary. Orchestrator produces the verified md once; delegates format from the md, never re-read the PDF.
- **Spec-posture departure** (impl-cited instead of spec-cited, e.g. paid ISO codec specs) is allowed **only for explicitly-designated crates** (e.g. `transmux`) and **only with golden-byte / reference-player validation** standing in for the citation.

## 3. Real-fixture gate (the bug-catcher)
- A **committed real fixture** (broadcast capture / spec test vector), not hand-made happy-path bytes. Real data carries the reserved bits, mixed stream_ids, stuffing, and layouts that expose bugs.
- Parser crate: **parse + byte-exact round-trip the real fixture.**
- Transform/repair/mux crate: **fault-inject → operate → assert-known-good** on real data (oracle = the clean original or a third-party reference). Each operation gets its own biting test.
- **Tests must BITE**: stub the impl to a no-op and the test must FAIL. A green suite that a wrong impl also passes is not a gate.
- **Fixtures must be genuine**: unscrambled, structurally valid for the field under test. (Session lesson: scrambled packets misparse as plausible typed fields under loose assertions — byte-identical + provenance checks catch it.)
- Fixtures read at runtime via `concat!(env!("CARGO_MANIFEST_DIR"), …)` + `std::fs` — NEVER a bare relative path (silently skips under cargo's CWD), NEVER `include_bytes!` for example fixtures (breaks publish/docs.rs), NEVER a skip-on-missing branch (a missing fixture must panic, or the gate vacuously passes).

## 4. The 6-gate CI suite (run by the orchestrator, not on a delegate's say-so)
`cargo build --workspace --all-features --locked` · `cargo test --workspace --all-features --locked` ·
`cargo build --workspace --no-default-features --locked` · `cargo clippy --workspace --all-features --all-targets --locked -- -D warnings` ·
`cargo fmt --all --check` · `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked`.
Plus **MSRV 1.81** and, for `no_std` crates, a **bare-metal `thumbv7em-none-eabi`** build.

## 5. `no_std`, labels, fuzz, examples, CLI
- **`no_std` + `alloc`** where practical; `--no-default-features` + bare-metal build green.
- **#204 label convention**: every public spec/field enum gets `name() -> &'static str` (hand-written) + `dvb_common`/`broadcast_common::impl_spec_display!`; a per-crate `tests/label_coverage.rs` drift-guard (or a documented SKIP list).
- **Fuzz target** added (the workspace fuzzes every parser crate; nightly fuzz-build gate).
- **≥2 runnable examples** (`cargo run -p <crate> --example …`), fixtures via `std::fs` at runtime.
- **CLI** (if any) follows [`CLI-STANDARD.md`](CLI-STANDARD.md): clap derive, named flags, no positional magic numbers, auto `--help`/`--version`.

## 6. Docs & release ([`RELEASE-DOCS.md`](RELEASE-DOCS.md))
CHANGELOG (Keep-a-Changelog) · `docs/release-notes/<ver>.md` · README (badges, install, quickstart, spec cite, license) · crate-root `//!` with spec citation + embedded example list · `[package.metadata.docs.rs]` (all-features + `doc(cfg)`) · GitHub Release. **Docs are updated in the same PR as the change**, not batched.

## 7. Versioning & releases
Independent crates version on their own cadence; lockstep crates move together. Dep-ref consistency verified against crates.io ground truth (the partial-publish trap). **Tag-driven, CI-only publish — never `cargo publish` from a workstation.** No publish without explicit owner sign-off.

---
### Acceptance checklist (tick before any publish � new crate or new version)
- [ ] Symmetric parse/serialize + **byte-identical** round-trip test
- [ ] No `self.raw` passthrough · no raw-byte public API · no magic numbers · `#[non_exhaustive]`
- [ ] Spec-cited modules; sources verified (no fabrication; pdf2md exit 0 / cross-checked)
- [ ] Committed **real fixture**; per-op **biting** test (fault-inject→assert-known-good for transforms)
- [ ] 6-gate CI suite + MSRV 1.81 (+ thumbv7em if no_std) — run by the orchestrator
- [ ] #204 labels + label_coverage · fuzz target · ≥2 examples · CLI-STANDARD (if CLI)
- [ ] RELEASE-DOCS complete (CHANGELOG/release-note/README/crate-root/docs.rs metadata)
- [ ] Versioning + dep-refs consistent; publish staged for owner sign-off
