//! Detection of in-band closed captions in a video track's access units.
//!
//! The carriage is ATSC A/53 caption data: an H.264/HEVC
//! `user_data_registered_itu_t_t35` SEI ([`caption_cc_data`]) or the MPEG-2
//! picture `user_data()` whose `user_data_type_code` selects `MPEG_cc_data()`
//! (`private/specs-verified/mpeg/transmux/codec/a53-caption-sei.md`, Tables
//! 6.6-6.10). The `cc_data()` triplets inside are split by `cc_type` per
//! ETSI TS 101 154 Table B.9
//! (`private/specs-verified/captions/cc-data/ts_101_154/b9-cc-data.md`).
//!
//! What is reported, and why:
//!
//! - A CEA-608 field 1 byte pair that carries real data (odd parity valid, at
//!   least one non-null byte after the parity bit is stripped) reports
//!   [`DetectedCaption::Cea608Field1`], which is channel CC1.
//! - Likewise field 2 reports [`DetectedCaption::Cea608Field2`], channel CC3
//!   (`private/specs-verified/captions/cc-data/decode/cea608-decode.md`: the
//!   odd channels are the primary of each field).
//! - CC2 and CC4 are never reported: telling them apart from CC1/CC3 needs the
//!   control-code channel bit, which `cc_data::decode::Cea608Decoder` keeps
//!   internally and does not expose per received pair.
//! - A CEA-708 service is reported when `cc_data::decode::Cea708Decoder`, fed
//!   the DTVCC triplets, has defined a window for it
//!   (`private/specs-verified/captions/cc-data/decode/cea708-decode.md`); that
//!   decoder models services 1 to 6, so higher service numbers are not
//!   reported.
//!
//! Padding triplets (`cc_valid` set, null byte pair) are not captions and
//! report nothing, so a stream that merely reserves caption bandwidth is not
//! advertised.

use crate::ir::CodecConfig;
use crate::nal::{NalCodec, caption_cc_data};
use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use broadcast_common::Parse;
use broadcast_common::parity::strip_parity;
use cc_data::decode::Cea708Decoder;
use cc_data::{CcData, CcType};

const CC_DATA_HEADER_LEN: usize = 2;
const CC_TRIPLET_LEN: usize = 3;
const CC_DATA_MARKER_LEN: usize = 1;
const MPEG2_USER_IDENTIFIER_LEN: usize = 4;
const MPEG2_ATSC_CC_USER_DATA_TYPE_CODE: u8 = 0x03;
const CEA708_SERVICES_MODELLED: u8 = 6;

/// One caption stream found in the video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum DetectedCaption {
    /// CEA-608 field 1 data, line-21 channel CC1.
    Cea608Field1,
    /// CEA-608 field 2 data, line-21 channel CC3.
    Cea608Field2,
    /// A CEA-708 DTVCC service, numbered from 1.
    Cea708Service(u8),
}

impl DetectedCaption {
    /// Short label for logs.
    pub fn name(&self) -> &'static str {
        match self {
            DetectedCaption::Cea608Field1 => "cea608-field1",
            DetectedCaption::Cea608Field2 => "cea608-field2",
            DetectedCaption::Cea708Service(_) => "cea708-service",
        }
    }

    /// The HLS `INSTREAM-ID` this stream is advertised under.
    pub fn instream_id(&self) -> broadcast_hls::InstreamId {
        match self {
            DetectedCaption::Cea608Field1 => broadcast_hls::InstreamId::Cc(CC1),
            DetectedCaption::Cea608Field2 => broadcast_hls::InstreamId::Cc(CC3),
            DetectedCaption::Cea708Service(n) => broadcast_hls::InstreamId::Service(*n),
        }
    }
}

const CC1: u8 = 1;
const CC3: u8 = 3;

broadcast_common::impl_spec_display!(DetectedCaption);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Carriage {
    Sei(NalCodec),
    Mpeg2UserData,
}

/// Accumulates the caption streams seen across a video track's access units.
#[derive(Debug, Clone)]
pub struct CaptionDetector {
    carriage: Carriage,
    seen: BTreeSet<DetectedCaption>,
    dtvcc: Cea708Decoder,
}

impl CaptionDetector {
    /// A detector for the video codec of `config`, or `None` when that codec
    /// has no caption carriage this module reads.
    pub fn for_config(config: &CodecConfig) -> Option<Self> {
        let carriage = match config {
            CodecConfig::Avc { .. } => Carriage::Sei(NalCodec::Avc),
            CodecConfig::Hevc { .. } => Carriage::Sei(NalCodec::Hevc),
            CodecConfig::Mpeg2Video { .. } => Carriage::Mpeg2UserData,
            _ => return None,
        };
        Some(CaptionDetector {
            carriage,
            seen: BTreeSet::new(),
            dtvcc: Cea708Decoder::new(),
        })
    }

