# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A Rust workspace of DVB (Digital Video Broadcasting) protocol parsers + builders, published to crates.io:

- **broadcast-common** — shared `Parse<'a>` / `Serialize` traits, the `mux` container-mux traits (`Unpackage`/`Package`/`Encrypt`/`Decrypt`), and CRC-32/MPEG-2. Everything else depends on it. (`dvb-common` is a deprecated re-export shim, **frozen at 8.1.0**, out of the lockstep.)
- **dvb-si** — the big one: ETSI EN 300 468 Service Information + MPEG-2 PSI. All 29 allocated table_ids, descriptors, DSM-CC data carousel, Annex A text decoding. TS packet / section reassembly lives in `mpeg-ts` (used internally via the `ts` feature).
- **mpeg-ts** — generic MPEG-2 TS framing (ITU-T H.222.0 / ISO/IEC 13818-1): TS packet, adaptation field, PCR, PSI section reassembly + packetization, resync. `no_std`. Independently versioned.
- **dvb-t2mi** — TS 102 773 T2-MI packet/payload parsing.
- **dvb-bbframe** — DVB-S2/S2X/T2 BBFrame headers, user packet extraction.
- **scte35-splice** — ANSI/SCTE 35 splice information (DPI cueing); independently versioned (left the DVB lockstep at v1.0.0). (`dvb-scte35` is a deprecated re-export shim at 7.9.1.)
- **dvb-conformance** — ETSI TR 101 290 stream conformance monitor.
- **dvb-tools** — CLI analyzer (`dump`/`services`/`epg`/`pids`/`t2mi`).
- **dvb-stream** — async/tokio stream adapters; independently versioned.
- **mpeg-pes** — PES depacketization + PTS/DTS (ISO/IEC 13818-1 §2.4.3); `no_std`, depends only on dvb-common; independently versioned. (`dvb-pes` is a deprecated re-export shim at 0.1.2.)
- **dvb-subtitle** — ETSI EN 300 743 DVB (bitmap) subtitling segments (page/region/CLUT/object/DDS/disparity + 2/4/8-bit pixel-data sub-blocks), fed the subtitle PES data field; `no_std`, depends only on dvb-common; independently versioned.
- **mpeg-ps** — MPEG-1/2 Program Stream framing (ISO/IEC 13818-1 §2.5): pack header (42-bit SCR), system header, program stream map; PES via mpeg-pes; `no_std`; independently versioned.
- **scte104** — ANSI/SCTE 104 2023 automation→compression DPI signalling: single/multiple operation messages + the full operation set; `no_std`, depends only on dvb-common; independently versioned.
- **cc-data** — DVB closed-caption carriage cc_data() (ETSI TS 101 154 Table B.9): typed CEA-608/708 triplets + 608/708 split; `no_std`, depends only on dvb-common; independently versioned. (`dvb-cc` is a deprecated re-export shim at 0.2.1.)
- **smpte2038** — SMPTE ST 2038 ancillary data in MPEG-2 TS: parse/build the PES payload carrying ANC packets (DID/SDID, user data); `no_std`, independently versioned. (`dvb-smpte2038` is a deprecated re-export shim at 0.1.1.)
- **ule** — Unidirectional Lightweight Encapsulation (RFC 4326): SNDU framing + bridged/non-bridged PDU parsing over DVB-S/T/C MPEG-2 TS; `no_std`, independently versioned. (`dvb-ule` is a deprecated re-export shim at 0.1.1.)
- **mp4-emsg** — ISO BMFF / DASH Event Message Box (`emsg`, ISO/IEC 23009-1): version 0/1 parse + serialize for inband DASH/CMAF timed events (SCTE 35 splice, ID3, ad/tracking); `no_std`, independently versioned. (`dvb-emsg` is a deprecated re-export shim at 0.1.1.)
- **timed-metadata** — Convert DPI/timed-metadata signalling between SCTE-35, HLS `EXT-X-DATERANGE` (RFC 8216 §4.4.5.1), and DASH `emsg` (SCTE 214-3); lossless round-trips, 33-bit PTS wrap-unroll via `Timeline`; `no_std`; independently versioned.
- **transmux** — any-to-any media **container** muxing hub (ISO/IEC 14496-12 / 13818-1 / 23009-1, RFC 8216/3550, MS-SSTR): demux any input (TS/fMP4/PS/WebM/FLV/RTMP) into one neutral IR (`Media`/`Track`) and mux to any output (CMAF/progressive-MP4/TS/CMAF-HLS/TS-HLS/DASH/LL-DASH/LL-HLS/Smooth); repackage, CENC decrypt, RTP/RTCP, IR transforms (PTS/DTS rebase, splice/SSAI, trick-play), fMP4/CMAF conformance validator, and a `cli`-gated `transmux` packager binary. Parses codec config headers only — never en/decodes; samples opaque. `no_std`+`alloc`; independently versioned.
- **rtsp-runtime** — sans-IO **RTSP 1.0** (RFC 2326) session engine: driveable client + server state machines (Appendix A), CSeq correlation, `Transport` negotiation, interleaved RTP/RTCP framing, Basic/Digest auth (via `http-auth`), over the `rtsp-types` + `sdp-types` codecs; optional `tokio` (+ `tls` for `rtsps://`) socket adapter. Independently versioned.
- **ts-fix** — MPEG-2 TS stream-conditioning CLI (PCR/continuity/timestamp repair); independently versioned.
- **media-doctor** — container/stream diagnostics (fMP4/CMAF/TS structural checks); independently versioned.
- **dvb-si-py** (`bindings/python/`) — PyO3/maturin Python bindings over dvb-si/dvb-t2mi: `parse_section(bytes)->dict` + `Demux`/`T2miDemux` classes (read-only, parse→serde_json→Python). NOT a workspace member (own MSRV); consumes published crates by version; abi3 wheels to PyPI via its own workflow.

