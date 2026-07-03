//! HLS Sample-AES + full-segment AES-128 integration tests (issue #479).
//!
//! Gated on the `sample-aes` feature (the crypto path). Each test *bites*:
//! - the AES KAT pins the block cipher to the published NIST vector (no stub);
//! - the sample tests assert the exact clear/encrypted byte partition, not just
//!   round-trip (a raw-passthrough "encrypt" would leave the body clear and fail);
//! - the emulation-prevention test round-trips a NAL containing `00 00 03`;
//! - the EXT-X-KEY tests pin the exact tag strings.
#![cfg(feature = "sample-aes")]

use transmux::sample_aes::{
    self, BLOCK_LEN, ExtXKey, H264_CLEAR_PREFIX_LEN, H264_MIN_ENCRYPTED_NAL_LEN, KEY_LEN,
    aac_decrypt_frame, aac_encrypt_frame, ac3_decrypt_frame, ac3_encrypt_frame,
    aes128_decrypt_segment, aes128_encrypt_segment, format_iv, h264_decrypt_nal, h264_encrypt_nal,
    iv_from_sequence_number,
};

// NIST SP 800-38A F.2.1/F.2.2 CBC-AES128 known-answer vector.
const KEY: [u8; KEY_LEN] = [
    0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf, 0x4f, 0x3c,
];
const IV: [u8; BLOCK_LEN] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
];

/// Test 1 — AES-128-CBC known-answer test via the full-segment API.
///
/// Encrypting exactly one 16-byte plaintext block yields (first block ==) the
/// published NIST ciphertext, proving real AES rather than a stub. The trailing
/// PKCS#7 padding block is dropped on decrypt, recovering the plaintext.
#[test]
fn aes128_cbc_known_answer_vector() {
    const PT: [u8; 16] = [
        0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93, 0x17,
        0x2a,
    ];
    const CT: [u8; 16] = [
        0x76, 0x49, 0xab, 0xac, 0x81, 0x19, 0xb2, 0x46, 0xce, 0xe9, 0x8e, 0x9b, 0x12, 0xe9, 0x19,
        0x7d,
    ];
    // Full-segment encrypt of one block → NIST CT block + one PKCS#7 pad block.
    let ct = aes128_encrypt_segment(&KEY, &IV, &PT);
    assert_eq!(ct.len(), 32, "one block + one full pad block");
    assert_eq!(&ct[..16], &CT, "first ciphertext block != NIST vector");
    let pt = aes128_decrypt_segment(&KEY, &IV, &ct).unwrap();
    assert_eq!(pt, PT, "KAT decrypt did not recover plaintext");
}

/// Test 2 — H.264 sample: clear leader, skip pattern, ≤48 untouched, and an
/// emulation-prevention NAL round-trip.
#[test]
fn h264_sample_pattern_and_round_trip() {
    // NAL type 5 (IDR), 200 bytes, payload chosen so escaped == raw.
    let mut nal = vec![0x65u8];
    nal.extend(core::iter::repeat_n(0xAAu8, 199));
    assert_eq!(nal.len(), 200);

    let enc = h264_encrypt_nal(&KEY, &IV, &nal);

    // First 32 bytes (1 hdr + 31 payload) unchanged.
    assert_eq!(&enc[..H264_CLEAR_PREFIX_LEN], &nal[..H264_CLEAR_PREFIX_LEN]);

    // Pattern offsets over 200 raw bytes:
    //   block1 = [32,48) ENC, skip = [48,192) CLEAR, tail = [192,200) CLEAR (<16).
    assert_ne!(&enc[32..48], &nal[32..48], "block1 must be encrypted");
    assert_eq!(&enc[48..192], &nal[48..192], "skip region must be clear");
    assert_eq!(&enc[192..200], &nal[192..200], "trailing <16 clear");

    let dec = h264_decrypt_nal(&KEY, &IV, &enc);
    assert_eq!(dec, nal, "H.264 round-trip mismatch");

    // A NAL of type 5 but <= 48 bytes is left entirely clear.
    let mut short = vec![0x65u8];
    short.extend((0u8..40).map(|i| i.wrapping_mul(9)));
    assert!(short.len() <= H264_MIN_ENCRYPTED_NAL_LEN);
    assert_eq!(
        h264_encrypt_nal(&KEY, &IV, &short),
        short,
        "short NAL untouched"
    );

    // A long non-slice NAL (type 7 SPS) is never encrypted.
    let mut sps = vec![0x67u8];
    sps.extend(core::iter::repeat_n(0x11u8, 80));
    assert_eq!(h264_encrypt_nal(&KEY, &IV, &sps), sps, "SPS untouched");

    // Emulation-prevention: a NAL whose payload contains 00 00 03 sequences
    // round-trips byte-identically (strip/reinsert correct).
    // Each group ends in a non-zero byte so no illegal 00-00-00 run spans
    // groups; `00 00 03 01` is a canonical emulation-prevention sequence.
    let mut emu = vec![0x65u8];
    for _ in 0..30 {
        emu.extend_from_slice(&[0xAA, 0x00, 0x00, 0x03, 0x01, 0x9C]);
    }
    assert!(emu.len() > H264_MIN_ENCRYPTED_NAL_LEN);
    let enc_emu = h264_encrypt_nal(&KEY, &IV, &emu);
    let dec_emu = h264_decrypt_nal(&KEY, &IV, &enc_emu);
    assert_eq!(dec_emu, emu, "emulation-prevention NAL round-trip mismatch");
}

