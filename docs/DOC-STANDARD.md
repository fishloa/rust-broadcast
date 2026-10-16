# Crate documentation standard

What every crate's `README.md` and crate-root `//!` documentation must contain,
in what order, and how each claim in it is kept true. It applies to every
published crate in the workspace and to every new crate (it is part of the
[CRATE-ACCEPTANCE](CRATE-ACCEPTANCE.md) bar). [RELEASE-DOCS](RELEASE-DOCS.md)
says *when* documentation is refreshed (every release); this document says
*what* it is.

The reader is someone deciding whether this crate does what they need. The
first screen must answer: what is it, which standard and edition, how much of
it, and how do I call it.

## Rules

1. **One source.** The crate's `README.md` is the single text for crates.io
   and docs.rs. The crate root includes it:

   ```rust
   #![doc = include_str!("../README.md")]
   ```

   Use that whenever the README's code blocks compile as doctests. If they
   cannot (for example the quick start needs a fixture file), keep a short
   `//!` front page whose first paragraph and standards table match the README
   and whose quick start is a doctest, and say in a comment why the README is
   not included.
2. **Every claim is checked against the code or the spec transcription.** A
   standards row, a feature description or a "supports X" sentence is written
   by reading the module that implements it and the `docs/` transcription it
   cites, not from memory, the issue text or another README. If it cannot be
   confirmed, write **unverified**. Never round "most of" up to "full".
3. **Editions come from the vendored spec.** Cite the edition the code was
   written against, exactly as the module doc or the `docs/` transcription
   header names it (`ETSI EN 300 468 V1.20.1`, `ANSI/SCTE 35 2023r1`,
   `SMPTE ST 377-1:2019`). Do not cite a newer edition the code has not been
   checked against.
4. **No hand-typed counts where a generator can produce them.** "115
   descriptors", "all 12 packet types", "29 table ids" go stale silently. Either
   generate the table or list (rule 5) or phrase the claim so it stays true
   ("every descriptor tag in EN 300 468 V1.20.1 Table 12") and back it with a
   test that enumerates the set.
5. **Generated tables have a drift guard.** A coverage table produced by a
   script must sit between `<!-- BEGIN GENERATED: <name> (<script>) -->` and
   `<!-- END GENERATED: <name> -->` markers, and the script must have a
   `--check` mode that runs in CI or in a test. The workspace crate index
   (`tools/gen-crate-index.py --check`) is the model.
6. **Say what it does not do.** Gaps are part of the contract: unimplemented
   clauses, payloads kept opaque, features that need `std`, profiles not
   covered. A partial implementation documented as partial is fine; one that
   reads as complete is a bug.
7. **Quick start compiles.** The quick-start block is a doctest (no `ignore`,
   no `no_run` unless it does I/O, and then still compiled). Dependency lines
   such as `my-crate = "0.5"` are checked against the manifest version by
   `tools/readme_snippets.py --check`.
8. **Plain words.** Name the standard, the structure and the type. No
   marketing adjectives, no "blazing fast", no emoji.
9. **Workspace-wide facts live once.** MSRV, licence and the
   `Parse`/`Serialize` contract are stated briefly per crate and linked to the
   workspace docs rather than re-explained at length.

## Status vocabulary

Use exactly these words in the standards table (they match
[STANDARDS.md](STANDARDS.md)):

| Status | Meaning |
|---|---|
| **Full** | Every syntax structure in the stated scope parses and serializes, typed, with round-trip tests. |
| **Partial** | Some structures are implemented; the Notes column says which and which not. |
| **Typed-opaque** | The framing is typed, but a payload the standard defines is deliberately kept as bytes (encrypted payloads, codec bitstreams, private data). Say which. |
| **Label-only** | Only identifiers are known (tag values, type codes, names); no parser for the structure. |
| **Reference** | The standard supplies constants, validation rules or semantics; it is not implemented as a wire format. |
| **Not implemented** | Transcribed or planned; no code. |

"Full" always has a scope: "Full (§6 RTCP packets)" is correct; a bare "Full"
on a 300-page standard is not.

## Checklist

Copy this into the PR description when you write or rewrite a crate README.

- [ ] Title is the crate name; first paragraph is one sentence saying what it
      is and which standard it implements.
- [ ] Badges: crates.io version, docs.rs, licence, MSRV.
- [ ] **What it does / what it does not do** section, the "does not" list
      non-empty unless the crate genuinely covers its whole standard.
- [ ] **Standards supported** table with a row per standard: number and title,
      edition implemented, scope (clauses/tables), status from the vocabulary
      above, notes. Every row traced to a module doc citation or a `docs/`
      transcription.
- [ ] **Feature flags** table: every feature in `Cargo.toml`, its default, what
      it enables, and whether it needs `std`. Must match `[features]` exactly.
