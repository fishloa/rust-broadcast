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
//! - A CEA-608 channel (CC1-CC4) is reported once `cc_data::decode::Cea608Decoder`
//!   has written a displayable character to it outside text mode
//!   (`private/specs-verified/captions/cc-data/decode/cea608-decode.md`). The
//!   decoder routes by field and by the data-channel bit of the control codes
//!   (field 1 -> CC1/CC2, field 2 -> CC3/CC4), skips XDS and ignores text
//!   mode, so a stream that only carries control codes, XDS or null padding
//!   advertises nothing.
//! - A CEA-708 service is reported once a service block with a non-null byte
//!   has arrived for it (`private/specs-verified/captions/cc-data/decode/cea708-decode.md`,
//!   service block header, extended service numbers 7 to 63 included).
//!
//! Padding triplets (`cc_valid` set, null byte pair) are not captions and
//! report nothing, so a stream that merely reserves caption bandwidth is not
//! advertised.

use crate::ir::CodecConfig;
use crate::nal::{NalCodec, caption_cc_data};
use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use broadcast_common::Parse;
use broadcast_common::parity::has_odd_parity;
use cc_data::decode::{Cea608Channel, Cea608Decoder, Cea708Decoder};
use cc_data::{CcData, CcType};

const CC_DATA_HEADER_LEN: usize = 2;
const CC_TRIPLET_LEN: usize = 3;
const CC_DATA_MARKER_LEN: usize = 1;
const MPEG2_USER_IDENTIFIER_LEN: usize = 4;
const MPEG2_ATSC_CC_USER_DATA_TYPE_CODE: u8 = 0x03;
const CEA608_CHANNELS: [Cea608Channel; 4] = [
    Cea608Channel::Cc1,
    Cea608Channel::Cc2,
    Cea608Channel::Cc3,
    Cea608Channel::Cc4,
];

/// One caption stream found in the video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum DetectedCaption {
    /// A CEA-608 line-21 channel, `CC1` to `CC4`.
    Cea608(Cea608Channel),
    /// A CEA-708 DTVCC service, numbered from 1.
    Cea708Service(u8),
}