MSRV is **1.86** (workspace `rust-version`); the committed `Cargo.lock` pins MSRV-compatible deps — always build/test with `--locked`.

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

# Analyzer CLI (the dvb-tools binary crate):
cargo run -p dvb-tools -- dump dvb-si/tests/fixtures/m6-single.ts
cargo run -p dvb-tools -- dump dvb-si/tests/fixtures/m6-single.ts --json
cargo run -p dvb-tools -- t2mi <file.ts> [--pid 0xNNN|raw] [--inner] [--plp N]
cargo run -p dvb-tools -- services|epg|pids <file.ts>
```

Formatting is rustfmt-clean and CI-gated (`cargo fmt --all --check`). The deliberately column-aligned enums (`TableId`, `DescriptorTag`) carry `#[rustfmt::skip]` — keep the attribute (and the alignment) when editing them, and use the same pattern for any new aligned table. Cargo.toml manifests keep their manual column alignment (rustfmt doesn't touch them).

Docs are warning-clean and CI-gated (`RUSTDOCFLAGS="-D warnings"`). Bit-range notation in doc comments must be backticked — `` `[7:4]` `` — or rustdoc parses it as an intra-doc link.

## Workflow: GitHub issues drive the work

Work in this repo is tracked as GitHub issues and lands via PRs to `main`. Use the `gh` CLI.

1. **Pick up work from an issue.** `gh issue list` to see open work; `gh issue view <n>` for the spec/acceptance criteria. If you're asked to do something non-trivial that has no issue, create one first (`gh issue create`) so the work is tracked.
2. **Branch per issue** off `main`, named for the work (e.g. `complete-descriptors`, `fix-tot-crc`).
3. **Commit style** follows the existing history: `feat(carousel): …`, `fix(text): …`, `docs(dvb-si): …`, or a plain scoped summary. Imperative, specific, references the spec section when relevant.
4. **Open a PR** with `gh pr create`, body referencing the issue (`Closes #n`). CI must pass before merge:
   - test matrix on stable **and** 1.86 (MSRV) — all-features and no-default-features builds
   - `cargo fmt --all --check`
   - clippy `-D warnings` on all targets
   - doc build with `RUSTDOCFLAGS="-D warnings"`
