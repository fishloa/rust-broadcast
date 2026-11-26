//! The one place this crate uses the binding's unsafe API.
//!
//! Every other module drives `ffmpeg-next` through its safe surface.

use ffmpeg_next::Frame;

/// A freshly allocated, empty libav frame for `receive_frame` to fill.
pub(crate) fn empty_frame() -> Frame {
    // SAFETY: `Frame::empty` is the binding's own allocation entry point; the
    // value returned owns the `AVFrame` and frees it on drop.
    unsafe { Frame::empty() }
}
