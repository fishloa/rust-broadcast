//! Behavioural proof for issue #1007 (r03-C6): a descriptor whose body does
//! not fit the 8-bit `descriptor_length` must make `serialize_into` return
//! `Err`, never `Ok` with a wrapped length followed by the whole body (which
//! misframes the rest of the enclosing descriptor loop). Each case also pins
//! that the widest legal body (255 bytes) still serializes and re-parses, so a
//! guard that rejects everything cannot pass. The lexical scan in
//! `serializer_length_truncation.rs` covers the remaining sites.

use broadcast_common::{Parse, Serialize};
use dvb_si::descriptors::iso_639_language::{AudioType, Iso639LanguageDescriptor, LanguageEntry};
use dvb_si::descriptors::network_name::NetworkNameDescriptor;
use dvb_si::descriptors::registration::RegistrationDescriptor;
use dvb_si::text::{DvbText, LangCode};

fn ser<T: Serialize<Error = dvb_si::error::Error>>(d: &T) -> Result<Vec<u8>, dvb_si::error::Error> {
    let mut buf = vec![0u8; d.serialized_len()];
    d.serialize_into(&mut buf)?;
    Ok(buf)
}

#[test]
fn iso_639_64_entries_is_rejected_not_wrapped() {
    let entry = || LanguageEntry {
        language_code: LangCode(*b"eng"),
        audio_type: AudioType::Undefined,
    };
    // 64 entries * 4 bytes = 256 > 255: the issue's scenario.
    let bad = Iso639LanguageDescriptor {
        entries: (0..64).map(|_| entry()).collect(),
    };
    assert!(ser(&bad).is_err());
    // 63 entries = 252 bytes is the widest legal body.
    let ok = Iso639LanguageDescriptor {
        entries: (0..63).map(|_| entry()).collect(),
    };
    let bytes = ser(&ok).unwrap();
    assert_eq!(bytes[1], 252);
    assert_eq!(Iso639LanguageDescriptor::parse(&bytes).unwrap(), ok);
}

#[test]
fn network_name_256_bytes_is_rejected_not_wrapped() {
    let long = vec![b'n'; 256];
    let bad = NetworkNameDescriptor {
        network_name: DvbText::new(&long),
    };
    assert!(ser(&bad).is_err());
    let max = vec![b'n'; 255];
    let ok = NetworkNameDescriptor {
        network_name: DvbText::new(&max),
    };
    let bytes = ser(&ok).unwrap();
    assert_eq!(bytes[1], 255);
    assert_eq!(bytes.len(), 257);
}

#[test]
fn registration_256_byte_body_is_rejected_not_wrapped() {
    let info = vec![0u8; 253]; // 4-byte format_identifier + 253 = 257
    let bad = RegistrationDescriptor {
        format_identifier: *b"CUEI",
        additional_identification_info: &info,
    };
    assert!(ser(&bad).is_err());
    let info = vec![0u8; 251]; // 255
    let ok = RegistrationDescriptor {
        format_identifier: *b"CUEI",
        additional_identification_info: &info,
    };
    let bytes = ser(&ok).unwrap();
    assert_eq!(bytes[1], 255);
}
