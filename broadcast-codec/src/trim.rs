//! Applying a container's gapless trim to decoded audio.
//!
//! A lossy audio encoder adds samples that are not part of the programme: a
//! priming lead-in before the first real sample and padding after the last. The
//! container says how many to discard in [`CodecParameters`] and, for
//! per-packet signalling, on each [`Packet`]. [`TrimStage`] applies that trim
//! once, for every adapter and caller.
//!
//! A frame's `pts` is in [`CodecParameters::time_base`]. Removing `samples`
//! from the front of a frame at `sample_rate` advances its timestamp by
//! `samples * time_base / sample_rate` ticks, via
//! [`broadcast_core::timescale::rescale_i64`] with `Rounding::TowardZero`.
//!
//! [`Packet`]: crate::Packet

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use broadcast_core::timescale::{Rounding, Timescale, rescale_i64};

use crate::audio::{AudioFrame, AudioSamples, SampleLayout};
use crate::codec::CodecParameters;
use crate::error::CodecError;
use crate::frame::Frame;
use crate::traits::Decoder;

/// The gapless trim of one decoded audio stream.
///
/// Built from a [`CodecParameters`], fed frames in decode order, and finished
/// at end of stream. It removes the initial padding from the front, applies
/// each frame's own `trim_start`/`trim_end` and clears them, and holds back the
/// trailing padding until [`Self::finish`], which discards it. It never holds
/// more than the trailing padding plus the frame just pushed.
pub struct TrimStage {
    time_base: Timescale,
    initial_padding: u32,
    initial_remaining: u32,
    trailing_padding: Option<u32>,
    held: VecDeque<AudioFrame>,
    held_samples: usize,
}

impl TrimStage {
    /// A stage for `parameters`'s padding and time base.
    #[must_use]
    pub fn new(parameters: &CodecParameters) -> Self {
        let initial_padding = parameters.initial_padding();
        Self {
            time_base: parameters.time_base(),
            initial_padding,
            initial_remaining: initial_padding,
            trailing_padding: parameters.trailing_padding(),
            held: VecDeque::new(),
            held_samples: 0,
        }
    }

    /// Feed one decoded frame in decode order and take the frames that are
    /// ready to emit.
    pub fn push(&mut self, frame: AudioFrame) -> Vec<AudioFrame> {
        let frames = frame.frames();
        let front = usize::try_from(frame.trim_start())
            .unwrap_or(usize::MAX)
            .min(frames);
        let back = usize::try_from(frame.trim_end())
            .unwrap_or(usize::MAX)
            .min(frames - front);
        let mut frame = if front == 0 && back == 0 {
            frame
        } else {
            match self.slice(&frame, front, frames - back) {
                Some(trimmed) => trimmed,
                None => return Vec::new(),
            }
        };
        if frame.frames() == 0 {
            return Vec::new();
        }
        if self.initial_remaining > 0 {
            let drop = usize::try_from(self.initial_remaining)
                .unwrap_or(usize::MAX)
                .min(frame.frames());
            let consumed = u32::try_from(drop).unwrap_or(self.initial_remaining);
            self.initial_remaining -= consumed;
            if drop == frame.frames() {
                return Vec::new();
            }
            frame = match self.slice(&frame, drop, frame.frames()) {
                Some(trimmed) => trimmed,
                None => return Vec::new(),
            };
        }
        self.hold(frame)
    }

    /// Discard the trailing padding held back; call at end of stream.
    ///
    /// The trailing padding is not part of the programme, so this returns no
    /// frames; a wrapper still appends whatever it returns.
    pub fn finish(&mut self) -> Vec<AudioFrame> {
        self.held.clear();
        self.held_samples = 0;
        Vec::new()
    }

    /// Drop all state so the next frame starts a new stream.
    pub fn reset(&mut self) {
        self.initial_remaining = self.initial_padding;
        self.held.clear();
        self.held_samples = 0;
    }

    /// Samples per channel currently held back for the trailing padding.
    #[must_use]
    pub fn held_samples(&self) -> usize {
        self.held_samples
    }

    /// Frames currently held back for the trailing padding.
    #[must_use]
    pub fn held_frames(&self) -> usize {
        self.held.len()
    }

    /// Keep all but the last `trailing_padding` samples of what has been fed.
    fn hold(&mut self, frame: AudioFrame) -> Vec<AudioFrame> {
        let Some(trailing) = self.trailing_padding else {
            return alloc::vec![frame];
        };
        let trailing = usize::try_from(trailing).unwrap_or(usize::MAX);
        if trailing == 0 {
            return alloc::vec![frame];
        }
        self.held_samples = self.held_samples.saturating_add(frame.frames());
        self.held.push_back(frame);
        let mut emitted = Vec::new();
        while self.held_samples > trailing {
            let excess = self.held_samples - trailing;
            let Some(front) = self.held.pop_front() else {
                break;
            };
            let frames = front.frames();
            if frames <= excess {
                self.held_samples -= frames;
                emitted.push(front);
            } else {
                if let Some(prefix) = self.slice(&front, 0, excess)
                    && let Some(suffix) = self.slice(&front, excess, frames)
                {
                    self.held.push_front(suffix);
                    emitted.push(prefix);
                }
                self.held_samples -= excess;
            }
        }
        emitted
    }

