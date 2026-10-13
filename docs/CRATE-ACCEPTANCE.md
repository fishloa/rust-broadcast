# Crate acceptance standard (every crates.io publish)

The consolidated hard bar **every crates.io publish** clears -- not just new crates. A **new crate** clears the entire bar; a **new version** of an existing crate keeps the whole crate compliant and holds any newly-added wire type to the same invariants. Gated per-release via [`RELEASE-AUDIT.md`](RELEASE-AUDIT.md).
Companion to [`DESCRIPTOR-ACCEPTANCE.md`](DESCRIPTOR-ACCEPTANCE.md) (descriptor-level),
[`RELEASE-DOCS.md`](RELEASE-DOCS.md) (per-release docs) and [`RELEASE-AUDIT.md`](RELEASE-AUDIT.md)
(pre-tag battery). Where those overlap, this doc is the index; they hold the detail.

> **This is a living document.** As we learn how to build these crates well, tighten it —
> every recurring failure mode becomes a line here so it is gated, not rediscovered. Improve
> code, docs, and quality all round, continuously. (CLAUDE.md "continuous improvement".)

## 1. Wire contract — the non-negotiable invariants

### Binary wire formats
- **Symmetric `Parse` + `Serialize`** for every wire structure, with a **round-trip test that is byte-identical** — `parse(real_bytes) → serialize → ==` the input bytes, INCLUDING stuffing/padding/reserved bits. "Semantic" round-trip (re-parse → fields equal) is **NOT sufficient** — it is passed by garbage and by raw-passthrough serializers.
- **No `self.raw` passthrough** in serialize: lengths/offsets are COMPUTED from fields, never stored-and-echoed. `grep self.raw` inside serialize must be empty.

### Text/markup formats (XML, M3U8, MPD, and similar)
Byte-identical round-trip for text formats **requires** storing and re-emitting source spans (comments, attribute order, namespace prefix spelling, whitespace, empty-element form) — which IS the `self.raw` passthrough the binary rule bans. The anti-cheat property is therefore enforced directly instead of via a proxy the format cannot honestly satisfy:
- **No source passthrough**: the serializer builds output **solely from the typed model**. `grep` for stored-source echo must be empty.
- **`parse → serialize → re-parse` must yield an equal document** (fields compare equal; structural fidelity).
- **Mutating any field must change the output** — this is what kills a passthrough serializer, since an echo cannot reflect a mutation.
- **The crate's README must enumerate every way its output may differ from its input** (comments, attribute ordering, prefix spelling, whitespace, empty-element form). Byte-identity is not required, and a crate must not fake it by storing raw spans.

### Reserved-bit / trailing-byte policy (workspace-wide; issue #1139)
One rule for every wire crate; the authoritative statement is the `broadcast-common` crate-level docs ("Reserved-bit and trailing-byte policy").
- **Parse is tolerant.** A non-conformant reserved bit or a trailing byte that real senders emit is accepted, never an error (stricter validation is a documented per-crate choice, e.g. `dvb-t2mi` rejects non-zero RFU).
- **Serialize writes the spec value** for reserved bits: `1` for `reserved_future_use` / H.222.0 "reserved", `0` for `reserved_zero_future_use`. So re-serializing a stream with non-standard reserved bits **normalizes** them and is deliberately not byte-identical (`dvb-si`, `mpeg-pes`, `transmux`).
- **Exception: preserve-on-parse.** A crate whose structs carry the reserved/trailing bytes as explicit typed fields re-emits them verbatim (`mpeg-ps`, `rmt-flute`, `cc-data` fixed bits, `rtcp-packet` `profile_extensions`, `mp4-emsg` flags). Such a crate's real-fixture round-trip must be byte-exact including those bits.
- A caller that needs byte-for-byte preservation of arbitrary reserved bits **keeps the original bytes**.
- **Trailing bytes inside a descriptor/section body are never silently dropped without a documented decision**: keep them in a typed field, or document the field/struct as "ignored".
- **Reserved bytes are always written explicitly** by `serialize_into`, never skipped on the assumption of a pre-zeroed buffer.

