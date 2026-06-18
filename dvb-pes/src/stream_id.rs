//! `stream_id` — identifies the elementary stream a PES packet belongs to
//! (ISO/IEC 13818-1 Table 2-22). Certain values carry no optional PES header.

/// 8-bit `stream_id` of a PES packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StreamId(pub u8);

// Special stream_ids that do NOT carry the optional PES header (the bytes after
// PES_packet_length are the payload directly) — ISO/IEC 13818-1 §2.4.3.6.
const PROGRAM_STREAM_MAP: u8 = 0xBC;
const PADDING_STREAM: u8 = 0xBE;
const PRIVATE_STREAM_2: u8 = 0xBF;
const ECM_STREAM: u8 = 0xF0;
const EMM_STREAM: u8 = 0xF1;
const DSMCC_STREAM: u8 = 0xF2;
const H222_1_TYPE_E: u8 = 0xF8;
const PROGRAM_STREAM_DIRECTORY: u8 = 0xFF;

impl StreamId {
    /// `program_stream_map` (0xBC).
    pub const PROGRAM_STREAM_MAP: StreamId = StreamId(PROGRAM_STREAM_MAP);
    /// `padding_stream` (0xBE).
    pub const PADDING_STREAM: StreamId = StreamId(PADDING_STREAM);
    /// `private_stream_2` (0xBF).
    pub const PRIVATE_STREAM_2: StreamId = StreamId(PRIVATE_STREAM_2);

    /// True if this `stream_id` carries the optional PES header (flags +
    /// `PES_header_data_length` + PTS/DTS). False for the special streams whose
    /// `PES_packet_data_byte`s follow `PES_packet_length` directly.
    #[must_use]
    pub const fn has_optional_header(self) -> bool {
        !matches!(
            self.0,
            PROGRAM_STREAM_MAP
                | PADDING_STREAM
                | PRIVATE_STREAM_2
                | ECM_STREAM
                | EMM_STREAM
                | DSMCC_STREAM
                | H222_1_TYPE_E
                | PROGRAM_STREAM_DIRECTORY
        )
    }

    /// True for an audio stream (`110x xxxx`, 0xC0–0xDF).
    #[must_use]
    pub const fn is_audio(self) -> bool {
        self.0 & 0xE0 == 0xC0
    }

    /// True for a video stream (`1110 xxxx`, 0xE0–0xEF).
    #[must_use]
    pub const fn is_video(self) -> bool {
        self.0 & 0xF0 == 0xE0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification() {
        assert!(StreamId(0xC0).is_audio());
        assert!(StreamId(0xE0).is_video());
        assert!(StreamId(0xE0).has_optional_header());
        assert!(StreamId(0xBD).has_optional_header()); // private_stream_1 does
        assert!(!StreamId::PADDING_STREAM.has_optional_header());
        assert!(!StreamId::PROGRAM_STREAM_MAP.has_optional_header());
        assert!(!StreamId(0xFF).has_optional_header());
    }
}
