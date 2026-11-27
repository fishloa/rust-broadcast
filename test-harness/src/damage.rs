//! Deterministic byte damage and outcome comparison for the demux damage
//! matrix.
//!
//! One implementation shared by every format's matrix rows: a [`Damage`] is
//! applied to a clean fixture image at an offset, the damaged image is
//! demuxed by the caller's own stage, and the [`DemuxOutcome`]s are compared
//! against the undamaged run. Nothing here picks a tolerance: callers pass
//! the allowed loss explicitly so the acceptable-cell list stays in the test.

use transmux::TrackId;

/// One deterministic mutation of a clean fixture image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Damage {
    /// Cut the image at the offset; every byte from it on is gone.
    Truncate,
    /// Replace `len` bytes with `0x00`.
    Zero,
    /// Replace `len` bytes with a fixed-seed pseudo-random run.
    Random,
    /// Remove `len` bytes.
    Delete,
    /// Insert `len` pseudo-random bytes.
    Insert,
    /// Overwrite the `width`-byte big-endian field at the offset with `value`.
    Field {
        /// Field width in bytes.
        width: usize,
        /// The value to write.
        value: u32,
    },
    /// Overwrite an EBML size VINT of `width` bytes with `payload` in its
    /// `7 * width` value bits and the marker bit set (RFC 8794 §4).
    VintSize {
        /// VINT width in bytes.
        width: usize,
        /// The value bits (the marker is added).
        payload: u64,
    },
}

impl Damage {
    /// `true` when the damage removes bytes from the image (so every later
    /// absolute offset shifts), rather than replacing or inserting them.
    #[must_use]
    pub fn shifts(&self) -> bool {
        matches!(self, Damage::Truncate | Damage::Delete)
    }
}

/// Apply `damage` to `clean` at `at`, returning the mutated image.
///
/// `at` is clamped into the image; a run longer than what remains is clamped
/// to the end. `seed` fixes the pseudo-random stream so a cell is
/// reproducible across runs.
#[must_use]
pub fn apply(clean: &[u8], at: usize, damage: Damage, seed: u64) -> Vec<u8> {
    let at = at.min(clean.len());
    let mut out = clean.to_vec();
    match damage {
        Damage::Truncate => out.truncate(at),
        Damage::Field { width, value } => {
            let end = at.saturating_add(width).min(out.len());
            let bytes = value.to_be_bytes();
            let src = &bytes[bytes.len().saturating_sub(width)..];
            if end > at {
                out[at..end].copy_from_slice(&src[..end - at]);
            }
        }
        Damage::VintSize { width, payload } => {
            let end = at.saturating_add(width).min(out.len());
            let marker = 1u64 << (7 * width);
            let encoded = marker | (payload & (marker - 1));
            let bytes = encoded.to_be_bytes();
            let src = &bytes[bytes.len().saturating_sub(width)..];
            if end > at {
                out[at..end].copy_from_slice(&src[..end - at]);
            }
        }
        Damage::Zero | Damage::Random | Damage::Delete | Damage::Insert => {
            let len = run_len(damage);
            let len = len.min(clean.len().saturating_sub(at).max(len));
            match damage {
                Damage::Zero => {
                    let end = at.saturating_add(len).min(out.len());
                    out[at..end].fill(0);
                }
                Damage::Random => {
                    let end = at.saturating_add(len).min(out.len());
                    fill_random(&mut out[at..end], seed);
                }
                Damage::Delete => {
                    let end = at.saturating_add(len).min(out.len());
                    out.drain(at..end);
                }
                Damage::Insert => {
                    let mut run = vec![0u8; len];
                    fill_random(&mut run, seed);
                    out.splice(at..at, run);
                }
                Damage::Truncate | Damage::Field { .. } | Damage::VintSize { .. } => {}
            }
        }
    }
    out
}

/// The fixed run length each run-shaped [`Damage`] uses.
#[must_use]
pub fn run_len(damage: Damage) -> usize {
    match damage {
        Damage::Truncate | Damage::Field { .. } | Damage::VintSize { .. } => 0,
        Damage::Zero | Damage::Random | Damage::Delete | Damage::Insert => RUN_LEN,
    }
}

/// The run length the matrix uses, in bytes.
pub const RUN_LEN: usize = 16;

