//! Channel layout and per-channel weighting (ITU-R BS.1770-5 §Annex 1, Table 3).
//!
//! Each layout carries the BS.1770 channel-weighting coefficients G_i.
//! The LFE channel is always excluded from measurement (weight = 0.0).

use broadcast_common::impl_spec_display;

/// Channel layout defining the set of channels to measure and their
/// BS.1770-5 per-channel weighting coefficients G_i.
///
/// See ITU-R BS.1770-5 Annex 1 Table 3.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum ChannelLayout {
    /// Mono (centre channel): 1 channel, G = 1.0.
    Mono,
    /// Stereo (left, right): 2 channels, G_L = 1.0, G_R = 1.0.
    Stereo,
    /// 5.1 surround (L, R, C, LFE, Ls, Rs): 6 channels, LFE excluded.
    /// G_L=1.0, G_R=1.0, G_C=1.0, G_Ls=1.41, G_Rs=1.41, G_LFE=0.0.
    Surround51,
    /// Custom channel count with explicit per-channel weights.
    /// `weights[i]` is G_i for channel `i`.
    Custom {
        /// Per-channel weighting coefficients (length = channel count).
        weights: &'static [f64],
    },
}

impl ChannelLayout {
    /// Number of audio channels (including LFE if present).
    #[must_use]
    pub fn channel_count(&self) -> usize {
        match self {
            Self::Mono => 1,
            Self::Stereo => 2,
            Self::Surround51 => 6,
            Self::Custom { weights } => weights.len(),
        }
    }

    /// BS.1770-5 weighting coefficient G_i for channel `index`.
    ///
    /// Returns 0.0 for LFE channels (excluded from measurement).
    #[must_use]
    pub fn weight(&self, index: usize) -> f64 {
        match self {
            Self::Mono => 1.0,
            Self::Stereo => {
                match index {
                    0 => 1.0, // L
                    1 => 1.0, // R
                    _ => 0.0,
                }
            }
            Self::Surround51 => {
                match index {
                    0 => 1.0,  // L
                    1 => 1.0,  // R
                    2 => 1.0,  // C
                    3 => 0.0,  // LFE (excluded)
                    4 => 1.41, // Ls
                    5 => 1.41, // Rs
                    _ => 0.0,
                }
            }
            Self::Custom { weights } => weights.get(index).copied().unwrap_or(0.0),
        }
    }

    /// Display name for this layout.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Mono => "Mono",
            Self::Stereo => "Stereo",
            Self::Surround51 => "5.1 Surround",
            Self::Custom { .. } => "Custom",
        }
    }
}

impl_spec_display!(ChannelLayout);