/// Test 3 — AAC ADTS frame: header + 16-byte leader clear, CBC body, round-trip.
#[test]
fn aac_frame_round_trip_clear_leader() {
    // byte1 bit0 = 1 → 7-byte header (no CRC).
    let mut frame = vec![0xFF, 0xF1, 0x00, 0x00, 0x00, 0x00, 0x00];
    frame.extend((0u8..100).map(|i| i.wrapping_mul(3)));

    let enc = aac_encrypt_frame(&KEY, &IV, &frame).unwrap();
    assert_eq!(enc.len(), frame.len(), "AAC has no padding");
    // 7-byte header + 16-byte leader = 23 clear bytes.
    let clear = 7 + 16;
    assert_eq!(&enc[..clear], &frame[..clear], "leader clear");
    assert_ne!(&enc[clear..], &frame[clear..], "body encrypted");

    let dec = aac_decrypt_frame(&KEY, &IV, &enc).unwrap();
    assert_eq!(dec, frame, "AAC round-trip mismatch");
}

/// AC-3 frame: 16-byte leader clear, CBC body, round-trip.
#[test]
fn ac3_frame_round_trip() {
    let mut frame = vec![0x0B, 0x77]; // AC-3 syncword
    frame.extend((0u8..90).map(|i| i.wrapping_add(5)));
    let enc = ac3_encrypt_frame(&KEY, &IV, &frame);
    assert_eq!(&enc[..16], &frame[..16], "16-byte leader clear");
    assert_eq!(ac3_decrypt_frame(&KEY, &IV, &enc), frame);
}

/// Test 4 — AES-128 full-segment: PKCS#7-padded, block-aligned length, round-trip.
#[test]
fn aes128_full_segment_padded_round_trip() {
    let segment: Vec<u8> = (0u16..300).map(|i| i as u8).collect(); // 300 bytes
    let ct = aes128_encrypt_segment(&KEY, &IV, &segment);
    assert_eq!(ct.len() % BLOCK_LEN, 0, "ciphertext block-padded");
    assert_eq!(ct.len(), 304, "300 → 304 (next multiple of 16)");
    assert_ne!(&ct[..segment.len()], &segment[..], "actually encrypted");
    let pt = aes128_decrypt_segment(&KEY, &IV, &ct).unwrap();
    assert_eq!(pt, segment, "full-segment round-trip mismatch");
    // Malformed length rejected (no panic).
    assert!(aes128_decrypt_segment(&KEY, &IV, &[0u8; 17]).is_err());
}

/// Test 5 — EXT-X-KEY exact tag strings + IV-from-sequence-number formatting.
#[test]
fn ext_x_key_exact_strings() {
    let sae = ExtXKey::fairplay_sample_aes("skd://asset-42");
    assert_eq!(
        sae.to_tag(),
        "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://asset-42\",\
         KEYFORMAT=\"com.apple.streamingkeydelivery\",KEYFORMATVERSIONS=\"1\""
    );

    let aes = ExtXKey::aes128(
        "https://keyserver.example.com/key",
        [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x01,
        ],
    );
    assert_eq!(
        aes.to_tag(),
        "#EXT-X-KEY:METHOD=AES-128,URI=\"https://keyserver.example.com/key\",\
         IV=0x00000000000000000000000000000001"
    );

    // Display == to_tag.
    assert_eq!(format!("{aes}"), aes.to_tag());

    // IV-when-absent = media sequence number as u128 BE.
    let iv = iv_from_sequence_number(7);
    assert_eq!(iv, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7]);
    assert_eq!(format_iv(&iv), "0x00000000000000000000000000000007");

    // Method label helper.
    assert_eq!(
        sample_aes::HlsEncryptionMethod::SampleAes.name(),
        "SAMPLE-AES"
    );
    assert_eq!(sample_aes::HlsEncryptionMethod::Aes128.name(), "AES-128");
}
