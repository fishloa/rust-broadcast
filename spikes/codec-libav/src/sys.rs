//! The one place this crate uses the binding's unsafe API.
//!
//! Every other module drives `ffmpeg-next` through its safe surface.

use std::vec::Vec;

use ffmpeg_next::Frame;
use ffmpeg_next::codec::Context;

/// A freshly allocated, empty libav frame for `receive_frame` to fill.
pub(crate) fn empty_frame() -> Frame {
    // SAFETY: `Frame::empty` is the binding's own allocation entry point; the
    // value returned owns the `AVFrame` and frees it on drop.
    unsafe { Frame::empty() }
}

/// The gapless padding libav reports for `context`, in samples per channel.
///
/// `initial_padding` is the encoder delay before the first real sample;
/// `trailing_padding` is `-1` when libav does not know it, and that becomes
/// `None` rather than a fabricated zero.
pub(crate) fn padding(context: &Context) -> (u32, Option<u32>) {
    // SAFETY: as for `extradata` above. Both fields are plain integers on the
    // live `AVCodecContext`; nothing is dereferenced through them.
    unsafe {
        let raw = context.as_ptr();
        if raw.is_null() {
            return (0, None);
        }
        let initial = u32::try_from((*raw).initial_padding).unwrap_or(0);
        let trailing = (*raw).trailing_padding;
        let trailing = if trailing < 0 {
            None
        } else {
            u32::try_from(trailing).ok()
        };
        (initial, trailing)
    }
}

/// The extradata libav set on `context` while opening or encoding.
///
/// `ffmpeg-next` 8.1 exposes no getter, so this reads the two
/// `AVCodecContext` fields through the binding's own raw pointer. The returned
/// copy owns its bytes; libav may replace or free its buffer at any point, and
/// the copy never outlives that risk.
///
/// A negative or absent `extradata_size` yields no bytes: the encoder wrote no
/// configuration record out of band.
pub(crate) fn extradata(context: &Context) -> Vec<u8> {
    // SAFETY: `Context::as_ptr` returns the live `AVCodecContext` the context
    // owns, and this binding's `Context` keeps it alive for the duration of
    // the borrow. The fields read are the ones libav documents as
    // `extradata`/`extradata_size`; the slice is bounded by the size libav
    // itself reports and copied out before the borrow ends.
    unsafe {
        let raw = context.as_ptr();
        if raw.is_null() {
            return Vec::new();
        }
        let size = (*raw).extradata_size;
        if size <= 0 || (*raw).extradata.is_null() {
            return Vec::new();
        }
        let len = usize::try_from(size).unwrap_or(0);
        if len == 0 {
            return Vec::new();
        }
        core::slice::from_raw_parts((*raw).extradata, len).to_vec()
    }
}
