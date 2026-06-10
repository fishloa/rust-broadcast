# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A Rust workspace of DVB (Digital Video Broadcasting) protocol parsers + builders, published to crates.io:

- **dvb-common** — shared `Parse<'a>` / `Serialize` traits and CRC-32/MPEG-2. Everything else depends on it.
- **dvb-si** — the big one: ETSI EN 300 468 Service Information + MPEG-2 PSI. All 29 allocated table_ids, descriptors, DSM-CC data carousel, Annex A text decoding, TS packet / section reassembly.
- **dvb-t2mi** — TS 102 773 T2-MI packet/payload parsing.
- **dvb-bbframe** — DVB-S2/S2X/T2 BBFrame headers, user packet extraction.
- **dvb-scte35** — reserved crate (planned; directory exists but is not yet a workspace member).

MSRV is **1.75** (workspace `rust-version`); the committed `Cargo.lock` pins MSRV-compatible deps — always build/test with `--locked`.

## Commands

```bash
# Full check, exactly what CI runs (CI sets RUSTFLAGS="-D warnings"):
cargo build --workspace --all-features --locked
cargo test  --workspace --all-features --locked
cargo build --workspace --no-default-features --locked
cargo clippy --workspace --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked

# Scoped runs:
cargo test -p dvb-si --all-features                # one crate
cargo test -p dvb-si --test round_trip             # one integration test file
cargo test -p dvb-si descriptors::pdc              # tests matching a path

# Examples (debug CLIs):
cargo run -p dvb-si --example si_dump -- dvb-si/tests/fixtures/m6-single.ts
cargo run -p dvb-si --example si_dump -- dvb-si/tests/fixtures/m6-single.ts --json
cargo run -p dvb-t2mi --example t2mi_dump -- <file.ts> [--pid 0xNNN|raw]
```

