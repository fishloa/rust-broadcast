//! EBU Teletext -> WebVTT/SRT, feature `teletext`.
//!
//! Thin wrapper over `timed-metadata`'s
//! [`timed_metadata::webvtt::TeletextCueExtractor`] (issue #666), which owns
//! the EN 300 706 decode (Hamming-8/4 + odd-parity FEC, the national G0
//! character subset, basic Level-1 page composition) layered on `dvb-vbi`'s
//! carriage-only [`dvb_vbi::TeletextDataField`]. See that module's docs for
//! the full documented losses (no enhancement packets, no styling, no
//! sub-code page selection).

use crate::error::Error;
use crate::webvtt::write_document;
use alloc::string::String;
use alloc::vec::Vec;
use dvb_vbi::TeletextDataField;
use timed_metadata::webvtt::{Cue, TeletextCueExtractor};

/// Converts a single `(magazine, page)` EBU Teletext subtitle page to
/// WebVTT/SRT.
pub struct TeletextToWebVtt {
    extractor: TeletextCueExtractor,
}

impl TeletextToWebVtt {
    /// A new converter tracking the given `(magazine, page)` (magazine
    /// `1..=8`; page `Pt << 4 | Pu`, e.g. `0x88` for the common "page 888"
    /// subtitle convention).
    #[must_use]
    pub fn new(magazine: u8, page: u8) -> Self {
        TeletextToWebVtt {
            extractor: TeletextCueExtractor::new(magazine, page),
        }
    }

    /// Feed every already-parsed [`TeletextDataField`] carried by one
    /// access unit, tagged with that access unit's raw 33-bit PTS. Fields
    /// for other magazines/pages are ignored.
    pub fn push_fields(&mut self, pts_90k: u64, fields: &[TeletextDataField]) {
        self.extractor.push_frame(pts_90k, fields);
    }

    /// Feed every raw 44-byte Teletext data-field wire payload (`header_byte`
    /// `+` `framing_code` `+` 42-byte `txt_data_block`, ETSI EN 301 775
    /// §4.5) carried by one access unit, parsing each with
    /// [`TeletextDataField::parse`].
    ///
    /// # Errors
    ///
    /// [`Error::DvbVbi`] if any wire payload fails to parse.
    pub fn push_wire_fields(&mut self, pts_90k: u64, wire_fields: &[&[u8]]) -> Result<(), Error> {
        let fields: Vec<TeletextDataField> = wire_fields
            .iter()
            .map(|w| TeletextDataField::parse(w))
            .collect::<Result<_, _>>()?;
        self.extractor.push_frame(pts_90k, &fields);
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
    use dvb_vbi::{FRAMING_CODE_EBU, LineHeader};

    fn hamming(n: u8) -> u8 {
        let (d1, d2, d3, d4) = (n & 1, (n >> 1) & 1, (n >> 2) & 1, (n >> 3) & 1);
        let p1 = 1 ^ d1 ^ d3 ^ d4;
        let p2 = 1 ^ d1 ^ d2 ^ d4;
        let p3 = 1 ^ d1 ^ d2 ^ d3;
        let p4 = 1 ^ p1 ^ d1 ^ p2 ^ d2 ^ p3 ^ d3 ^ d4;
        p1 | (d1 << 1) | (p2 << 2) | (d2 << 3) | (p3 << 4) | (d3 << 5) | (p4 << 6) | (d4 << 7)
    }

    fn parity(d7: u8) -> u8 {
        let d = d7 & 0x7F;
        if d.count_ones().is_multiple_of(2) {
            d | 0x80
        } else {
            d
        }
    }

    fn field(block: [u8; 42]) -> TeletextDataField {
        TeletextDataField {
            header: LineHeader::new(true, 0),
            framing_code: FRAMING_CODE_EBU,
            txt_data_block: block,
        }
    }

    #[test]
    fn header_then_row_via_typed_fields() {
        let mut header = [0u8; 42];
        header[0] = hamming(0); // magazine 8, row 0
        header[1] = hamming(0);
        header[2] = hamming(8); // page units
        header[3] = hamming(8); // page tens -> page 0x88
        header[5] = hamming(0b1000); // C4 erase_page
        header[7] = hamming(0b1000); // C6 subtitle
        for b in header.iter_mut().skip(10) {
            *b = parity(0x20);
        }

        let mut row1 = [0u8; 42];
        row1[0] = hamming(1 << 3); // magazine 8, Y bit0 = 1 (row 1)
        row1[1] = hamming(0);
        row1[2] = parity(b'H');
        row1[3] = parity(b'I');
        for b in row1.iter_mut().skip(4) {
            *b = parity(0x20);
        }

        let mut conv = TeletextToWebVtt::new(8, 0x88);
        conv.push_fields(0, &[field(header)]);
        conv.push_fields(1, &[field(row1)]);
        conv.finalize(2);
        assert_eq!(conv.cues().len(), 1);
        assert_eq!(conv.cues()[0].text, "HI");
    }

    #[test]
    fn push_wire_fields_matches_push_fields() {
        let mut header = [0u8; 42];
        header[0] = hamming(0);
        header[1] = hamming(0);
        header[2] = hamming(8);
        header[3] = hamming(8);
        header[5] = hamming(0b1000);
        header[7] = hamming(0b1000);
        for b in header.iter_mut().skip(10) {
            *b = parity(0x20);
        }
        let mut wire = alloc::vec![
            LineHeader::new(true, 0).to_byte().unwrap(),
            FRAMING_CODE_EBU
        ];
        wire.extend_from_slice(&header);

        let mut conv = TeletextToWebVtt::new(8, 0x88);
        conv.push_wire_fields(0, &[&wire]).unwrap();
        conv.finalize(1);
        // The erase-page header alone opens/produces no visible-text cue.
        assert_eq!(conv.cues().len(), 0);
    }

    #[test]
    fn push_wire_fields_rejects_short_input() {
        let mut conv = TeletextToWebVtt::new(8, 0x88);
        let err = conv.push_wire_fields(0, &[&[0x00]]);
        assert!(err.is_err());
    }
}
