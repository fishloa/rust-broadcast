//! `CodecPrivateData` hex and attribute bounds (remote alloc-DoS defense).

use broadcast_smooth::{
    MAX_CODEC_PRIVATE_DATA_HEX_LEN, SmoothManifest, SmoothParseError, default_limits, hex_decode,
};

#[test]
fn hex_decode_accepts_exactly_the_cap_and_rejects_longer() {
    assert_eq!(MAX_CODEC_PRIVATE_DATA_HEX_LEN, 65_536);
    let at_cap = "0".repeat(65_536);
    assert_eq!(hex_decode(&at_cap).unwrap().len(), 32_768);
    let over_cap = "0".repeat(65_538);
    assert_eq!(
        hex_decode(&over_cap).unwrap_err(),
        SmoothParseError::CodecPrivateDataTooLong { len: 65_538 }
    );
}

#[test]
fn default_limits_admit_twice_the_codec_private_data_cap() {
    assert_eq!(default_limits().max_attribute_bytes, 131_072);
    // A CodecPrivateData of exactly 2 * MAX chars passes the generic
    // attribute bound, so the typed CodecPrivateDataTooLong fires (not
    // TooLarge).
    let cpd = "0".repeat(131_072);
    let xml = format!(
        r#"<SmoothStreamingMedia MajorVersion="2" MinorVersion="0" TimeScale="100"><StreamIndex Type="video" Url="u"><QualityLevel Index="0" Bitrate="1" FourCC="H264" CodecPrivateData="{cpd}"/></StreamIndex></SmoothStreamingMedia>"#
    );
    assert_eq!(
        SmoothManifest::parse(&xml).unwrap_err(),
        SmoothParseError::CodecPrivateDataTooLong { len: 131_072 }
    );
}