### Shared (both format classes)
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
- Parser crate: **parse + round-trip the real fixture** (byte-exact for binary; equal re-parse for text/markup).
- Transform/repair/mux crate: **fault-inject → operate → assert-known-good** on real data (oracle = the clean original or a third-party reference). Each operation gets its own biting test.
- **Tests must BITE**: stub the impl to a no-op and the test must FAIL. A green suite that a wrong impl also passes is not a gate.
- **Fixtures must be genuine**: unscrambled, structurally valid for the field under test. (Session lesson: scrambled packets misparse as plausible typed fields under loose assertions — byte-identical + provenance checks catch it.)
- Fixtures read at runtime via `concat!(env!("CARGO_MANIFEST_DIR"), …)` + `std::fs` — NEVER a bare relative path (silently skips under cargo's CWD), NEVER `include_bytes!` for example fixtures (breaks publish/docs.rs), NEVER a skip-on-missing branch (a missing fixture must panic, or the gate vacuously passes).

### Test-integrity hard gates (issue #1131 -- "tests that cannot fail")
Each of these is a gate, not advice; a crate or new wire type that misses one is not accepted.
- **Byte-exact against the ORIGINAL fixture slice.** The round-trip gate compares the serializer output to the bytes sliced from the committed real fixture. Never compare parsed ASTs to each other, and never compare against re-canonicalised bytes (both pass when parse and serialize are wrong in the same way; #1045, #1047). A parse failure that degrades to a raw `Unknown` passthrough must fail the gate, not satisfy it.
- **An independent oracle per protocol/format.** Expected values must come from something that is not this crate's own encoder/serializer: libsrt (SRT), ffmpeg / GPAC `MP4Box` (ISO BMFF, CMAF, TS), Apple `mediastreamvalidator` (HLS), libebur128 / the EBU Tech 3341 + 3342 files (loudness), libzvbi (teletext/VBI), TSDuck (TS/SI), spec worked examples. A test whose only expected values are produced by the crate's own serializer (self-vs-self, a mock that returns 0, a synthetic fixture made by the crate's encoder) does **not** count toward the gate. State the oracle in the crate README/test module doc; if none exists, record the gap.
- **One real or oracle-produced fixture per spec branch**: each box/section version (v0 and v1), each flag combination, each length-form (e.g. BER short/long), count 0 and count >= 2. A branch with no real fixture is untested.
- **>= 2 entities wherever a uniqueness or cross-entity invariant exists** (two PLPs, two services, two tracks, two PIDs, two sessions). Single-entity fixtures hide cross-entity bugs.
- **Sibling-grep on every fix.** A fix PR greps the whole workspace for sibling copies of the same bug class, fixes them or files issues, and adds a regression test per sibling; the PR states the grep used. Record the class in the "Bug classes seen" table in [`RELEASE-AUDIT.md`](RELEASE-AUDIT.md) section 3.6.

## 4. The 6-gate CI suite (run by the orchestrator, not on a delegate's say-so)
`cargo build --workspace --all-features --locked` · `cargo test --workspace --all-features --locked` ·
`cargo build --workspace --no-default-features --locked` · `cargo clippy --workspace --all-features --all-targets --locked -- -D warnings` ·
`cargo fmt --all --check` · `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked`.
Plus **MSRV 1.95.0** (`rust-toolchain.toml`, single source) and, for `no_std` crates, a **bare-metal `thumbv7em-none-eabi`** build.

## 5. `no_std`, labels, fuzz, examples, CLI
- **`no_std` + `alloc`** where practical; `--no-default-features` + bare-metal build green.
- **#204 label convention**: every public spec/field enum gets `name() -> &'static str` (hand-written) + `broadcast_common::impl_spec_display!`; a per-crate `tests/label_coverage.rs` drift-guard (or a documented SKIP list).
- **No-hand-roll tripwire** (protocol/runtime crate): a lexical `tests/no_handroll_guard.rs` (or `tests/dehandroll_guard.rs`) scanning `src/**/*.rs` outside `#[cfg(test)] mod` bodies for the de-hand-roll spec **§5** pattern list (`"HTTP/1.`, `"\r\n\r\n"`, `find("://")`, `strip_prefix("<scheme>://")`, SDP `"a=`/`"m=`/`"v=0` building, `civil_from_days`/`days_from_civil`, a base64 alphabet literal, `thread::sleep`/`sleep(` in non-test async code), with a **reasoned allowlist**. Each allow entry names the file, pins the exact source line (so a new occurrence of the same needle elsewhere still fails) and gives the reason; a genuinely-infeasible site is a **documented exception** in the de-hand-roll spec §9, not a silent allow. The module doc states it is a lexical tripwire and that review is the real control.
- **Fuzz target** added (the workspace fuzzes every parser crate; nightly fuzz-build gate).
- **≥2 runnable examples** (`cargo run -p <crate> --example …`), fixtures via `std::fs` at runtime.
- **CLI** (if any) follows [`CLI-STANDARD.md`](CLI-STANDARD.md): clap derive, named flags, no positional magic numbers, auto `--help`/`--version`.

## 6. Docs & release ([`RELEASE-DOCS.md`](RELEASE-DOCS.md))
CHANGELOG (Keep-a-Changelog) · `docs/release-notes/<ver>.md` · README (badges, install, quickstart, spec cite, license) · crate-root `//!` with spec citation + embedded example list · `[package.metadata.docs.rs]` (all-features + `doc(cfg)`) · GitHub Release. **Docs are updated in the same PR as the change**, not batched.

## 7. Versioning & releases
Independent crates version on their own cadence; lockstep crates move together. Dep-ref consistency verified against crates.io ground truth (the partial-publish trap). **Tag-driven, CI-only publish — never `cargo publish` from a workstation.** No publish without explicit owner sign-off.

---
### Acceptance checklist (tick before any publish -- new crate or new version)
- [ ] Symmetric parse/serialize + round-trip test (**byte-identical** for binary; **equal re-parse + mutation-bites** for text/markup)
- [ ] No `self.raw` passthrough · no raw-byte public API · no magic numbers · `#[non_exhaustive]`
- [ ] Spec-cited modules; sources verified (no fabrication; pdf2md exit 0 / cross-checked)
- [ ] Round-trip is **byte-exact against the original fixture slice**; an **independent oracle** is named; a fixture per spec branch and >= 2 entities for uniqueness invariants (section 3 test-integrity gates)
- [ ] Reserved-bit / trailing-byte policy followed (section 1): tolerant parse, spec-value serialize or documented preserve-on-parse, explicit reserved writes
- [ ] Committed **real fixture**; per-op **biting** test (fault-inject→assert-known-good for transforms)
- [ ] 6-gate CI suite + MSRV 1.95.0 (+ thumbv7em if no_std) — run by the orchestrator
- [ ] #204 labels + label_coverage · fuzz target · ≥2 examples · CLI-STANDARD (if CLI)
- [ ] No-hand-roll tripwire (`tests/*guard*.rs`) with a reasoned, line-pinned allowlist (protocol/runtime crate)
- [ ] RELEASE-DOCS complete (CHANGELOG/release-note/README/crate-root/docs.rs metadata)
- [ ] Versioning + dep-refs consistent; publish staged for owner sign-off
