# Story 55b — decoded ISSY accessors (dvb-bbframe)

GitHub issue **#55(b)**. The raw ISSY fields are already decoded
(`SignallingKind::Bufs { bufs, units }` / `Tto { tto_e, tto_m, tto_l }` in
`dvb-bbframe/src/issy.rs`). This story adds the **physical-unit decode accessors**
on top — no wire/parse changes. Touch only `dvb-bbframe/src/issy.rs` and
`dvb-bbframe/CHANGELOG.md`. Run the full gate suite; fix until green. Do **not** commit.

## Source of truth (verbatim, already in-repo)
`dvb-bbframe/docs/en_302_755_t2.md` → "Annex C — ISSY field coding" (Table C.1 +
the §8.3 BUFS/TTO semantics), reconstructed verbatim from the EN 302 755 PDF. Use
it; do not invent. Key facts from it:
- BUFS unit `[3:2]`: `00`=bits, `01`=Kbits, `10`=Mbits, `11`=8Kbits. The standard
  names the units but does not numerically define K/M; the decimal reading
  (K=1 000, M=1 000 000, 8K=8 000) is taken (documented in the transcription).
- `TTO = (TTO_M + TTO_L/256) × 2^TTO_E`, in units of the elementary period **T**
  (TTO_L = 0 when ISCRshort is used).

## Add to `dvb-bbframe/src/issy.rs`

1. `impl BufsUnit { pub fn multiplier_bits(self) -> u64 }` — `Bits=>1`,
   `Kbits=>1_000`, `Mbits=>1_000_000`, `Kbits8=>8_000`. Doc-cite Table C.1 + the
   decimal-convention note.
2. `impl SignallingKind`:
   - `pub fn bufs_bits(&self) -> Option<u64>` — `Some(bufs as u64 * units.multiplier_bits())` for the `Bufs` variant, else `None`.
   - `pub fn bufs_bytes(&self) -> Option<u64>` — `bufs_bits().map(|b| b / 8)` (documented: integer floor; BUFS is a buffer-size bound).
   - `pub fn tto_t_over_256(&self) -> Option<u64>` — exact, integer: `Some(((tto_m as u64) * 256 + tto_l as u64) << tto_e)` for the `Tto` variant, else `None`. Doc: this is `TTO × 256` in units of the elementary period T (i.e. units of T/256), so the fractional `TTO_L/256` term is preserved exactly without floating point. Consumers divide by 256.0 for T.
   - Keep these accessors `#[must_use]`, doc-cited to `docs/en_302_755_t2.md` Annex C / EN 302 755 §8.3.
   - (No `set_*` encoders: the physical→mantissa/exponent TTO encoding is lossy, and the wire round-trip is already guaranteed by the existing raw-field serialize. State this in a short doc note on the accessors. Do not add lossy encoders.)

## Tests (in-module, hand-computed values shown in comments)
- `bufs_bits`/`bufs_bytes`: e.g. `Bufs { bufs: 2, units: Mbits }` → `2_000_000` bits → `250_000` bytes; a `Bits`-unit and a `Kbits8`-unit case; and `None` for a `Tto` variant.
- `tto_t_over_256`: e.g. `Tto { tto_e: 0, tto_m: 1, tto_l: 0 }` → `256` (= 1·T); `Tto { tto_e: 1, tto_m: 0, tto_l: 128 }` → `((0*256+128)<<1) = 256` (= 0.5·T·2 = 1·T → 256 in T/256 units); a `None` for a `Bufs` variant. Comment each hand-computation.

## CHANGELOG
`dvb-bbframe/CHANGELOG.md` `## [Unreleased]` → `### Added`: decoded ISSY
accessors `BufsUnit::multiplier_bits`, `SignallingKind::bufs_bits/bufs_bytes/
tto_t_over_256` (#55, EN 302 755 Annex C).

## Gate commands (run ALL; fix until green)
```
cargo build  --workspace --all-features --locked
cargo test   --workspace --all-features --locked
cargo build  --workspace --no-default-features --locked
cargo clippy --workspace --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
```

## Boundaries
Touch only `dvb-bbframe/src/issy.rs` and `dvb-bbframe/CHANGELOG.md`. No wire/parse
changes. No `set_*` encoders. Do not commit.