    /// The frames `[start, end)` of `frame`, with its timestamp advanced by the
    /// samples removed from its front.
    fn slice(&self, frame: &AudioFrame, start: usize, end: usize) -> Option<AudioFrame> {
        let frames = frame.frames();
        if start >= end || end > frames {
            return None;
        }
        let channels = frame.channels().count();
        let samples = slice_samples(
            frame.samples(),
            frame.layout(),
            channels,
            frames,
            start,
            end,
        );
        let mut trimmed = AudioFrame::new(
            frame.sample_rate(),
            frame.channels(),
            frame.layout(),
            samples,
        )?;
        if let Some(pts) = frame.pts() {
            let removed = i64::try_from(start).unwrap_or(i64::MAX);
            let rate = Timescale::from_nonzero(frame.sample_rate());
            let shift = rescale_i64(removed, rate, self.time_base, Rounding::TowardZero);
            trimmed = trimmed.with_pts(pts.saturating_add(shift));
        }
        Some(trimmed)
    }
}

/// A decoder whose output goes through a [`TrimStage`].
pub(crate) struct TrimmedDecoder {
    inner: Box<dyn Decoder>,
    trim: TrimStage,
    ready: VecDeque<Frame>,
    finished: bool,
    drained: bool,
}

impl TrimmedDecoder {
    /// Wrap `inner` so its audio output is trimmed by `parameters`.
    pub(crate) fn new(inner: Box<dyn Decoder>, parameters: &CodecParameters) -> Self {
        Self {
            inner,
            trim: TrimStage::new(parameters),
            ready: VecDeque::new(),
            finished: false,
            drained: false,
        }
    }
}

impl Decoder for TrimmedDecoder {
    fn parameters(&self) -> &CodecParameters {
        self.inner.parameters()
    }

    fn send(&mut self, packet: crate::Packet<'_>) -> Result<(), CodecError> {
        self.inner.send(packet)
    }

    fn receive(&mut self) -> Result<Option<Frame>, CodecError> {
        loop {
            if let Some(frame) = self.ready.pop_front() {
                return Ok(Some(frame));
            }
            match self.inner.receive()? {
                Some(Frame::Audio(audio)) => {
                    for audio in self.trim.push(audio) {
                        self.ready.push_back(Frame::Audio(audio));
                    }
                }
                Some(frame) => self.ready.push_back(frame),
                None => {
                    if self.finished && !self.drained {
                        self.drained = true;
                        for audio in self.trim.finish() {
                            self.ready.push_back(Frame::Audio(audio));
                        }
                    }
                    return Ok(None);
                }
            }
        }
    }

    fn finish(&mut self) -> Result<(), CodecError> {
        self.inner.finish()?;
        self.finished = true;
        Ok(())
    }

    fn reset(&mut self) {
        self.inner.reset();
        self.trim.reset();
        self.ready.clear();
        self.finished = false;
        self.drained = false;
    }
}

/// The samples of `frame` between two whole-frame boundaries.
fn slice_samples(
    samples: &AudioSamples,
    layout: SampleLayout,
    channels: usize,
    frames: usize,
    start: usize,
    end: usize,
) -> AudioSamples {
    match samples {
        AudioSamples::U8(values) => {
            AudioSamples::U8(slice_typed(values, layout, channels, frames, start, end))
        }
        AudioSamples::S16(values) => {
            AudioSamples::S16(slice_typed(values, layout, channels, frames, start, end))
        }
        AudioSamples::S32(values) => {
            AudioSamples::S32(slice_typed(values, layout, channels, frames, start, end))
        }
        AudioSamples::F32(values) => {
            AudioSamples::F32(slice_typed(values, layout, channels, frames, start, end))
        }
        AudioSamples::F64(values) => {
            AudioSamples::F64(slice_typed(values, layout, channels, frames, start, end))
        }
    }
}

/// One typed buffer between two whole-frame boundaries.
fn slice_typed<T: Copy>(
    values: &[T],
    layout: SampleLayout,
    channels: usize,
    frames: usize,
    start: usize,
    end: usize,
) -> Vec<T> {
    match layout {
        SampleLayout::Interleaved => values[start * channels..end * channels].to_vec(),
        SampleLayout::Planar => {
            let mut out = Vec::with_capacity((end - start) * channels);
            for channel in 0..channels {
                let base = channel * frames;
                out.extend_from_slice(&values[base + start..base + end]);
            }
            out
        }
    }
}
