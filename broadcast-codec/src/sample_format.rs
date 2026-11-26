//! The sample storage a decoder produces or an encoder accepts.

/// Sample storage of one audio sample.
///
/// A [`CodecDescriptor`](crate::CodecDescriptor) lists the formats an
/// implementation accepts or produces; an empty list means the provider does
/// not state them.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleFormat {
    /// Unsigned 8-bit samples.
    U8,
    /// Signed 16-bit samples.
    S16,
    /// Signed 32-bit samples.
    S32,
    /// 32-bit floating point samples.
    F32,
    /// 64-bit floating point samples.
    F64,
}

impl SampleFormat {
    /// Label of the sample format.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::U8 => "u8",
            Self::S16 => "s16",
            Self::S32 => "s32",
            Self::F32 => "f32",
            Self::F64 => "f64",
        }
    }
}

broadcast_core::impl_spec_display!(SampleFormat);
