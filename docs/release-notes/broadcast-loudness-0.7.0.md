# broadcast-loudness 0.7.0

_Released 2026-11-08._

Breaking-class release (0.6 -> 0.7): `LoudnessMeter::new` rejects sample rates below the new `MIN_SAMPLE_RATE` (3364 Hz), and the meter's gating memory no longer grows with stream length.

Version: 0.6.0 -> 0.7.0.

## Breaking changes and migration

### New sample-rate floor

`LoudnessMeter::new` now returns `Error::InvalidSampleRate` for any rate below `MIN_SAMPLE_RATE` (3364 Hz), the smallest integer rate above twice the 1681.97 Hz K-weighting shelf corner. Below that the bilinear pre-warp `tan(pi*fc/fs)` wraps negative and the stage 1 poles leave the unit circle (radius 1.12 at 1 Hz, 2.14 at 1 kHz, 2.15 at 2 kHz), so 100, 1000 and 2000 Hz used to return NaN or infinity instead of an error (#1734). The accepted range is now `MIN_SAMPLE_RATE` to `MAX_SAMPLE_RATE` (768 kHz). Results at 44.1, 48, 96 and 192 kHz are unchanged.

```rust
// before: a 2 kHz meter was built and produced NaN
let meter = LoudnessMeter::new(2_000, layout)?;

// after: that call returns Err(Error::InvalidSampleRate); resample first
assert!(sample_rate >= broadcast_loudness::MIN_SAMPLE_RATE);
let meter = LoudnessMeter::new(sample_rate, layout)?;
```

Callers that already handled `Error::InvalidSampleRate` for rates above `MAX_SAMPLE_RATE` need no other change.

## Fixed

- `LoudnessMeter` no longer keeps one value per 100 ms block for gated integrated loudness and loudness range. That history was unbounded on a long-running stream (#1729). Blocks now accumulate into fixed-size 0.01 LU histograms with exact per-bin energy sums, so memory is constant in stream length.
- Rates in the unstable range now fail with `InvalidSampleRate` instead of returning NaN or infinity (#1734).

## Dependencies

No workspace sibling requirement changes epoch.

---

Published from tag `broadcast-loudness-v0.7.0`.