/// xorshift64* — deterministic, dependency-free.
fn fill_random(bytes: &mut [u8], seed: u64) {
    let mut state = seed.max(1);
    for byte in bytes {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        *byte = (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8;
    }
}

/// One track as a demuxer delivered it: its identity, coded samples, and
/// decode timestamps.
#[derive(Debug, Clone, Default)]
pub struct TrackOutcome {
    /// The track's ID.
    pub id: Option<TrackId>,
    /// A coarse codec label used to match a track across two runs: track IDs
    /// are reassigned when a damaged run announces in a different order.
    pub kind: &'static str,
    /// The coded sample bytes, in delivery order.
    pub packets: Vec<Vec<u8>>,
    /// Each sample's decode timestamp, when it carried one.
    pub dts: Vec<Option<i64>>,
}

/// Everything one demux run delivered.
#[derive(Debug, Clone, Default)]
pub struct DemuxOutcome {
    /// The error the stage returned, formatted, if it returned one.
    pub error: Option<String>,
    /// The tracks, in the order the demuxer announced them.
    pub tracks: Vec<TrackOutcome>,
    /// How many damage findings the run reported.
    pub findings: usize,
}

impl DemuxOutcome {
    /// The total number of samples delivered across every track.
    #[must_use]
    pub fn delivered(&self) -> usize {
        self.tracks.iter().map(|t| t.packets.len()).sum()
    }

    /// Whether every delivered track's decode timestamps never step backwards.
    #[must_use]
    pub fn monotonic(&self) -> bool {
        self.tracks.iter().all(|track| {
            let mut previous: Option<i64> = None;
            for dts in track.dts.iter().flatten().copied() {
                if previous.is_some_and(|previous| dts < previous) {
                    return false;
                }
                previous = Some(dts);
            }
            true
        })
    }
}

impl DemuxOutcome {
    /// Fold a collected capture into the matrix's per-track view.
    #[must_use]
    pub fn from_collected(collected: &crate::collect::Collected) -> Self {
        Self {
            error: None,
            tracks: collected
                .tracks
                .iter()
                .map(|track| TrackOutcome {
                    id: Some(track.spec.track_id),
                    kind: codec_kind(&track.spec.config),
                    packets: track.samples.iter().map(|s| s.data.to_vec()).collect(),
                    dts: track
                        .samples
                        .iter()
                        .map(|s| s.timing.and_then(|t| t.dts()))
                        .collect(),
                })
                .collect(),
            findings: collected.degraded.len(),
        }
    }
}

/// A coarse, stable label for a track's codec configuration.
fn codec_kind(config: &transmux::CodecConfig) -> &'static str {
    match config {
        transmux::CodecConfig::Avc { .. } => "avc",
        transmux::CodecConfig::Hevc { .. } => "hevc",
        transmux::CodecConfig::Aac { .. } => "aac",
        transmux::CodecConfig::Ac3 { .. } => "ac3",
        transmux::CodecConfig::Eac3 { .. } => "eac3",
        transmux::CodecConfig::Data { .. } => "data",
        _ => "other",
    }
}

/// A damaged run measured against the undamaged one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// The damaged run's error, if it returned one.
    pub error: Option<String>,
    /// Samples the damaged run delivered.
    pub delivered: usize,
    /// Leading clean samples delivered byte-identically, summed over tracks.
    pub before: usize,
    /// Trailing clean samples delivered byte-identically, summed over tracks.
    pub after: usize,
    /// Clean tracks the damaged run never announced.
    pub dropped_tracks: usize,
    /// Samples the damaged run delivered that match no clean sample position.
    pub extra: usize,
    /// Whether every delivered track's timestamps stayed monotonic.
    pub monotonic: bool,
    /// How many damage findings the damaged run reported.
    pub findings: usize,
}

impl Summary {
    /// Clean samples neither kept before nor after the damage, plus whole
    /// tracks lost: what the damage cost.
    #[must_use]
    pub fn lost(&self, clean: &DemuxOutcome) -> usize {
        clean.delivered().saturating_sub(self.before + self.after)
    }
}

/// Measure `damaged` against `clean`, matching tracks by codec kind.
#[must_use]
pub fn compare(clean: &DemuxOutcome, damaged: &DemuxOutcome) -> Summary {
    let mut before = 0usize;
    let mut after = 0usize;
    let mut extra = 0usize;
    let mut dropped_tracks = 0usize;
    let mut used = vec![false; damaged.tracks.len()];
    for c in &clean.tracks {
        let matched = damaged
            .tracks
            .iter()
            .enumerate()
            .position(|(index, d)| !used[index] && d.kind == c.kind);
        let Some(index) = matched else {
            dropped_tracks += 1;
            continue;
        };
        used[index] = true;
        let d = &damaged.tracks[index];
        let prefix = c
            .packets
            .iter()
            .zip(d.packets.iter())
            .take_while(|(a, b)| a == b)
            .count();
        let rest_c = &c.packets[prefix.min(c.packets.len())..];
        let rest_d = &d.packets[prefix.min(d.packets.len())..];
        let suffix = rest_c
            .iter()
            .rev()
            .zip(rest_d.iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        before += prefix;
        after += suffix;
        extra += rest_d.len().saturating_sub(suffix);
    }
    for (index, d) in damaged.tracks.iter().enumerate() {
        if !used[index] {
            extra += d.packets.len();
        }
    }
    Summary {
        error: damaged.error.clone(),
        delivered: damaged.delivered(),
        before,
        after,
        dropped_tracks,
        extra,
        monotonic: damaged.monotonic(),
        findings: damaged.findings,
    }
}