Formatting is rustfmt-clean and CI-gated (`cargo fmt --all --check`). The deliberately column-aligned enums (`TableId`, `DescriptorTag`) carry `#[rustfmt::skip]` — keep the attribute (and the alignment) when editing them, and use the same pattern for any new aligned table. Cargo.toml manifests keep their manual column alignment (rustfmt doesn't touch them).

Docs are warning-clean and CI-gated (`RUSTDOCFLAGS="-D warnings"`). Bit-range notation in doc comments must be backticked — `` `[7:4]` `` — or rustdoc parses it as an intra-doc link.

## Workflow: GitHub issues drive the work

Work in this repo is tracked as GitHub issues and lands via PRs to `main`. Use the `gh` CLI.

1. **Pick up work from an issue.** `gh issue list` to see open work; `gh issue view <n>` for the spec/acceptance criteria. If you're asked to do something non-trivial that has no issue, create one first (`gh issue create`) so the work is tracked.
2. **Branch per issue** off `main`, named for the work (e.g. `complete-descriptors`, `fix-tot-crc`).
3. **Commit style** follows the existing history: `feat(carousel): …`, `fix(text): …`, `docs(dvb-si): …`, or a plain scoped summary. Imperative, specific, references the spec section when relevant.
4. **Open a PR** with `gh pr create`, body referencing the issue (`Closes #n`). CI must pass before merge:
   - test matrix on stable **and** 1.75 (MSRV) — all-features and no-default-features builds
   - `cargo fmt --all --check`
   - clippy `-D warnings` on all targets
   - doc build with `RUSTDOCFLAGS="-D warnings"`
5. **Releases are tag-driven and CI-only.** Bump all four crate versions together, merge, then push a `v<version>` tag — `release.yml` gates (tests, clippy, tag==version check) and publishes to crates.io in dependency order (dvb-common first). **Never `cargo publish` from a workstation.**

## Workflow: the delegated-engineering loop

Token-heavy authoring is delegated to DeepSeek (via the `delegate` skill → headless `opencode`); Claude stays the orchestrator, auditor, and release engineer. **Claude never marks a story done on the delegate's say-so** — only on its own fresh gate evidence.

**Claude owns (does NOT delegate):** story ordering (by dependency then value-for-effort), version semantics (patch = fixes only, minor = additive API, major = breaking; lockstep across all four crates), release bundling (batch related additive stories into one minor; ship breaking/urgent work standalone), and the correctness of every CHANGELOG, `docs/release-notes/vX.Y.Z.md`, README coverage table, module spec-citation, and example/doctest.

**Per-story loop:**

1. **Scope** — read the issue's acceptance criteria and the cited `docs/` transcription. Resolve any design ambiguity *before* delegating (the delegate sees only the brief, none of this context).
2. **Baseline** — branch `story/<n>-<slug>` off `main`; commit any in-flight working state first, so the delegate's `git diff` is cleanly attributable.
3. **Delegate** — write a self-contained brief (exact files, decided behaviour/signatures, the project conventions that apply, the exact gate commands, and "fix until all pass before finishing"; boundaries: touch only <scope>, do not commit). Run in the background.
4. **Audit** — judge by `git diff` + running the **full gate suite yourself** (see Commands), never by the delegate's stdout (often empty on success) or its claims. Then check line-by-line against every AC and the hard invariants (symmetric serialize + round-trip test, no magic numbers outside `#[cfg(test)]`, spec citation in the module doc, `--no-default-features` builds, feature-gating). If a delegated test doesn't *bite*, reject or rewrite it — Claude owns verification.
5. **Drive fixes** — feed concrete findings back via `opencode run --continue` (same session keeps context). After 2 failed fix cycles on the same point, take over and finish it directly.
6. **Repeat 4–5** until every gate is green *and* every AC is met, on Claude's own run.
7. **Ship** — update CHANGELOG/release-note/README/examples; branch→PR (`Closes #n`)→CI green→merge; then the lockstep version bump + `v<version>` tag (per the tag-driven release rule above). Verify all four crates went live.

**Continuous improvement:** treat this loop as living. When a brief pattern, gate ordering, or audit check repeatedly saves (or costs) time, refine this section and say so in the turn. Recurring delegate failure modes belong in the brief template, not rediscovered each story.

## Architecture

### The Parse/Serialize contract (dvb-common/src/traits.rs)

Every wire structure in every crate implements the same symmetric pair:

- `Parse<'a>` — `parse(&'a [u8]) -> Result<Self>`, borrowing from the input (zero-copy: parsed structs hold `&'a [u8]` slices and carry `<'a>` lifetimes).
- `Serialize` — `serialized_len()` + `serialize_into(&mut [u8])`.

Every parser has a symmetric serializer and a **round-trip test** (parse → serialize → byte-identical, and serialize → parse → equal). This symmetry is a hard project invariant.

### dvb-si layout

- `tables/` — one file per table (pat, pmt, sdt, eit, nit, …). Tables expose typed header fields; descriptor loops are borrowed `&[u8]` slices the caller walks with the descriptor parsers.
- `descriptors/` — one file per descriptor tag. Each module exports a `TAG` const, length consts, a `XxxDescriptor<'a>` struct, and the Parse/Serialize impls. `descriptors/any.rs` defines `AnyDescriptor` + `parse_loop` (the lazy descriptor-loop walker); `descriptors/registry.rs` adds `DescriptorRegistry` for private tags.
- `carousel/` — DSM-CC DSI/DII/DDB messages + `ModuleReassembler`, layered on `tables/dsmcc.rs` section framing.
- `text/` — EN 300 468 Annex A string decoding. `DvbText<'a>` wraps raw wire bytes and decodes on demand (`.decode()`/`Display`/serde); `LangCode` is the 3-byte language/country newtype. Serde serializes both as decoded strings; `DvbText`-bearing structs are serialize-only.
- `demux.rs` (feature `ts`) — `SiDemux`: PID-filtered, version-gated, PAT-following section pump. Feed 188-byte TS packets, get a `SectionEvent` per *changed* section; `event.table_section()` gives an `AnyTableSection`. `section.rs`/`ts.rs` provide the underlying TS packet handling and `SectionReassembler`.
- Features: `chrono` (MJD+BCD → `DateTime<Utc>`), `ts`, `serde` — all on by default; everything must also build `--no-default-features`.

### Trait-driven dispatch (the `*Def` trait + `declare_*!` macro pattern)

Each crate's unified dispatch enum — `dvb_si` `AnyTableSection`/`AnyDescriptor`,
`dvb_t2mi` `AnyPayload` — is generated from a single declarative list (the
`declare_tables!` / `declare_descriptors!` / `declare_payloads!` macro). One line
per type produces the enum variant, the `From<T>` impl, the dispatcher arm, and a
**drift test** that pins each table_id/tag/packet_type literal to the type's
`TableDef`/`DescriptorDef`/`PayloadDef` trait const (`TABLE_ID_RANGES`/`TAG`/
`PACKET_TYPE` + a SCREAMING_SNAKE `NAME`). The list is the single source of truth,
so the dispatcher can never silently drift from the implemented set. To add a
type: implement the module + the `*Def` trait, then add one line to the macro
invocation — the integration completeness test walks the generated set
automatically.

The runnable debug CLIs (`dvb-si/examples/si_dump.rs`,
`dvb-t2mi/examples/t2mi_dump.rs`) wire the pump → dispatch → decode story together
with no new dependencies (plain `std::env::args`).

### Spec grounding (the project's defining discipline)

- ETSI PDFs are vendored in `specs/`; their syntax tables are machine-extracted into reviewable markdown in `dvb-si/docs/` by `tools/dvb-si-audit/` (deterministic pdfplumber pipeline — see its README to regenerate).
- **Every layout is cited**: module doc comments name the spec, section, and tag/table_id (e.g. `//! Network Name Descriptor — ETSI EN 300 468 §6.2.28 (tag 0x40)`). When implementing or changing a layout, read the corresponding `dvb-si/docs/` transcription first and cite it.
- **No magic numbers** outside `#[cfg(test)]`: every hex literal is a named constant or enum.
- Every field in a section's syntax appears in the parsed struct (spec fidelity).
- Fixture tests (`dvb-si/tests/`) validate against real broadcast captures; round-trip and serde round-trip tests are required for new types.

### Error conventions

Structured `thiserror` errors with context: `BufferTooShort { need, have, what }`, `InvalidDescriptor { tag, reason }`, etc. Parsers validate the tag byte and length before slicing; serializers check `OutputBufferTooSmall` first. Reserved-bit policy varies by crate and is documented at the crate root (e.g. dvb-t2mi rejects non-zero RFU bits except individual addressing).

### Adding a descriptor/table (the recurring task)

Follow an existing implemented module (e.g. `descriptors/network_name.rs`) exactly: spec-cited module doc → `TAG`/length consts → borrowed struct with `#[cfg_attr(feature = "serde", …)]` (+ `serde(borrow)` on slices) → `Parse` with tag + length validation → symmetric `Serialize` → unit tests in-module + round-trip coverage. Stub modules carrying only a doc comment exist for not-yet-implemented descriptors; implementing them is the current push (`complete-descriptors` branch).
