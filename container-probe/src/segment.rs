//! Structural "is this body a media segment" checks, for a fetched HLS/DASH segment body that has
//! to be told apart from an error page, a truncated transfer or a progressive file.
//!
//! These are not format probes: [`probe`](crate::probe) names a container, these answer whether a
//! body that is *expected* to be a segment of a given kind has that kind's structure.
//!
//! - [`ts_structure`]: MPEG-2 TS (ISO/IEC 13818-1 §2.4.3.2, 188-byte packets, `sync_byte` 0x47
//!   first) at some packet phase.
//! - [`media_fragment`]: a complete `moof` followed by its complete `mdat` (ISO/IEC 14496-12
//!   §8.8.4, §8.1.1), what every fMP4/CMAF media segment or part carries.
//! - [`progressive_movie`]: a `moov` with no `moof`, a progressive MP4 rather than a fragmented one.

use crate::isobmff::{
    BOX_HEADER_LARGESIZE_SIZE, BOX_HEADER_MIN_SIZE, SIZE_INDICATES_LARGESIZE, SIZE_TO_EOF,
    TYPE_MDAT, TYPE_MOOF, TYPE_MOOV,
};
use crate::ts::{TS_PACKET_SIZE, TS_SYNC_BYTE};

/// The `uuid` box type, whose header carries a 16-byte extended type (ISO/IEC 14496-12 §4.2).
const TYPE_UUID: [u8; 4] = *b"uuid";
/// Length of the extended type following a `uuid` box header (§4.2).
const UUID_USERTYPE_LEN: usize = 16;
/// Byte offset of the box type within a box header (§4.2).
const TYPE_OFFSET: usize = 4;
/// Byte offset of the 64-bit `largesize` within a box header (§4.2).
const LARGESIZE_OFFSET: usize = 8;
/// Lanes at or above which a TS body needs only half of them synced.
const TS_TRUSTED_LANES: usize = 3;

/// `true` when `bytes` is MPEG-TS at some packet phase (ISO/IEC 13818-1 §2.4.3.2, 188-byte
/// packets, `sync_byte` first): a body cut mid-packet or led by junk is still media, a demuxer
/// resynchronises. At the best phase, a body of at least three lanes needs half of them synced; a
/// shorter one needs every lane synced, and a single lane is trusted only at phase 0 (a one-packet
/// body has up to 188 phases, so noise would match one). An HTML error page, random bytes and an
/// empty body all fail; a segment of nothing but null packets passes. 192-byte M2TS packets are
/// not accepted.
pub fn ts_structure(bytes: &[u8]) -> bool {
    (0..TS_PACKET_SIZE).any(|phase| {
        let lanes = bytes.len().saturating_sub(phase) / TS_PACKET_SIZE;
        let lanes_synced = (0..lanes)
            .filter(|lane| bytes[phase + lane * TS_PACKET_SIZE] == TS_SYNC_BYTE)
            .count();
        match lanes {
            0 => false,
            1 => phase == 0 && lanes_synced == 1,
            n if n < TS_TRUSTED_LANES => lanes_synced == n,
            n => lanes_synced.saturating_mul(2) >= n,
        }
    })
}

/// `true` when `bytes` holds a complete `moof` box followed, anywhere later at the top level, by a
/// complete `mdat` box (ISO/IEC 14496-12 §8.8.4, §8.1.1). Any leading box (`styp`, `prft`, `emsg`,
/// `free`, `sidx`) is skipped; a malformed or truncated box ends the walk with `false`.
pub fn media_fragment(bytes: &[u8]) -> bool {
    let mut fragment = false;
    for step in TopLevelBoxes::new(bytes) {
        let Some(box_type) = step else { return false };
        match box_type {
            TYPE_MOOF => fragment = true,
            TYPE_MDAT if fragment => return true,
            _ => {}
        }
    }
    false
}

/// `true` when the top-level boxes of `bytes` include a `moov` and no `moof` before the walk ends:
/// a progressive MP4, not a media segment of fragmented MP4. A malformed or truncated box ends the
/// walk; what was seen before it decides.
pub fn progressive_movie(bytes: &[u8]) -> bool {
    let mut movie = false;
    for step in TopLevelBoxes::new(bytes) {
        let Some(box_type) = step else { break };
        match box_type {
            TYPE_MOOF => return false,
            TYPE_MOOV => movie = true,
            _ => {}
        }
    }
    movie
}

struct TopLevelBoxes<'a> {
    remaining: &'a [u8],
}

impl<'a> TopLevelBoxes<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { remaining: bytes }
    }
}

impl Iterator for TopLevelBoxes<'_> {
    /// `None` is a malformed or truncated box; the walk ends after yielding it.
    type Item = Option<[u8; 4]>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining.is_empty() {
            return None;
        }
        match next_box(self.remaining) {
            Some((box_type, consumed)) => {
                self.remaining = &self.remaining[consumed.min(self.remaining.len())..];
                Some(Some(box_type))
            }
            None => {
                self.remaining = &[];
                Some(None)
            }
        }
    }
}

