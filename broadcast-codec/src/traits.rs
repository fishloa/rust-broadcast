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

    /// Adopt `parameters` mid-stream: a live source changed its resolution,
    /// sample rate or parameter sets.
    ///
    /// The caller drains first: it calls [`Self::receive`] until it returns
    /// `Ok(None)`, and a decoder that still holds a frame refuses with
    /// [`CodecError::Full`] rather than lose it. On success
    /// [`parameters`](Self::parameters) returns the new set and the next
    /// [`Self::receive`] continues from where the old stream left off; coded
    /// input that had not been decoded yet belonged to the old configuration
    /// and is discarded, exactly as a stream switch discards a partial access
    /// unit.
    ///
    /// A decoder that cannot change its configuration returns
    /// [`CodecError::CannotReconfigure`] naming why, so the caller can build a
    /// fresh decoder. The default implementation is that refusal.
    ///
    /// # Errors
    /// [`CodecError::Full`] while output is still pending;
    /// [`CodecError::CannotReconfigure`] when the decoder cannot adopt them;
    /// [`CodecError::Failed`] when the decoder is unusable.
    fn reconfigure(&mut self, parameters: CodecParameters) -> Result<(), CodecError> {
        let _ = parameters;
        Err(CodecError::cannot_reconfigure(
            "a decoder that keeps the parameters it was built for",
        ))
    }
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
    ///
    /// The reported parameters name the output stream a caller announces
    /// downstream, so the configuration record they need must be there or
    /// said to be in-band: for each [`CodecId`](crate::CodecId), the record is
    /// the one [`CodecParameters::extradata`] documents (an `avcC` body for
    /// H.264, an `AudioSpecificConfig` for AAC, and so on). An encoder that
    /// emits the record inside its first access unit reports
    /// [`ConfigurationRecord::InBand`](crate::ConfigurationRecord::InBand)
    /// rather than empty bytes, so a caller can derive it from that access
    /// unit; one that states no record at all leaves
    /// `configuration_record()` at `Absent`.
    ///
    /// The encoder may report different parameters as it learns more (for
    /// example once its first picture or frame is encoded); a caller
    /// re-announces only when they change.
    fn parameters(&self) -> Option<&CodecParameters>;
}
