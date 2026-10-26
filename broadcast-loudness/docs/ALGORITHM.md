The verbatim spec transcriptions are in the private repo (`private/specs-verified/loudness/broadcast-loudness/`); this is the public own-words summary of how the crate behaves.

# broadcast-loudness: how the measurement works

Everything below is read off `src/` (`filter.rs`, `loudness_meter.rs`, `true_peak.rs`, `channel_layout.rs`) and `tests/compliance.rs`. Clause references: ITU-R BS.1770-5 (Annex 1 loudness, Annex 2 true peak), EBU R 128, EBU Tech 3341 (compliance signals), EBU Tech 3342 (loudness range).

## 1. K-weighting

Each audio channel goes through a cascade of two second-order IIR sections (direct form I, `f64` state):

1. A high shelf that models the head's acoustic effect, with a gain of about +4 dB.
2. A second-order high-pass (the "RLB" curve) with a corner near 38 Hz.

The standard only tabulates coefficients for 48 kHz. `k_weighting_coeffs(sample_rate)` therefore derives them for any rate: it takes analog prototype parameters (corner frequency, Q, shelf gain), pre-warps the corner with `tan(pi * fc / fs)`, and applies the bilinear transform. The prototype constants are tuned so the 48 kHz result matches the tabulated values to floating-point epsilon (a unit test, `coeffs_at_48k_match_tabulated`, checks this). The high-pass keeps the plain `1, -2, 1` numerator and normalises only the denominator. `LoudnessMeter::new` rejects only `sample_rate == 0` (`Error::InvalidSampleRate`); 44.1, 96 and other rates are accepted.

## 2. Channel weighting and mean square

`ChannelLayout` carries the per-channel weights G_i: Mono, Stereo (all 1.0), `Surround51` (L/R/C 1.0, Ls/Rs 1.41, LFE 0.0), and `Custom { weights }`. A weight of 0.0 skips the channel entirely (no filtering). Per frame the meter adds `G_i * y_i^2` over channels, with `y_i` the K-weighted sample. Non-finite samples are rejected with `Error::NonFiniteSample` before they can poison filter state, and the meter stays usable afterwards.

Mean square converts to LUFS as `-0.691 + 10*log10(ms)`; zero or negative power gives negative infinity.

## 3. Blocks, momentary, short-term

Samples accumulate into 100 ms sub-blocks (`SUB_BLOCK_S`; `sample_rate/10` frames). The meter keeps a ring of at most the last 30 completed sub-blocks, so memory is bounded and independent of duration:

- Every time a sub-block completes and at least 4 are available, the mean of the last 4 (400 ms) becomes one gating-block value (75 % overlap, a 100 ms hop).
- Once 30 are available (3 s), the mean of all 30 becomes one short-term value every 100 ms.
- A partial trailing sub-block is discarded by `finish()`, not averaged in.

Maximum momentary and maximum short-term loudness do not come from that 100 ms grid. They use an exact per-sample sliding-window maximum (`SlidingWindowMax`) over 0.4 s and 3 s of weighted power, because the Tech 3341 cases for those maxima need the continuously slid peak.

## 4. Integrated loudness gating

`finish()` computes integrated loudness from the 400 ms gating-block values:

1. Absolute gate: drop blocks at or below -70 LUFS. If none remain the result is negative infinity.
2. Average the survivors in the power domain (convert to mean square, average, convert back).
3. Relative gate: threshold = that average - 10 LU; keep blocks above it.
4. Integrated loudness = power-domain mean of those blocks (again negative infinity if empty).

`integrated_lu()` reports the value relative to -23 LUFS.

## 5. Loudness range (Tech 3342)

LRA is computed from the 3 s short-term values (about 10 Hz), not from the 400 ms blocks (an earlier version used the wrong input and inflated the result). In words:

1. Absolute gate at -70 LUFS.
2. Power-domain mean of what remains.
3. Relative gate 20 LU below that mean (a looser gate than the integrated-loudness one).
4. Sort the survivors and take the 10th and 95th percentiles, indexing the way the Tech 3342 reference MATLAB does (`round((n-1)*p/100 + 1)`, one-based).
5. LRA = 95th percentile minus 10th percentile; 0.0 if nothing survives.

## 6. True peak

`TruePeakMeter` implements the Annex 2 approach: 4x oversampling with a polyphase FIR interpolator, then the maximum absolute value of the interpolated points. The code holds 4 phases of 12 taps (48 coefficients) and a 12-sample shift register, so memory does not grow with stream length. Because the filter is causal, `finish()` also evaluates an 11-sample zero tail so a peak just after the last real sample is not missed (`current_level()` is the cheaper running value without that tail). The 12.04 dB attenuate/restore pair from the standard is skipped because `f64` makes it cancel. Result in dBTP is `20*log10(max)`, negative infinity for silence or no input. The meter is single-channel (error indices report channel 0).

## 7. What the compliance tests cover

`tests/compliance.rs` runs generated signals against Tech 3341 Table 1:

- Loudness cases 1-6 (levels, gating with multi-tone programmes, 5.1 weighting), 9-12 (short-term and momentary maxima, live and file-based forms); case 1 is also run at 44.1 and 96 kHz.
- True-peak cases 15-19 (sines at fs/4, fs/6, fs/8 with various phases and amplitudes).
- LRA cases with synthesized tone sets, plus two programme WAV fixtures compared against an ffmpeg oracle.

Not covered: Tech 3341 loudness cases 7-8 and 13-14 have no test in this file, and true-peak cases 20-23 exist only as `#[ignore]`d stubs because they need the EBU reference WAVs.
