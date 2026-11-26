//! One decoded frame of either kind.

use crate::audio::AudioFrame;
use crate::video::VideoFrame;

/// A decoded frame.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// An audio frame.
    Audio(AudioFrame),
    /// A video frame.
    Video(VideoFrame),
}