/// The type and total length of the box at the start of `bytes` (a to-EOF box runs to the end of
/// `bytes`), or `None` when its header is truncated or inconsistent or its payload is not all
/// present (ISO/IEC 14496-12 §4.2).
fn next_box(bytes: &[u8]) -> Option<([u8; 4], usize)> {
    if bytes.len() < BOX_HEADER_MIN_SIZE {
        return None;
    }
    let raw_size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let box_type = [
        bytes[TYPE_OFFSET],
        bytes[TYPE_OFFSET + 1],
        bytes[TYPE_OFFSET + 2],
        bytes[TYPE_OFFSET + 3],
    ];
    let mut header = BOX_HEADER_MIN_SIZE;
    let size = if raw_size == SIZE_INDICATES_LARGESIZE {
        let large = bytes.get(LARGESIZE_OFFSET..BOX_HEADER_LARGESIZE_SIZE)?;
        header = BOX_HEADER_LARGESIZE_SIZE;
        u64::from_be_bytes(large.try_into().ok()?)
    } else {
        u64::from(raw_size)
    };
    if size != u64::from(SIZE_TO_EOF) && size < header as u64 {
        return None;
    }
    if box_type == TYPE_UUID {
        if bytes.len() < header + UUID_USERTYPE_LEN {
            return None;
        }
        header += UUID_USERTYPE_LEN;
    }
    if size == u64::from(SIZE_TO_EOF) {
        return Some((box_type, bytes.len()));
    }
    let total = usize::try_from(size).ok()?;
    (total >= header && total <= bytes.len()).then_some((box_type, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;
    use std::vec::Vec;

    fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = u32::try_from(8 + body.len())
            .unwrap()
            .to_be_bytes()
            .to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(body);
        b
    }

    #[test]
    fn real_cmaf_fragments_are_media_fragments_and_inits_are_not() {
        for rel in [
            "hls/cmaf-fmp4/index0.m4s",
            "hls/cmaf-fmp4/index1.m4s",
            "dash/chunk-stream0-00001.m4s",
            "mp4/llcmaf/ll_chunked.mp4",
        ] {
            let bytes = test_fixtures::read(rel);
            assert!(media_fragment(&bytes), "{rel}");
            assert!(!progressive_movie(&bytes), "{rel}");
        }
        let init = test_fixtures::read("hls/cmaf-fmp4/init.mp4");
        assert!(!media_fragment(&init));
        assert!(
            progressive_movie(&init),
            "an init segment is a moov with no moof"
        );
    }

    #[test]
    fn a_real_progressive_mp4_is_progressive_and_not_a_fragment() {
        let bytes = test_fixtures::read("mp4/progressive/av_faststart.mp4");
        assert!(progressive_movie(&bytes));
        assert!(!media_fragment(&bytes));
    }

    #[test]
    fn a_moov_followed_by_a_moof_is_not_progressive() {
        let mut body = boxed(b"moov", &[]);
        body.extend(boxed(b"moof", &[]));
        assert!(!progressive_movie(&body));
    }

    #[test]
    fn a_progressive_walk_that_hits_a_truncated_box_keeps_what_it_saw() {
        let mut body = boxed(b"moov", &[]);
        body.extend_from_slice(&[0, 0, 0, 40, b'm', b'd', b'a', b't']);
        assert!(progressive_movie(&body));
    }

    #[test]
    fn fragments_after_any_leading_boxes_or_in_pairs_are_media() {
        let moof = boxed(b"moof", &[]);
        let mdat = boxed(b"mdat", &[0; 4]);
        for lead in [
            vec![boxed(b"styp", b"msdh")],
            vec![boxed(b"styp", b"msdh"), boxed(b"prft", &[0; 24])],
            vec![boxed(b"emsg", &[0; 12])],
            vec![boxed(b"free", &[0; 3])],
            vec![boxed(b"sidx", &[0; 20])],
            vec![],
        ] {
            let mut body: Vec<u8> = lead.concat();
            body.extend_from_slice(&moof);
            body.extend_from_slice(&mdat);
            assert!(media_fragment(&body));
        }
        assert!(media_fragment(
            &[moof.clone(), mdat.clone()].concat().repeat(3)
        ));
    }

    #[test]
    fn incomplete_or_misordered_fragments_are_not_media() {
        let moof = boxed(b"moof", &[]);
        let mdat = boxed(b"mdat", &[0; 4]);
        assert!(!media_fragment(&moof), "moof only");
        assert!(
            !media_fragment(&[mdat.clone(), moof.clone()].concat()),
            "mdat first"
        );
        assert!(
            !media_fragment(&[moof.clone(), mdat[..mdat.len() - 1].to_vec()].concat()),
            "truncated mdat"
        );
        assert!(!media_fragment(&[]));
        assert!(!media_fragment(&[0; 7]));
        assert!(!media_fragment(b"<html>503 service unavailable</html>"));
    }

    #[test]
    fn box_headers_are_decoded_like_a_full_iso_bmff_parser() {
        let mut large = vec![0, 0, 0, 1];
        large.extend_from_slice(b"mdat");
        large.extend_from_slice(&20u64.to_be_bytes());
        large.extend_from_slice(&[0; 4]);
        assert_eq!(next_box(&large), Some((*b"mdat", 20)));
        assert_eq!(next_box(&large[..19]), None, "largesize payload cut short");
        assert_eq!(next_box(&large[..15]), None, "largesize field cut short");

        let mut under = vec![0, 0, 0, 1];
        under.extend_from_slice(b"mdat");
        under.extend_from_slice(&8u64.to_be_bytes());
        assert_eq!(next_box(&under), None, "largesize smaller than its header");

        let to_eof = [&[0, 0, 0, 0][..], b"mdat", &[1, 2, 3]].concat();
        assert_eq!(next_box(&to_eof), Some((*b"mdat", to_eof.len())));

        let mut uuid = 24u32.to_be_bytes().to_vec();
        uuid.extend_from_slice(b"uuid");
        uuid.extend_from_slice(&[7; 16]);
        assert_eq!(next_box(&uuid), Some((*b"uuid", 24)));
        assert_eq!(next_box(&uuid[..23]), None, "uuid usertype cut short");
        let mut short_uuid = 16u32.to_be_bytes().to_vec();
        short_uuid.extend_from_slice(b"uuid");
        short_uuid.extend_from_slice(&[7; 16]);
        assert_eq!(
            next_box(&short_uuid),
            None,
            "size smaller than the uuid header"
        );

        assert_eq!(next_box(&[0, 0, 0, 7, b'f', b'r', b'e', b'e']), None);
        assert_eq!(
            next_box(&[0, 0, 0, 8, b'f', b'r', b'e', b'e']),
            Some((*b"free", 8))
        );
    }

    fn null_packet() -> [u8; TS_PACKET_SIZE] {
        let mut packet = [0xFF; TS_PACKET_SIZE];
        packet[..4].copy_from_slice(&[TS_SYNC_BYTE, 0x1F, 0xFF, 0x10]);
        packet
    }

    #[test]
    fn real_ts_is_structured_at_any_packet_phase() {
        let whole = test_fixtures::read("ts/h264_aac.ts");
        const PROBE_PACKETS: usize = 40;
        const CUT: usize = 100;
        let aligned = &whole[..PROBE_PACKETS * TS_PACKET_SIZE];
        let mut junk = vec![0xA5; TS_PACKET_SIZE - 1];
        junk.extend_from_slice(aligned);
        assert!(ts_structure(aligned));
        assert!(ts_structure(
            &whole[CUT..CUT + PROBE_PACKETS * TS_PACKET_SIZE]
        ));
        assert!(ts_structure(&junk));
        assert!(ts_structure(&whole[CUT..]));
        assert!(ts_structure(&test_fixtures::read(
            "container-probe/ts_midpacket_phase.ts"
        )));
    }

    #[test]
    fn tiny_ts_bodies_need_every_lane_synced() {
        let whole = test_fixtures::read("ts/h264_aac.ts");
        assert!(ts_structure(&whole[..TS_PACKET_SIZE]), "one packet");
        assert!(ts_structure(&whole[..2 * TS_PACKET_SIZE]), "two packets");
        assert!(ts_structure(&null_packet().repeat(3)), "only null packets");
        let mut second_lane_broken = whole[..2 * TS_PACKET_SIZE].to_vec();
        second_lane_broken[TS_PACKET_SIZE] = 0;
        assert!(!ts_structure(&second_lane_broken));
        let mut one_lane_late = vec![0u8; 3];
        one_lane_late.extend_from_slice(&whole[..TS_PACKET_SIZE]);
        assert!(
            !ts_structure(&one_lane_late),
            "one lane is only trusted at phase 0"
        );
    }

    #[test]
    fn a_lattice_below_half_sync_is_not_ts() {
        let mut body = vec![0u8; 4 * TS_PACKET_SIZE];
        body[0] = TS_SYNC_BYTE;
        assert!(!ts_structure(&body));
        body[2 * TS_PACKET_SIZE] = TS_SYNC_BYTE;
        assert!(ts_structure(&body), "exactly half synced passes");
    }

    #[test]
    fn garbage_html_empty_and_a_lone_sync_byte_are_never_ts() {
        assert!(!ts_structure(&[]));
        assert!(!ts_structure(&[TS_SYNC_BYTE]));
        assert!(!ts_structure(b"<html>503</html>"));
        let garbage: Vec<u8> = (0..4096usize)
            .map(|i| u8::try_from(i.wrapping_mul(37) % 251).unwrap())
            .collect();
        assert!(!ts_structure(&garbage));
        for len in [0usize, 1, 187, 188, 189, 375, 376, 377, 564, 4096, 65_536] {
            let body: Vec<u8> = (0..len)
                .map(|i| u8::try_from(i.wrapping_mul(2_654_435_761) >> 13 & 0xFF).unwrap())
                .collect();
            assert!(!ts_structure(&body), "{len} random bytes");
        }
    }

    #[test]
    fn a_real_fmp4_body_is_not_ts() {
        assert!(!ts_structure(&test_fixtures::read(
            "hls/cmaf-fmp4/index0.m4s"
        )));
    }
}