    /// Examine one video sample's data (length-prefixed NALs for AVC/HEVC, the
    /// elementary-stream picture for MPEG-2). Returns `true` when a caption
    /// stream not seen before was found.
    pub fn observe(&mut self, sample_data: &[u8]) -> bool {
        let cc_bytes = match self.carriage {
            Carriage::Sei(codec) => caption_cc_data(codec, sample_data, true),
            Carriage::Mpeg2UserData => mpeg2_cc_data(sample_data),
        };
        if cc_bytes.is_empty() {
            return false;
        }
        let before = self.seen.len();
        let mut rest = cc_bytes.as_slice();
        while let Ok(cc) = CcData::parse(rest) {
            let consumed =
                CC_DATA_HEADER_LEN + cc.triplets.len() * CC_TRIPLET_LEN + CC_DATA_MARKER_LEN;
            if cc.process_cc_data_flag {
                self.fold(&cc);
            }
            rest = rest.get(consumed..).unwrap_or(&[]);
        }
        self.seen.len() > before
    }

    /// Every caption stream seen so far, in a stable order.
    pub fn detected(&self) -> Vec<DetectedCaption> {
        self.seen.iter().copied().collect()
    }

    fn fold(&mut self, cc: &CcData) {
        let mut dtvcc_seen = false;
        for t in cc.triplets.iter().filter(|t| t.cc_valid) {
            match t.cc_type {
                CcType::Ntsc608Field1 if is_608_data(t.cc_data_1, t.cc_data_2) => {
                    self.seen.insert(DetectedCaption::Cea608Field1);
                }
                CcType::Ntsc608Field2 if is_608_data(t.cc_data_1, t.cc_data_2) => {
                    self.seen.insert(DetectedCaption::Cea608Field2);
                }
                CcType::Dtvcc708Start | CcType::Dtvcc708Data => dtvcc_seen = true,
                _ => {}
            }
        }
        if dtvcc_seen {
            self.dtvcc.push_triplets(cc.triplets.iter());
            for service in 1..=CEA708_SERVICES_MODELLED {
                let defined = self
                    .dtvcc
                    .windows(usize::from(service))
                    .iter()
                    .any(Option::is_some);
                if defined {
                    self.seen.insert(DetectedCaption::Cea708Service(service));
                }
            }
        }
    }
}

fn is_608_data(b1: u8, b2: u8) -> bool {
    match (strip_parity(b1), strip_parity(b2)) {
        (Some(a), Some(b)) => a != 0 || b != 0,
        _ => false,
    }
}

fn mpeg2_cc_data(au: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let (_, payloads) = crate::h262::picture_user_data_payloads(au);
    for payload in payloads {
        if payload.len() < MPEG2_USER_IDENTIFIER_LEN {
            continue;
        }
        let (identifier, body) = payload.split_at(MPEG2_USER_IDENTIFIER_LEN);
        let is_atsc = identifier == crate::user_data::GA94_USER_IDENTIFIER.to_be_bytes();
        if let Some((&type_code, cc)) = body.split_first()
            && is_atsc
            && type_code == MPEG2_ATSC_CC_USER_DATA_TYPE_CODE
        {
            out.extend_from_slice(cc);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PICTURE_START: [u8; 4] = [0x00, 0x00, 0x01, 0x00];
    const USER_DATA_START: [u8; 4] = [0x00, 0x00, 0x01, 0xB2];
    const CC_DATA_FIELD1_RESUME_CAPTION_LOADING: [u8; 7] =
        [0xC1, 0xFF, 0xFC, 0x94, 0x20, 0xFF, 0x00];

    fn mpeg2_picture(user_identifier: [u8; 4], type_code: u8) -> Vec<u8> {
        let mut au = PICTURE_START.to_vec();
        au.extend_from_slice(&[0x00, 0x0F, 0xFF, 0xF8]);
        au.extend_from_slice(&USER_DATA_START);
        au.extend_from_slice(&user_identifier);
        au.push(type_code);
        au.extend_from_slice(&CC_DATA_FIELD1_RESUME_CAPTION_LOADING[..6]);
        au
    }

    fn mpeg2_detector() -> CaptionDetector {
        CaptionDetector {
            carriage: Carriage::Mpeg2UserData,
            seen: BTreeSet::new(),
            dtvcc: Cea708Decoder::new(),
        }
    }

    #[test]
    fn mpeg2_ga94_cc_data_reports_field1() {
        let mut d = mpeg2_detector();
        let au = mpeg2_picture(*b"GA94", MPEG2_ATSC_CC_USER_DATA_TYPE_CODE);
        assert!(d.observe(&au));
        assert_eq!(d.detected(), vec![DetectedCaption::Cea608Field1]);
        assert!(!d.observe(&au), "a repeat is not new");
    }

    #[test]
    fn mpeg2_other_identifier_or_type_reports_nothing() {
        let mut d = mpeg2_detector();
        assert!(!d.observe(&mpeg2_picture(*b"DTG1", MPEG2_ATSC_CC_USER_DATA_TYPE_CODE)));
        assert!(!d.observe(&mpeg2_picture(*b"GA94", 0x06)));
        assert!(d.detected().is_empty());
    }

    #[test]
    fn null_padding_pair_is_not_a_caption() {
        assert!(!is_608_data(0x80, 0x80));
        assert!(is_608_data(0x94, 0x20));
        assert!(!is_608_data(0x14, 0x20), "bad parity is not data");
    }
}