impl DetectedCaption {
    /// Short label for logs.
    pub fn name(&self) -> &'static str {
        match self {
            DetectedCaption::Cea608(channel) => channel.name(),
            DetectedCaption::Cea708Service(_) => "cea708-service",
        }
    }

    /// The HLS `INSTREAM-ID` this stream is advertised under.
    pub fn instream_id(&self) -> broadcast_hls::InstreamId {
        match self {
            DetectedCaption::Cea608(channel) => broadcast_hls::InstreamId::Cc(channel.number()),
            DetectedCaption::Cea708Service(n) => broadcast_hls::InstreamId::Service(*n),
        }
    }
}

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
    line21: Cea608Decoder,
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
            line21: Cea608Decoder::new(),
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
        self.line21
            .push_triplets(cc.triplets.iter().filter(|t| match t.cc_type {
                CcType::Ntsc608Field1 | CcType::Ntsc608Field2 => {
                    has_odd_parity(t.cc_data_1) && has_odd_parity(t.cc_data_2)
                }
                _ => false,
            }));
        self.dtvcc.push_triplets(cc.triplets.iter());
        for channel in CEA608_CHANNELS {
            if self.line21.has_caption_text(channel) {
                self.seen.insert(DetectedCaption::Cea608(channel));
            }
        }
        for service in self.dtvcc.services_seen() {
            self.seen.insert(DetectedCaption::Cea708Service(service));
        }
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
    const FIELD1: u8 = 0xFC;
    const FIELD2: u8 = 0xFD;
    const DTVCC_START: u8 = 0xFF;
    const DTVCC_DATA: u8 = 0xFE;

    fn odd(b: u8) -> u8 {
        if b.count_ones() % 2 == 1 { b } else { b | 0x80 }
    }

    fn pair(kind: u8, b1: u8, b2: u8) -> [u8; 3] {
        [kind, odd(b1), odd(b2)]
    }

    fn cc_data(triplets: &[[u8; 3]]) -> Vec<u8> {
        let mut out = vec![0xC0 | triplets.len() as u8, 0xFF];
        out.extend(triplets.iter().flatten());
        out.push(0xFF);
        out
    }

    fn mpeg2_picture(user_identifier: [u8; 4], type_code: u8, cc: &[u8]) -> Vec<u8> {
        let mut au = PICTURE_START.to_vec();
        au.extend_from_slice(&[0x00, 0x0F, 0xFF, 0xF8]);
        au.extend_from_slice(&USER_DATA_START);
        au.extend_from_slice(&user_identifier);
        au.push(type_code);
        au.extend_from_slice(cc);
        au
    }

    fn atsc_picture(triplets: &[[u8; 3]]) -> Vec<u8> {
        mpeg2_picture(
            *b"GA94",
            MPEG2_ATSC_CC_USER_DATA_TYPE_CODE,
            &cc_data(triplets),
        )
    }

    fn mpeg2_detector() -> CaptionDetector {
        CaptionDetector {
            carriage: Carriage::Mpeg2UserData,
            seen: BTreeSet::new(),
            line21: Cea608Decoder::new(),
            dtvcc: Cea708Decoder::new(),
        }
    }

    fn hi_on(first: u8, second: u8, field: u8) -> Vec<[u8; 3]> {
        vec![
            pair(field, first, 0x29),
            pair(field, b'H', b'I'),
            pair(field, second, 0x2F),
        ]
    }

    #[test]
    fn mpeg2_ga94_text_on_cc1_reports_cc1_only() {
        let mut d = mpeg2_detector();
        let au = atsc_picture(&hi_on(0x14, 0x14, FIELD1));
        assert!(d.observe(&au));
        assert_eq!(
            d.detected(),
            vec![DetectedCaption::Cea608(Cea608Channel::Cc1)]
        );
        assert!(!d.observe(&au), "a repeat is not new");
    }

    #[test]
    fn each_608_channel_is_told_apart_by_field_and_control_code() {
        let cases = [
            (FIELD1, 0x14, Cea608Channel::Cc1),
            (FIELD1, 0x1C, Cea608Channel::Cc2),
            (FIELD2, 0x15, Cea608Channel::Cc3),
            (FIELD2, 0x1D, Cea608Channel::Cc4),
        ];
        for (field, code, want) in cases {
            let mut d = mpeg2_detector();
            assert!(d.observe(&atsc_picture(&hi_on(code, code, field))));
            assert_eq!(
                d.detected(),
                vec![DetectedCaption::Cea608(want)],
                "{}",
                want.name()
            );
        }
    }

    #[test]
    fn control_codes_xds_and_null_padding_are_not_captions() {
        let mut d = mpeg2_detector();
        let au = atsc_picture(&[
            pair(FIELD1, 0x14, 0x20),
            pair(FIELD1, 0x00, 0x00),
            pair(FIELD2, 0x01, 0x03),
            pair(FIELD2, b'A', b'B'),
            pair(FIELD2, 0x0F, 0x2C),
        ]);
        assert!(!d.observe(&au));
        assert!(d.detected().is_empty());
    }

    #[test]
    fn bad_parity_pair_is_not_a_caption() {
        let mut d = mpeg2_detector();
        let mut au = atsc_picture(&hi_on(0x14, 0x14, FIELD1));
        let first = au.len() - 6;
        au[first] ^= 0x80;
        assert!(!d.observe(&au));
        assert!(d.detected().is_empty());
    }

    #[test]
    fn dtvcc_services_including_extended_are_reported() {
        let service_block = |header: &[u8], body: &[u8]| {
            let mut v = header.to_vec();
            v.extend_from_slice(body);
            v
        };
        let mut blocks = service_block(&[(1 << 5) | 1], b"A");
        blocks.extend(service_block(&[(7 << 5) | 1, 12], b"B"));
        let size_code = (blocks.len() + 1).div_ceil(2);
        let mut ccp = vec![size_code as u8];
        ccp.extend(&blocks);
        ccp.resize(size_code * 2, 0);
        let mut triplets = Vec::new();
        for (i, chunk) in ccp.chunks(2).enumerate() {
            let kind = if i == 0 { DTVCC_START } else { DTVCC_DATA };
            triplets.push([kind, chunk[0], chunk[1]]);
        }
        let mut d = mpeg2_detector();
        assert!(d.observe(&atsc_picture(&triplets)));
        assert_eq!(
            d.detected(),
            vec![
                DetectedCaption::Cea708Service(1),
                DetectedCaption::Cea708Service(12)
            ]
        );
    }

    #[test]
    fn mpeg2_other_identifier_or_type_reports_nothing() {
        let mut d = mpeg2_detector();
        let cc = cc_data(&hi_on(0x14, 0x14, FIELD1));
        assert!(!d.observe(&mpeg2_picture(
            *b"DTG1",
            MPEG2_ATSC_CC_USER_DATA_TYPE_CODE,
            &cc
        )));
        assert!(!d.observe(&mpeg2_picture(*b"GA94", 0x06, &cc)));
        assert!(d.detected().is_empty());
    }
}
