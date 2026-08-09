//! CEA-608/CEA-708 -> WebVTT/SRT, feature `cc-data`.
//!
//! Thin wrappers over `timed-metadata`'s
//! [`timed_metadata::webvtt::Cea608CueExtractor`] /
//! [`timed_metadata::webvtt::Cea708CueExtractor`] (issue #568): those own
//! the correct roll-up/pop-on/paint-on cue-boundary detection (a
//! diff-on-displayed-text state machine -- see that module's docs for the
//! full design and the documented losses). What this module adds is a raw
//! `cc_data()` byte-string entry point (parsed via `cc_data::CcData::parse`)
//! and direct `into_webvtt`/`into_srt` string output, so a caller feeding
//! carriage bytes never has to depend on `cc_data`/`timed_metadata` types
//! itself.

use crate::error::Error;
use crate::webvtt::write_document;
use alloc::string::String;
use alloc::vec::Vec;
use cc_data::decode::Cea608Channel;
use timed_metadata::webvtt::{Cea608CueExtractor, Cea708CueExtractor, Cue};

/// Converts a single CEA-608 data channel's `cc_data()` byte stream to
/// WebVTT/SRT.
pub struct Cea608ToWebVtt {
    extractor: Cea608CueExtractor,
}

impl Cea608ToWebVtt {
    /// A new converter tracking the given CEA-608 data channel (e.g. `Cc1`).
    #[must_use]
    pub fn new(channel: Cea608Channel) -> Self {
        Cea608ToWebVtt {
            extractor: Cea608CueExtractor::new(channel),
        }
    }

    /// Feed one access unit's raw `cc_data()` bytes (ETSI TS 101 154 Table
    /// B.9 wire framing), tagged with that access unit's raw 33-bit PTS.
    ///
    /// # Errors
    ///
    /// [`Error::CcData`] if `cc_data_bytes` is not a valid `cc_data()`
    /// structure.
    pub fn push_cc_data(&mut self, pts_90k: u64, cc_data_bytes: &[u8]) -> Result<(), Error> {
        use broadcast_common::Parse;
        let cc = cc_data::CcData::parse(cc_data_bytes)?;
        self.extractor.push_frame(pts_90k, &cc.triplets);
        Ok(())
    }

    /// Close any still-open cue at end of stream, at `end_pts_90k`.
    pub fn finalize(&mut self, end_pts_90k: u64) {
        self.extractor.finalize(end_pts_90k);
    }

    /// The cues extracted so far.
    #[must_use]
    pub fn cues(&self) -> &[Cue] {
        self.extractor.cues()
    }

    /// Consume the converter, returning the extracted cues.
    #[must_use]
    pub fn into_cues(self) -> Vec<Cue> {
        self.extractor.into_cues()
    }

    /// Consume the converter, rendering a standalone WebVTT document.
    #[must_use]
    pub fn into_webvtt(self) -> String {
        write_document(&self.into_cues())
    }

    /// Consume the converter, rendering an SRT document.
    #[must_use]
    pub fn into_srt(self) -> String {
        crate::srt::write_srt(&self.into_cues())
    }
}

/// Converts a single CEA-708 (DTVCC) service's `cc_data()` byte stream to
/// WebVTT/SRT.
pub struct Cea708ToWebVtt {
    extractor: Cea708CueExtractor,
}

impl Cea708ToWebVtt {
    /// A new converter tracking the given CEA-708 service number (`1`-`6`;
    /// service 1 is the primary caption service).
    #[must_use]
    pub fn new(service_number: usize) -> Self {
        Cea708ToWebVtt {
            extractor: Cea708CueExtractor::new(service_number),
        }
    }

    /// Feed one access unit's raw `cc_data()` bytes, tagged with that access
    /// unit's raw 33-bit PTS.
    ///
    /// # Errors
    ///
    /// [`Error::CcData`] if `cc_data_bytes` is not a valid `cc_data()`
    /// structure.
    pub fn push_cc_data(&mut self, pts_90k: u64, cc_data_bytes: &[u8]) -> Result<(), Error> {
        use broadcast_common::Parse;
        let cc = cc_data::CcData::parse(cc_data_bytes)?;
        self.extractor.push_frame(pts_90k, &cc.triplets);
        Ok(())
    }

