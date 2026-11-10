# broadcast-common 11.0.1

_Released 2026-11-10._

Patch release: two fixes in the shared clock and sequence helpers.

Version: 11.0.0 -> 11.0.1.

## Fixed
- `clock33::unwrap_delta` resolves a step of exactly half the range forward (`+2^32`), inside its documented `(-2^32, 2^32]` and agreeing with `signed_distance`.
- `seq::SeqSpace` ordering is antisymmetric at exactly half the space: `gt`/`geq` now answer `false` there like `lt`/`leq`; new `SeqSpace::is_tie`; `diff` still reports `+half` for both orders.

## Dependencies
- None.

---

Published from tag `broadcast-common-v11.0.1`.