5. **Releases are tag-driven and CI-only.** Bump all **six** lockstep crate versions together (`broadcast-common`, `dvb-si`, `dvb-t2mi`, `dvb-bbframe`, `dvb-conformance`, `dvb-tools`; `dvb-stream`, `scte35-splice`, `mp4-emsg`, and other independent crates — including the deprecated shims like `dvb-common`, frozen at 8.1.0 — release on their own cadence or not at all), merge, then push a `v<version>` tag — `release.yml` gates (tests, clippy, tag==version check) and publishes to crates.io in dependency order (broadcast-common first). **Never `cargo publish` from a workstation.**
6. **Every release produces documentation** per [`docs/RELEASE-DOCS.md`](docs/RELEASE-DOCS.md) — the authoritative standard for the docs.rs / crates.io / GitHub surfaces. Run its **per-release checklist** each tag (CHANGELOG → release note → README coverage → crate-root `//!` → Cargo.toml + docs.rs metadata sweep → GitHub Release → post-publish verify docs.rs built green). This is enforced like the gate suite.
7. **Every release is audited** per [`docs/RELEASE-AUDIT.md`](docs/RELEASE-AUDIT.md) — the full battery of tests/checks run before a tag: the 6-gate CI suite (run yourself, not on CI/subagent say-so), the version + inter-crate dep-ref consistency audit (the v7.7.0 partial-publish trap; verify live versions against crates.io, not CHANGELOG headings), and the adversarial **extensibility/code-quality audit** (round-trip symmetry + no `self.raw` passthrough, no raw-byte public API, decode-completeness, spec-fidelity/no-magic-numbers, the #204 `name()`+`impl_spec_display!` label convention + per-crate `label_coverage` drift-guard, `#[non_exhaustive]`, panic-class safety, `declare_*`-macro dispatch). Companion to the doc standard above; [`AUDIT-LEDGER.md`](docs/AUDIT-LEDGER.md) records which PDF→md fidelity audits are already done.

## Workflow: the delegated-engineering loop

Token-heavy authoring is delegated to DeepSeek (via the `delegate` skill → headless `opencode`); Claude stays the orchestrator, auditor, and release engineer. **Claude never marks a story done on the delegate's say-so** — only on its own fresh gate evidence.

**Claude owns (does NOT delegate):** story ordering (by dependency then value-for-effort), version semantics (patch = fixes only, minor = additive API, major = breaking; lockstep across the six lockstep crates), release bundling (batch related additive stories into one minor; ship breaking/urgent work standalone), and the correctness of every CHANGELOG, `docs/release-notes/vX.Y.Z.md`, README coverage table, module spec-citation, and example/doctest.

**Per-story loop:**

1. **Scope** — read the issue's acceptance criteria and the cited `docs/` transcription. Resolve any design ambiguity *before* delegating (the delegate sees only the brief, none of this context).
2. **Baseline** — branch `story/<n>-<slug>` off `main`; commit any in-flight working state first, so the delegate's `git diff` is cleanly attributable.
   - **Prep the gate's inputs BEFORE delegating** (spec/parser work): transcribe the spec syntax tables into the *target crate's own* `docs/`, AND commit a **real fixture** (extract from an existing capture — e.g. scan committed `.ts` for the signature — pull a TSDuck stream, or use spec test vectors). Inline hand-made bytes only test the happy path; real data carries the reserved bits / mixed stream_ids / real layouts that expose bugs. No spec md + no real fixture = do not delegate yet.
3. **Delegate** — write a self-contained brief (exact files, decided behaviour/signatures, the project conventions that apply, the exact gate commands, and "fix until all pass before finishing"; boundaries: touch only <scope>, do not commit). Run in the background. The brief's exit gate must be **ungameable** and include a **real-fixture run** (parse + byte-exact round-trip the committed fixture) — a plain round-trip test is gameable by raw-passthrough serialize, and a green inline suite is not "done". Pass these gates in round ONE, never bolt them on after burning a round.
4. **Audit** — judge by `git diff` + running the **full gate suite yourself** (see Commands), never by the delegate's stdout (often empty on success) or its claims. Then check line-by-line against every AC and the hard invariants (symmetric serialize + round-trip test, no magic numbers outside `#[cfg(test)]`, spec citation in the module doc, `--no-default-features` builds, feature-gating). If a delegated test doesn't *bite*, reject or rewrite it — Claude owns verification.
5. **Drive fixes** — feed concrete findings back via `opencode run --continue` (same session keeps context). After 2 failed fix cycles on the same point, take over and finish it directly.
6. **Repeat 4–5** until every gate is green *and* every AC is met, on Claude's own run.
7. **Ship** — update CHANGELOG/release-note/README/examples; branch→PR (`Closes #n`)→CI green→merge; then the lockstep version bump + `v<version>` tag (per the tag-driven release rule above). Verify all six lockstep crates went live (plus any independent crates in the same release).

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

The runnable analyzer CLI (the `dvb-tools` binary crate — `dump` / `services` /
`epg` / `pids` / `t2mi` subcommands) wires the pump → dispatch → decode story
together. All CLIs follow the workspace **CLI standard** ([`docs/CLI-STANDARD.md`](docs/CLI-STANDARD.md)):
`clap` derive, named flags (no bare positional magic numbers), auto
`--help`/`--version`. `ci-probe` (the `dvb-ci-runtime` CAM tool) follows the same
standard.

### Spec grounding (the project's defining discipline)

- ETSI PDFs are vendored in `specs/`; their syntax tables are machine-extracted into reviewable markdown in `dvb-si/docs/` by `tools/dvb-si-audit/` (deterministic pdfplumber pipeline — see its README to regenerate).
- **Every layout is cited**: module doc comments name the spec, section, and tag/table_id (e.g. `//! Network Name Descriptor — ETSI EN 300 468 §6.2.28 (tag 0x40)`). When implementing or changing a layout, read the corresponding `dvb-si/docs/` transcription first and cite it.
- **No magic numbers** outside `#[cfg(test)]`: every hex literal is a named constant or enum.
- Every field in a section's syntax appears in the parsed struct (spec fidelity).
- Fixture tests (`dvb-si/tests/`) validate against real broadcast captures; round-trip and serde round-trip tests are required for new types.

### Error conventions

Structured `thiserror` errors with context: `BufferTooShort { need, have, what }`, `InvalidDescriptor { tag, reason }`, etc. Parsers validate the tag byte and length before slicing; serializers check `OutputBufferTooSmall` first. Reserved-bit policy varies by crate and is documented at the crate root (e.g. dvb-t2mi rejects non-zero RFU bits except individual addressing).

### Spec/field-enum label convention (every public enum — #204)

Every public spec/field enum across all crates exposes a uniform label pair:

- **`pub fn name(&self) -> &'static str`** — inherent method, hand-written
  `match` arms (labels live in source, next to the variant docs; the spec token
  for known variants, `"reserved"` for the reserved/unknown arm).
- **`Display`** — generated by `dvb_common::impl_spec_display!`, a label-free
  macro that delegates to `name()`. `impl_spec_display!(Ty)` makes `Display ==
  name()`; `impl_spec_display!(Ty, Reserved, …)` renders each named byte-bearing
  catch-all as `"{name}(0x{:02X})"` so `Display` stays lossless.

Labels are NEVER put in the macro — only in `name()`. A per-crate
`tests/label_coverage.rs` drift-guard scans `src/` and fails CI if any public
`pub enum` (minus a documented SKIP list: errors, `Any*`/tag dispatch enums,
section-kind discriminants, data-carrying ADTs) lacks a `Display`. So a **new
spec/field enum must get `name()` + `impl_spec_display!(...)`**, or be added to
that crate's SKIP list if it is genuinely not a label.

### Adding a descriptor/table (the recurring task)

Follow an existing implemented module (e.g. `descriptors/network_name.rs`) exactly: spec-cited module doc → `TAG`/length consts → borrowed struct with `#[cfg_attr(feature = "serde", …)]` (+ `serde(borrow)` on slices) → `Parse` with tag + length validation → symmetric `Serialize` → unit tests in-module + round-trip coverage. Stub modules carrying only a doc comment exist for not-yet-implemented descriptors; implementing them is the current push (`complete-descriptors` branch).