- [ ] **Quick start**: a compiling doctest of the most common use, plus the
      `[dependencies]` line with the current version.
- [ ] **Coverage** detail for large surfaces (descriptor lists, box lists,
      message types): generated where possible (rule 5).
- [ ] **Design notes**: zero-copy borrowing, the parse/serialize round-trip
      invariant, the reserved-bit policy, `no_std` + `alloc` status (and
      whether CI proves it: the crate is in the `no-std` job list in
      `.github/workflows/ci.yml`), the `name()`/`Display` label convention.
- [ ] **Related crates**: which workspace crates feed this one and which
      consume it (see [CRATES.md](CRATES.md)).
- [ ] **MSRV and licence** lines.
- [ ] Crate root uses `#![doc = include_str!("../README.md")]`, or a matching
      `//!` front page with the reason recorded.
- [ ] `Cargo.toml` `description` matches the README's first sentence in
      substance (it feeds the crates.io search line and
      [CRATES.md](CRATES.md)).
- [ ] The crate's row in [`crates-index.toml`](crates-index.toml) and in
      [STANDARDS.md](STANDARDS.md) agrees with the README; run
      `python3 tools/gen-crate-index.py`.
- [ ] `cargo test -p <crate> --doc --all-features --locked` and
      `RUSTDOCFLAGS="-D warnings" cargo doc -p <crate> --all-features --no-deps --locked`
      pass.

## README skeleton

Copy, then replace every `<...>`. Delete a section only if the note under it
says it may be deleted.

````markdown
# <crate-name>

[![crates.io](https://img.shields.io/crates/v/<crate-name>.svg)](https://crates.io/crates/<crate-name>)
[![docs.rs](https://img.shields.io/docsrs/<crate-name>)](https://docs.rs/<crate-name>)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#licence)
[![MSRV](https://img.shields.io/badge/MSRV-1.95.0-blue.svg)](#msrv)

<One sentence: what the crate is and which standard (with edition) it implements.>

## What it does

- <capability, naming the structure or clause>
- <capability>

## What it does not do

- <clause / structure / profile not implemented, and where to look instead if
  another crate covers it>
- <payload kept opaque, and why>

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| <ETSI EN 300 468 — DVB Service Information> | <V1.20.1> | <§5 tables, §6 descriptors, Annex A> | <Full / Partial / Typed-opaque / Label-only / Reference> | <what is and is not covered> |

Spec transcriptions: [`docs/`](docs/).

## Feature flags

| Feature | Default | Enables | Needs `std` |
|---|---|---|---|
| `std` | yes | <`std::error::Error` impls, ...> | yes |
| `serde` | <yes/no> | <`Serialize` on the public types> | no |

## Quick start

```toml
[dependencies]
<crate-name> = "<major.minor>"
```

```rust
use broadcast_common::{Parse, Serialize};
// <the most common use, end to end, asserting on a real value>
```

## Coverage

<For a large surface: a generated table between BEGIN/END GENERATED markers,
with its --check wired into CI. Delete this section for small crates whose
"What it does" list is already complete.>

## Design notes

- **Zero-copy:** parsed types borrow from the input (`&'a [u8]`).
- **Round trip:** every type implements `Parse` and `Serialize`;
  parse then serialize is byte-identical, and tests enforce it on real fixtures.
- **Reserved bits:** <policy: preserved / must-be-one checked / rejected>.
- **`no_std`:** <`#![no_std]` + `alloc` with default features off, proven in
  CI on `thumbv7em-none-eabi` / `std` only, because ...>.
- **Labels:** spec enums expose `name()` and `Display` with the spec's own
  names; reserved values render as `reserved(0xNN)`.

## Related crates

- [`<crate>`](https://crates.io/crates/<crate>) — <how it relates>

See the [workspace crate map](https://github.com/fishloa/rust-broadcast/blob/main/docs/CRATES.md).

## MSRV

Rust 1.95.0, shared by the whole workspace.

## Licence

Licensed under either of MIT or Apache-2.0, at your option.
````

## Crate-root front page (when the README cannot be included)

```rust
//! <Same first sentence as the README.>
//!
//! | Standard | Edition | Status |
//! |---|---|---|
//! | <...> | <...> | <...> |
//!
//! # Quick start
//!
//! ```
//! // <compiling example>
//! ```
//!
//! # Feature flags
//!
//! <same table as the README>
#![cfg_attr(docsrs, feature(doc_cfg))]
```

Feature-gated public items carry
`#[cfg_attr(docsrs, doc(cfg(feature = "...")))]`, and the crate's
`Cargo.toml` has `[package.metadata.docs.rs] all-features = true` so docs.rs
shows them (see [RELEASE-DOCS](RELEASE-DOCS.md)).
