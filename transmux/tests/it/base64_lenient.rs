//! RFC 4648 §10 vectors, plus the leniency real SDP producers depend on.
use broadcast_common::Serialize;
use transmux::rtp::{base64_decode, base64_encode};
use transmux::rtp_sdp::avc_config_from_sprop;

const VECTORS: [(&str, &str); 7] = [
    ("", ""),
    ("f", "Zg=="),
    ("fo", "Zm8="),
    ("foo", "Zm9v"),
    ("foob", "Zm9vYg=="),
    ("fooba", "Zm9vYmE="),
    ("foobar", "Zm9vYmFy"),
];

#[test]
fn rfc4648_section_10_vectors_encode_and_decode() {
    for (plain, b64) in VECTORS {
        assert_eq!(base64_encode(plain.as_bytes()), b64);
        assert_eq!(base64_decode(b64).unwrap(), plain.as_bytes());
    }
}

#[test]
fn unpadded_input_decodes_like_padded() {
    for (plain, b64) in VECTORS {
        assert_eq!(
            base64_decode(b64.trim_end_matches('=')).unwrap(),
            plain.as_bytes()
        );
    }
}

/// `Zh==` carries non-zero trailing bits; the old decoder ignored them and
/// strict base64 rejects them. Real encoders occasionally emit them.
#[test]
fn non_zero_trailing_bits_are_tolerated() {
    assert_eq!(base64_decode("Zh==").unwrap(), b"f");
}

#[test]
fn invalid_bytes_and_whitespace_are_errors() {
    for bad in ["Zm9v!", "Zm 9v", "Zm9v\n", "-_-_"] {
        assert!(base64_decode(bad).is_err(), "{bad:?}");
    }
}

/// Intentional tightening (CHANGELOG): the old decoder stripped every `=`
/// wherever it appeared, so `Zm9v=YmFy` decoded; a stray `=` inside the data is
/// now an error.
#[test]
fn stray_equals_inside_the_data_is_an_error() {
    assert!(base64_decode("Zm9v=YmFy").is_err());
}

/// The real ffmpeg High-profile `sprop-parameter-sets` (fixture
/// `tests/fixtures/rtp/high-ffmpeg.sdp`) with its padding removed must give a
/// byte-identical avcC.
#[test]
fn unpadded_real_sprop_gives_identical_avcc() {
    let padded = "Z2QADazZQUH7ARAAAAMAEAAAAwMg8UKZYA==,aOvjyyLA";
    let unpadded = "Z2QADazZQUH7ARAAAAMAEAAAAwMg8UKZYA,aOvjyyLA";
    let ser = |s: &str| {
        let cfg = avc_config_from_sprop(s).expect("sprop");
        let mut out = vec![0u8; cfg.config.serialized_len()];
        cfg.config.serialize_into(&mut out).unwrap();
        out
    };
    assert_eq!(ser(unpadded), ser(padded));
}

/// Intentional tightenings vs the pre-crate decoder (CHANGELOG): input whose
/// length is 1 mod 4 (`QUJDR`) cannot be base64 (RFC 4648) and used to lose its
/// last character silently; excess padding (`Zg===`) used to be stripped.
#[test]
fn truncated_last_quantum_and_excess_padding_are_errors() {
    for bad in ["QUJDR", "Z", "Zm9vY", "Zg===", "Zm8==", "Zm9v===="] {
        assert!(base64_decode(bad).is_err(), "{bad:?}");
    }
}
