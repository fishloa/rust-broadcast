# New descriptor / table acceptance gate

Every new descriptor, table, or coded enum must satisfy these invariants so it is
**consistent with the existing set** and cannot become a future breaking change.
Each item is tagged with how it is enforced:

- **[CI]** — a test fails if violated (automatic; runs in the gate suite).
- **[AUDIT]** — checked by `tests/convention_audit.rs` (mechanical scan).
- **[REVIEW]** — human review against the spec (cannot be mechanised).

## 1. Structure & symmetry
- [REVIEW] Module doc cites the spec: standard, section §, table number, tag/table_id.
  Also mechanically required by **[AUDIT]** (a spec-citation line must be present).
- [REVIEW] Every field in the spec syntax table appears in the parsed struct.
- [CI] Symmetric `Parse` + `Serialize`; **[CI]** byte-identical round-trip
  (`round_trip.rs`, `serde_round_trip.rs`, `yoke.rs`).
- [AUDIT] An **in-module** round-trip test exists (per the project convention),
  exercising each conditional branch where the layout is conditional.
- [REVIEW] No magic numbers: every hex literal outside `#[cfg(test)]` is a named
  const, enum discriminant, or an idiomatic bit-mask. (clippy + review.)

## 2. Dispatch & drift
- [CI] Wired into the generating macro (`declare_descriptors!` /
  `declare_extension_bodies!` / `declare_tables!`); the drift test pins the tag
  literal to `DescriptorDef::TAG` (or `ExtensionBodyDef::TAG_EXTENSION` /
  `TableDef`) and `NAME` (SCREAMING_SNAKE).
- [CI] New coded enum has a co-located `docs/enums/<spec>/<name>.toml` drift-guard;
  `spec_drift.rs` keeps toml ↔ enum in lockstep (`*_toml_matches_enum`). Fixed-count
  guards (DescriptorTag, ExtensionTag, …) updated.

## 3. Forward-compatibility (no future breaking change) — MUST be pre-publish
- [CI] Every public **enum** is `#[non_exhaustive]` (`non_exhaustive_coverage.rs`)
  AND has a `Reserved`/`Unknown` byte-bearing catch-all that round-trips unknown
  values — so a spec adding a value is additive, never breaking.
- [REVIEW] Public **structs** stay bare/constructable (matches the existing set —
  downstream builds them for the serialize path); do NOT add `#[non_exhaustive]`
  to a descriptor struct unless the spec genuinely reserves growth there.
- [CI] Every public enum has `name()` + `impl_spec_display!` (`label_coverage.rs`);
  data-carrying ADTs go in its SKIP list.

## 4. Typing (minimise untyped surface)
- [REVIEW] No public raw-byte / bare-integer field where a transcribed spec table
  defines the values — use the typed enum. A bare `&[u8]` / `uN` is allowed ONLY
  when: the bytes are an **opaque sub-structure defined by another spec** (e.g.
  `InitialObjectDescriptor`, `MuxCodeTableEntry`, `si_rbsp`, private_data), or the
  value is an **external codec code** not transcribed here (e.g. `profile_idc`,
  `level_idc`, `*_profile_and_level`). Justify each in the field doc-comment.

## 5. Fixture coverage
- [REVIEW] Where the reference toolkit (**TSDuck `tstabcomp`**) can encode the
  descriptor, add it to a `tsduck-*.bin` interop fixture and assert a byte-exact
  decode (`tsduck_interop.rs`) — cross-tool validation, not just self-round-trip.
- [REVIEW] Where a real broadcast capture carries it, add a skip-if-absent
  fixture test (`downloaded_*.rs`).
- Synthetic round-trip is the floor; interop/real-capture is the goal.

## 6. Build
- [CI] Builds `--no-default-features` (`alloc::vec::Vec` imported for any `Vec`);
  clippy `-D warnings`; fmt; `RUSTDOCFLAGS=-D warnings` doc.

---
The **[AUDIT]** items run as `cargo test -p dvb-si --test convention_audit`; the
**[CI]** items run in the standard gate suite; the **[REVIEW]** items are the
pre-merge / pre-release checklist for the author and auditor.
