//! SCTE 35 §9.6.1: `splice_command_length` = 0xFFF is the deprecated
//! backwards-compatibility value that "shall be ignored by downstream
//! equipment". Every CRC-valid seed section is rewritten to carry the sentinel
//! (CRC repaired) and must still parse to the SAME typed command / descriptor
//! loop, then re-serialize to the sentinel-carrying bytes unchanged. The real
//! TSDuck capture exercising the same path is in `real_capture.rs`
//! (`test-091.ts`, private fixtures only).

use base64::{Engine, engine::general_purpose::STANDARD};
use broadcast_common::{Parse, Serialize, crc32_mpeg2};
use scte35_splice::SpliceInfoSection;

const SEEDS: &[&str] = &[
    "/DA0AAAAAAAA///wBQb+cr0AUAAeAhxDVUVJSAAAjn/PAAGlmbAICAAAAAAsoKGKNAIAmsnRfg==",
    "/DARAAAAAAAAAP/wAAAAAHpPv/8=",
    "/DAvAAAAAAAA///wBQb+dGKQoAAZAhdDVUVJSAAAjn+fCAgAAAAALKChijUCAKnMZ1g=",
    "/DAvAAAAAAAA///wFAVIAACPf+/+c2nALv4AUsz1AAAAAAAKAAhDVUVJAAABNWLbowo=",
    "/DAvAAAAAAAA///wBQb+rvF8TAAZAhdDVUVJSAAAB3+fCAgAAAAALKVslxEAAMSHai4=",
    "/DBhAAAAAAAA///wBQb+qM1E7QBLAhdDVUVJSAAArX+fCAgAAAAALLLXnTUCAAIXQ1VFSUgAACZ/nwgIAAAAACyy150RAAACF0NVRUlIAAAnf58ICAAAAAAsstezEAAAihiGnw==",
];

fn with_sentinel(mut b: Vec<u8>) -> Vec<u8> {
    b[11] |= 0x0F;
    b[12] = 0xFF;
    let n = b.len() - 4;
    let crc = crc32_mpeg2::compute(&b[..n]);
    b[n..].copy_from_slice(&crc.to_be_bytes());
    b
}

#[test]
fn sentinel_sections_parse_like_the_real_length_and_round_trip() {
    let mut checked = 0;
    for seed in SEEDS {
        let bytes = STANDARD.decode(seed).unwrap();
        let plain = SpliceInfoSection::parse(&bytes).expect("seed parses");
        let ignored = with_sentinel(bytes.clone());
        let s = SpliceInfoSection::parse(&ignored)
            .unwrap_or_else(|e| panic!("0xFFF sentinel must be ignored, got {e:?}"));
        let (a, b) = (plain.clear.as_ref().unwrap(), s.clear.as_ref().unwrap());
        assert_eq!(a.command, b.command, "same typed command");
        assert_eq!(a.descriptor_loop, b.descriptor_loop);
        assert!(b.legacy_command_length_sentinel && !a.legacy_command_length_sentinel);
        assert_eq!(
            s.try_to_bytes().unwrap(),
            ignored,
            "the sentinel must be preserved on re-serialize"
        );
        // Serializing the plain parse still writes the real length.
        assert_eq!(plain.try_to_bytes().unwrap(), bytes);
        checked += 1;
    }
    assert!(checked >= 5, "{checked}");
}

#[test]
fn a_sentinel_with_alignment_stuffing_is_an_error_not_a_guess() {
    // splice_null + 2 stuffing bytes: with the real length (0) it parses and
    // keeps the stuffing; with the 0xFFF sentinel the command extent cannot be
    // derived exactly (the descriptor loop no longer ends at the CRC), so it
    // is rejected rather than guessed.
    let seed = STANDARD.decode(SEEDS[1]).unwrap();
    let mut b = seed[..seed.len() - 4].to_vec();
    b.extend_from_slice(&[0xFF, 0xFF]);
    let section_length = (((u16::from(b[1] & 0x0F) << 8) | u16::from(b[2])) + 2).to_be_bytes();
    b[1] = (b[1] & 0xF0) | (section_length[0] & 0x0F);
    b[2] = section_length[1];
    b.extend_from_slice(&[0; 4]);
    let recrc = |v: &mut Vec<u8>| {
        let n = v.len() - 4;
        let crc = crc32_mpeg2::compute(&v[..n]);
        v[n..].copy_from_slice(&crc.to_be_bytes());
    };
    recrc(&mut b);
    let plain = SpliceInfoSection::parse(&b).expect("real length 0 + stuffing parses");
    assert_eq!(
        plain.clear.as_ref().unwrap().alignment_stuffing,
        [0xFF, 0xFF]
    );
    b[11] |= 0x0F;
    b[12] = 0xFF;
    recrc(&mut b);
    assert!(SpliceInfoSection::parse(&b).is_err());
}
