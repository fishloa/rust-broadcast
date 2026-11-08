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

### `k_weighting_coeffs` returns a `Result`

`kfilter::k_weighting_coeffs` now returns `Result<(BiquadCoeffs, BiquadCoeffs)>` and reports a rate below `MIN_SAMPLE_RATE` as `Error::InvalidSampleRate`. 0.6.0 returned NaN coefficients at 0 Hz and unstable ones from 1 to 3363 Hz.

```rust
// before
let (stage1, stage2) = kfilter::k_weighting_coeffs(sample_rate);

// after
let (stage1, stage2) = kfilter::k_weighting_coeffs(sample_rate)?;
```

### `Error::SampleTooLoud`

The loudness histograms span -70 to +30 LKFS, so the pushers (`push_f32`, `push_f64`, `push_stereo_planar_f32` and the others) now reject a sample whose magnitude could carry a block to or above +30 LKFS with the new `Error::SampleTooLoud { index, channel, value, max }`, the way non-finite samples are already rejected. `LoudnessMeter::max_sample_magnitude()` reports the limit: about +19.8 dBFS for mono, +16.8 dBFS for stereo and +12.2 dBFS for 5.1, from the K-weighting filter's largest gain (an impulse-response l1 norm below 3.5 at every accepted rate) and the layout's channel weights. Louder blocks used to share the top histogram bin. `Error` is `#[non_exhaustive]`, so an existing `match` already has a wildcard arm; a caller that handles `NonFiniteSample` should handle this too:

```rust
match meter.push_f32(frame) {
    Ok(()) => {}
    Err(Error::NonFiniteSample { .. }) => skip_frame(),
    Err(Error::SampleTooLoud { max, .. }) => {
        // clip or attenuate to the limit, then push again
        let limit = max as f32;
        let clipped: Vec<f32> = frame.iter().map(|s| s.clamp(-limit, limit)).collect();
        meter.push_f32(&clipped)?;
    }
    Err(e) => return Err(e),
}
```

## Fixed

- `LoudnessMeter` no longer grows its gated-loudness history without bound on a long-running stream (#1729). Each of the two block histories (400 ms gating blocks and 3 s short-term values) keeps an exact list of up to 40 000 blocks (66 min 40 s at the 100 ms hop), preallocated; integrated loudness and loudness range from it are bit-identical to 0.6.0 (all 27 Tech 3341/3342 compliance tests and a 0.0001 dB sweep across both relative gates). The next block folds the list once into a fixed-size 0.01 LU histogram with exact per-bin energy sums, and memory is constant after that: 640 000 bytes for the two histories, plus a 960 000 byte peak during the fold.
- The loudness-range absolute gate keeps a short-term value of exactly -70 LKFS (`>=`, as the EBU Tech 3342 reference does); the histogram change had made it strict. Integrated loudness keeps the strict BS.1770-5 `>`.
- Rates in the unstable range now fail with `InvalidSampleRate` instead of returning NaN or infinity (#1734).
- The crate's unit tests are gated on `std`, so `cargo test -p broadcast-loudness --no-default-features` builds; it used to fail in test code.

## Dependencies

No workspace sibling requirement changes epoch.

---

Published from tag `broadcast-loudness-v0.7.0`.
