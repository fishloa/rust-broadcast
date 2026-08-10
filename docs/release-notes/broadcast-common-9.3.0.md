# broadcast-common 9.3.0

**Release date:** 2026-08-10

Two new consolidation modules, no breaking changes. `broadcast-common` is the fan-in crate roughly three dozen others in this workspace depend on, so both additions replace hand-rolled logic that previously existed as multiple independent, disagreeing copies scattered across the dependents — a consumer that adopts either module retires its own copy and inherits one, tested, spec-cited implementation instead.

## What changed

- **`ts_dup`** — the ITU-T H.222.0 (08/2023) / ISO/IEC 13818-1 §2.4.3.3 "legal duplicate transport-stream-packet" check: `is_legal_duplicate_pair` (byte-for-byte identity between two raw TS packets, with the 6-byte PCR field the sole exception, and `adaptation_field_control` required to be `01`/`11`) and `check_duplicate` (adds the spec's "two, and only two consecutive" cardinality rule via a caller-tracked `dup_already_used` flag, returning a `DuplicateVerdict`). This replaces three independent, disagreeing hand-rolled copies: `dvb-conformance`'s duplicate check (which only compared payload bytes — too lenient on the PCR-exception rule) and `ts-fix`'s own duplicate-detection hashes. A consumer gets the full three-part spec rule (byte identity minus the PCR exception, AFC gating, and run-length cardinality) in one call instead of re-deriving it. `no_std`, no allocation.
- **`clock33`** — generic 33-bit wrapping-clock helpers: `unwrap_delta` (bidirectional wrap-corrected unroll of a repeating 90 kHz counter into an ever-growing accumulator) and `wrapping_forward_distance` (stateless modular forward-distance for a wrap-vs-past comparison), plus the `WRAP_33BIT`/`WRAP_33BIT_HALF` constants. This consolidates four independent hand-rolled copies of the same math (`timed-metadata::Timeline`, `transmux::ts_demux`, `media-doctor::pts_check`, `compliance-probe::scte35`). It also fixes a latent correctness gap the consolidation surfaced: the previous forward-only epoch-counter approach in `timed-metadata` misclassified a small backward reorder straddling the wrap origin as a huge forward jump; `unwrap_delta`'s bidirectional correction gets this case right. A consumer adopting `clock33` gets that fix along with the shared implementation.
- MSRV raised to **1.95.0** (issue #949) — see below. No functional or API change from this alone.

## Migration

No API changes; no action required to keep building against this crate. Adopting `clock33`/`ts_dup` in place of a local copy is optional and non-breaking — both are additive modules.

### MSRV

This crate now requires **Rust 1.95.0**, workspace-wide (issue #949). If you build with an older toolchain, bump it.
