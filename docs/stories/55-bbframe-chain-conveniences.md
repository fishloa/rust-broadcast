# Story 55 — bbframe chain conveniences: `BbframePump` + decoded ISSY (#55 a, b)

Delegated brief for GitHub issue **#55** parts (a) and (b). Part (c) (GSE handoff
docs) is handled separately by the orchestrator. Self-contained. Touch only the
files listed. Do **not** commit.

## Spec sources (read FIRST — cite in code)

- **`dvb-bbframe/docs/en_302_307_1_s2.md`** — ISSY field semantics (DVB-S2).
- **`dvb-bbframe/docs/en_302_755_t2.md`** — the DVB-T2 ISSY variant + BUFS/TTO
  scaling. Cite the exact table you implement each accessor from.

## Existing code to model on (read, reuse, do not duplicate)

- **`dvb-t2mi/src/inner_ts.rs`** — `InnerTsRecovery::feed(&[u8]) -> &[[u8; 188]]`
  + `stats()`. This is the **shape** to mirror for `BbframePump` (one-call driver,
  internal owned output buffer returned by slice, a `Stats` accessor).
- **`dvb-bbframe/src/packet.rs`** — `CarryOverExtractor` (`feed_hem_into` /
  `feed_nm_into`, `stats() -> CarryOverStats`), `up_iter`, `NmTsIter`/`HemTsIter`.
  `BbframePump` composes these — it does **not** reimplement carry-over.
- **`dvb-bbframe/src/header.rs`** — `Bbheader::parse`, `Mode` (NM/HEM), `Matype`.
- **`dvb-bbframe/src/issy.rs`** — `Issy` enum, `BufsUnit`, `SignallingKind`,
  `decode_issy_short`/`decode_issy_long`.

## (a) `BbframePump` adaptor — `dvb-bbframe/src/packet.rs` (or a new `pump.rs`)

Package the BBFrame→inner-TS chain that consumers currently hand-write. **Home:
`dvb-bbframe`, no `dvb-t2mi` dependency** — it takes already-unwrapped BBFrame
data-field bytes, so the crates stay decoupled.

```rust
pub struct BbframePump { /* per-PLP carry-over state + output buffer + stats */ }

impl BbframePump {
    pub fn new() -> Self;
    /// Feed one PLP's BBFrame data-field bytes (`df_bytes` = BBHEADER + data
    /// field, as `dvb_t2mi::AnyPayload::Bbframe`'s `bbframe` field yields).
    /// Parses the BBHEADER, detects NM vs HEM, runs carry-over extraction for
    /// THIS plp_id, and returns the inner 188-byte TS packets completed by this
    /// frame. Per-PLP state is keyed by `plp_id` so interleaved PLPs don't
    /// corrupt each other's carry-over.
    pub fn feed(&mut self, plp_id: u8, df_bytes: &[u8]) -> &[[u8; 188]];
    pub fn stats(&self) -> BbframePumpStats;
}
```

- Per-PLP state: a map (or small Vec) of `plp_id -> CarryOverExtractor`. A new
  `plp_id` lazily creates one.
- `feed` is **infallible** (mirrors `InnerTsRecovery`): a malformed/short
  `df_bytes`, a bad BBHEADER, or a GSE/`TsGs`-non-TS MATYPE emits no packets and
  bumps a stat counter — never panics, never errors out.
- `BbframePumpStats` (`#[non_exhaustive]`, mirror `CarryOverStats`/t2mi `Stats`):
  counters for `header_parse_failures`, `non_ts_payloads` (GSE/generic), and the
  forwarded `CarryOverStats` totals (or a summary). Keep names consistent with
  the existing stats structs.
- Reuse `Bbheader::parse` + `up_iter`/`CarryOverExtractor::feed_*_into` — the
  pump is orchestration over them, not new framing logic.

### (a) tests
- Move the chain glue: `dvb-t2mi/tests/chain.rs` should drive the pipeline
  through `BbframePump` instead of hand-calling `Bbheader::parse` + `up_iter`,
  and still assert the full `T2miPump → BbframePump → SiDemux → PatSection`
  result. (You MAY edit `dvb-t2mi/tests/chain.rs` for this one purpose — it is
  the AC. dvb-t2mi already has dvb-bbframe as a dev/optional dep.)
- Unit tests in dvb-bbframe: NM and HEM frames each yield the right inner packets;
  two interleaved `plp_id`s keep independent carry-over; a malformed `df_bytes`
  bumps a stat and yields nothing (no panic).

## (b) Decoded ISSY accessors — `dvb-bbframe/src/issy.rs`

Follow the 4.1.0 decoded-accessor pattern (see `dvb-si/src/tables/eit.rs` +
`dvb-si/tests/accessors.rs` for the shape: raw field stays, decoded accessor
added, symmetric `set_*` encoder, round-trip test with hand-computed values).

On the `Issy::Signalling` (BUFS/TTO) data add:
- `bufs_bytes(&self) -> u32` — BUFS decoded to **bytes** applying the
  `BufsUnit` scaling (the unit multiplier from the spec table).
- `tto_elementary_periods(&self) -> u32` (or the spec's unit) — TTO in
  elementary-period units with the documented scaling.
- `set_bufs_bytes` / `set_tto_*` encoders that round-trip
  (`decode → set → encode → decode` equal; `to_u8`/byte round-trip preserved).
- Each accessor's doc cites the exact `docs/en_302_307_1_s2.md` /
  `docs/en_302_755_t2.md` table and clause. **Hand-computed scaling shown in a
  comment in the test** (no magic numbers outside `#[cfg(test)]`).

## CHANGELOG (required — keep it current as we go)

Add bullets under **`dvb-bbframe/CHANGELOG.md` → `## Unreleased`** (create the
section if absent, above the latest version): `### Added` for `BbframePump` +
`BbframePumpStats` and the decoded ISSY accessors, each with the #55 reference
and a spec cite. Match the existing changelog bullet style.

## Constraints

- No magic numbers outside `#[cfg(test)]` (named consts, spec-cited).
- `#[non_exhaustive]` on the new stats struct; `#[cfg_attr(feature = "serde", …)]`
  on any new public data type matching the crate's existing derive pattern.
- Symmetric serialize + round-trip for the ISSY encoders (hard project invariant).
- MSRV 1.75; `--locked`; `--no-default-features` clean; no new runtime deps.

## Gate commands (run ALL; fix until every one is green before finishing)

```
cargo build  --workspace --all-features --locked
cargo test   --workspace --all-features --locked
cargo build  --workspace --no-default-features --locked
cargo clippy --workspace --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
```

## Boundaries

- Touch only: `dvb-bbframe/src/packet.rs` (or new `dvb-bbframe/src/pump.rs` +
  its `lib.rs` mod line), `dvb-bbframe/src/issy.rs`, `dvb-bbframe/CHANGELOG.md`,
  and `dvb-t2mi/tests/chain.rs` (for the (a) AC only).
- Do **not** add a `dvb-t2mi` dependency to `dvb-bbframe`.
- Do **not** implement part (c) (GSE docs). Do **not** commit.
