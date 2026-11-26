//! The sans-IO codec seam.

use crate::codec::CodecParameters;
use crate::error::CodecError;
use crate::frame::Frame;
use crate::packet::Packet;

/// A decoder driven by [`send`](Self::send) and
/// [`receive`](Self::receive).
///
/// The canonical loop is: send one packet, then call `receive` until it returns
/// `Ok(None)`; call [`finish`](Self::finish) at end of input and keep calling
/// `receive` until it returns `Ok(None)` again. `Ok(None)` from `receive` means
/// "send more" (or "the stream is drained" after `finish`), never an error.
///
/// A decoder is sans-IO: it owns no thread, reads no clock and performs no I/O.
/// [`reset`](Self::reset) drops every held packet and partial frame so the next
/// packet starts a new stream.
pub trait Decoder {
    /// What the decoder was built for.
    fn parameters(&self) -> &CodecParameters;

    /// Queue one coded access unit.
    ///
    /// # Errors
    /// [`CodecError::Full`] when the decoder wants [`Self::receive`] called
    /// first; [`CodecError::Refused`] for unacceptable input (still usable);
    /// [`CodecError::Failed`] when the decoder is unusable;
    /// [`CodecError::Unsupported`] for input it does not implement.
    fn send(&mut self, packet: Packet<'_>) -> Result<(), CodecError>;

    /// Take the next decoded frame, or `Ok(None)` when more input is needed.
    ///
    /// The returned frame holds every sample the decoder produced. It is
    /// untrimmed: the container's [`CodecParameters::initial_padding`] and
    /// [`CodecParameters::trailing_padding`], and the per-packet
    /// [`Packet::trim_start`]/[`Packet::trim_end`] it copies onto the frame,
    /// are left for a [`TrimStage`](crate::TrimStage) to apply, **unless** the
    /// implementation's descriptor says
    /// [`applies_trim`](crate::CodecDescriptor::applies_trim) is true, in which
    /// case it has already applied them itself.
    ///
    /// # Errors
    /// [`CodecError::Failed`] when the decoder is unusable;
    /// [`CodecError::Unsupported`] when decoding this stream is not supported.
    fn receive(&mut self) -> Result<Option<Frame>, CodecError>;

    /// Signal end of input; keep calling [`Self::receive`] to drain.
    ///
    /// # Errors
    /// [`CodecError::Failed`] when the decoder is unusable.
    fn finish(&mut self) -> Result<(), CodecError>;

    /// Drop every held packet and partial frame.
    fn reset(&mut self);
}

/// An encoder driven by [`send`](Self::send) and
/// [`receive`](Self::receive).
///
/// The loop mirrors [`Decoder`]: send one frame, then call `receive` until it
/// returns `Ok(None)`; call [`finish`](Self::finish) at end of input and keep
/// calling `receive` until it returns `Ok(None)` again. [`parameters`](
/// Self::parameters) is `None` until the encoder has seen enough to report its
/// output format.
pub trait Encoder {
    /// Queue one decoded frame.
    ///
    /// # Errors
    /// As [`Decoder::send`].
    fn send(&mut self, frame: Frame) -> Result<(), CodecError>;

    /// Take the next coded access unit, or `Ok(None)` when more input is
    /// needed.
    ///
    /// # Errors
    /// As [`Decoder::receive`].
    fn receive(&mut self) -> Result<Option<Packet<'static>>, CodecError>;

    /// Signal end of input; keep calling [`Self::receive`] to drain.
    ///
    /// # Errors
    /// As [`Decoder::finish`].
    fn finish(&mut self) -> Result<(), CodecError>;

    /// What the encoder now knows it produces, or `None` before it does.
    fn parameters(&self) -> Option<&CodecParameters>;
}
