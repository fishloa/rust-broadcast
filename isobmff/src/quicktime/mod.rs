//! QuickTime atoms no ISO/IEC 14496-12:2026 file carries — read and written.
//!
//! A camera or editing `.mov` carries sound description versions 1 and 2, the
//! `wave` decompression container, the base media information header `gmhd`,
//! timecode tracks (`tmcd`), track aperture dimensions (`tapt`) and chapter
//! tracks. These types parse and serialise, so a `.mov` reads and writes
//! completely; every `Serialize` is the exact inverse of its `Parse`
//! (streaming-first design §4.7, §4.9).
//!
//! Sources (private):
//! `private/specs-verified/apple/isobmff/quicktime-file-format-online.md` and
//! `private/specs-verified/apple/isobmff/quicktime-file-format-2001.md`.

pub mod aperture;
pub mod chapter;
pub mod gmhd;
pub mod preload;
pub mod sound;
pub mod timecode;
pub mod video;
pub mod wave;

pub use aperture::{ApertureDimensions, TrackApertureModeDimensionsBox};
pub use chapter::chapter_title;
pub use gmhd::{GmhdAtom, GminAtom, TcmiAtom};
pub use preload::PreloadBox;
pub use sound::{QuickTimeSound, SoundDescriptionV1, SoundDescriptionV2, SoundEntry};
pub use timecode::{Timecode, TimecodeEntry, timecode_from_sample};
pub use video::QuickTimeVideoEntry;
pub use wave::WaveAtom;
