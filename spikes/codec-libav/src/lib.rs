//! Codec-seam spike: FFmpeg's libavcodec behind [`broadcast_codec`]'s
//! `Decoder` and `Encoder` traits.
//!
//! The seam crate holds no codec. This adapter is a [`CodecProvider`]
//! ([`LibavProvider`]) whose descriptors are every decoder and encoder the
//! linked libav reports for a [`CodecId`](broadcast_codec::CodecId) the seam
//! names, including the hardware codecs it registers under a name of their
//! own. An application reaches them through a
//! [`Registry`](broadcast_codec::Registry), so no call site names libav.
//!
//! # What the adapter does with the binding
//!
//! **Framing.** `ffmpeg-next` 8.1 exposes no extradata setter, so the `avcC`
//! record and the AAC `AudioSpecificConfig` a container carries cannot be
//! handed to libav. A H.264 decoder is opened with libav's own in-band mode
//! (`is_avc`, `nal_length_size`) and the `avcC` parameter sets are repeated in
//! front of every random-access access unit; a raw AAC access unit is wrapped
//! in an ADTS header. Both transforms are this crate's own.
//!
//! **Reading back what an encoder produced.** The binding exposes no extradata
//! getter either, so `sys::extradata` reads `AVCodecContext.extradata` through
//! the binding's raw pointer (one `SAFETY` argument); the AAC encoder's
//! `AudioSpecificConfig` therefore reaches the announced track. The same place
//! reads the encoder's `initial_padding`/`trailing_padding`, which become the
//! seam's gapless trim. An encoder with no extradata for a codec that needs a
//! record declares in-band configuration, so the stage derives it from the
//! first access unit.
//!
//! **No mid-stream reconfiguration.** [`Decoder::reconfigure`] is refused: the
//! binding cannot hand a new configuration record to an open context, so the
//! caller builds a fresh decoder instead.
//!
//! **Hardware decode.** libav offers hardware *acceleration modes* of an
//! ordinary decoder, and the binding exposes no hardware-device API, so an
//! acceleration mode is neither listed nor used here; a dedicated hardware
//! codec (for example `h264_videotoolbox`, an encoder) is listed as its own
//! implementation. A decoder that did hand back a hardware surface would have
//! to be transferred to CPU planes first, because
//! [`VideoStorage`](broadcast_codec::VideoStorage) has CPU storage only; this
//! adapter never receives one.
//!
//! **Frame side data.** The binding exposes frame side data only as opaque
//! bytes, so content-light and mastering-display metadata are not carried;
//! the colour description libav reports is.
//!
//! **Options.** Every [`EncoderConfig`](broadcast_codec::EncoderConfig)
//! option is forwarded to libav when the encoder is opened. libav reports a
//! rejected *value* through the open error; it silently leaves an unknown key
//! in the option dictionary, so an unknown key cannot be named from here.
//!
//! **Licence.** The adapter's own source is the workspace licence. It links
//! the FFmpeg build on the machine, dynamically and only through
//! `pkg-config`: the terms of that build (LGPL, or GPL when it was built with
//! GPL parts) apply to whoever distributes the linked result, and codec patent
//! licences are the distributor's matter. See `README.md`.

mod decoder;
mod encoder;
mod framing;
mod map;
mod provider;
mod sys;

pub use decoder::LibavDecoder;
pub use encoder::LibavEncoder;
pub use provider::LibavProvider;
