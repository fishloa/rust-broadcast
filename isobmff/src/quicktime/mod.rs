//! QuickTime atoms no ISO/IEC 14496-12:2026 file carries — **read only**.
//!
//! A camera or editing `.mov` carries sound description versions 1 and 2, the
//! `wave` decompression container, the base media information header `gmhd`,
//! timecode tracks (`tmcd`), track aperture dimensions (`tapt`) and chapter
//! tracks. These types parse and are kept so such a file reads completely;
//! [`Serialize`](broadcast_common::Serialize) is deliberately not implemented:
//! writing a track read from a `.mov` either produces its ISO form or fails
//! with [`Error::QuickTimeOnly`](crate::Error::QuickTimeOnly) (streaming-first
//! design §4.7, §4.9).
//!
//! Sources (private):
//! `private/specs-verified/apple/isobmff/quicktime-file-format-online.md` and
//! `private/specs-verified/apple/isobmff/quicktime-file-format-2001.md`.

pub mod aperture;
pub mod chapter;
pub mod gmhd;
pub mod sound;
pub mod timecode;
pub mod wave;

pub use aperture::{ApertureDimensions, TrackApertureModeDimensionsBox};
pub use chapter::chapter_title;
pub use gmhd::{GmhdAtom, GminAtom, TcmiAtom};
pub use sound::{QuickTimeSound, SoundDescriptionV1, SoundDescriptionV2, SoundEntry};
pub use timecode::{Timecode, TimecodeEntry, timecode_from_sample};
pub use wave::WaveAtom;