    /// Close any still-open cue at end of stream, at `end_pts_90k`.
    pub fn finalize(&mut self, end_pts_90k: u64) {
        self.extractor.finalize(end_pts_90k);
    }

    /// The cues extracted so far.
    #[must_use]
    pub fn cues(&self) -> &[Cue] {
        self.extractor.cues()
    }

    /// Consume the converter, returning the extracted cues.
    #[must_use]
    pub fn into_cues(self) -> Vec<Cue> {
        self.extractor.into_cues()
    }

    /// Consume the converter, rendering a standalone WebVTT document.
    #[must_use]
    pub fn into_webvtt(self) -> String {
        write_document(&self.into_cues())
    }

    /// Consume the converter, rendering an SRT document.
    #[must_use]
    pub fn into_srt(self) -> String {
        crate::srt::write_srt(&self.into_cues())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Serialize;
    use cc_data::{CcData, CcTriplet, CcType};

    fn cc_data_bytes(triplets: &[CcTriplet]) -> alloc::vec::Vec<u8> {
        let cc = CcData {
            process_cc_data_flag: true,
            triplets: triplets.to_vec(),
        };
        let mut buf = alloc::vec![0u8; cc.serialized_len()];
        cc.serialize_into(&mut buf).unwrap();
        buf
    }

    fn t608(b1: u8, b2: u8) -> CcTriplet {
        CcTriplet {
            cc_valid: true,
            cc_type: CcType::Ntsc608Field1,
            cc_data_1: b1,
            cc_data_2: b2,
        }
    }

    #[test]
    fn pop_on_via_raw_cc_data_bytes() {
        let mut conv = Cea608ToWebVtt::new(Cea608Channel::Cc1);
        conv.push_cc_data(0, &cc_data_bytes(&[t608(0x14, 0x20)]))
            .unwrap(); // RCL
        conv.push_cc_data(1, &cc_data_bytes(&[t608(0x14, 0x70)]))
            .unwrap(); // PAC row 15
        conv.push_cc_data(2, &cc_data_bytes(&[t608(b'H', b'I')]))
            .unwrap();
        conv.push_cc_data(3, &cc_data_bytes(&[t608(0x14, 0x2F)]))
            .unwrap(); // EOC
        conv.push_cc_data(4, &cc_data_bytes(&[t608(0x14, 0x2C)]))
            .unwrap(); // EDM
        let vtt = conv.into_webvtt();
        assert!(vtt.starts_with("WEBVTT\n\n"));
        assert!(vtt.contains("HI"));
    }

    #[test]
    fn into_srt_matches_webvtt_text() {
        let mut conv = Cea608ToWebVtt::new(Cea608Channel::Cc1);
        conv.push_cc_data(0, &cc_data_bytes(&[t608(0x14, 0x20)]))
            .unwrap();
        conv.push_cc_data(1, &cc_data_bytes(&[t608(0x14, 0x70)]))
            .unwrap();
        conv.push_cc_data(2, &cc_data_bytes(&[t608(b'O', b'K')]))
            .unwrap();
        conv.push_cc_data(3, &cc_data_bytes(&[t608(0x14, 0x2F)]))
            .unwrap();
        conv.finalize(10);
        let srt = conv.into_srt();
        assert!(srt.starts_with("1\n"));
        assert!(srt.contains("OK"));
        assert!(
            srt.contains(","),
            "SRT timestamps must use a comma separator"
        );
    }

    #[test]
    fn invalid_cc_data_bytes_error() {
        let mut conv = Cea608ToWebVtt::new(Cea608Channel::Cc1);
        // cc_count with more bytes claimed than provided.
        let err = conv.push_cc_data(0, &[0xFF]);
        assert!(err.is_err());
    }

    #[test]
    fn cea708_service_selection() {
        let mut conv = Cea708ToWebVtt::new(1);
        assert_eq!(conv.cues().len(), 0);
        conv.finalize(0);
        assert_eq!(conv.into_cues().len(), 0);
    }
}
