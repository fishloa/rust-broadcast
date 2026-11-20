//! Behavioural proof for issue #1007 (r03-C6): a descriptor whose body does
//! not fit the 8-bit `descriptor_length` must make `serialize_into` return
//! `Err`, never `Ok` with a wrapped length followed by the whole body (which
//! misframes the rest of the enclosing descriptor loop). Each case also pins
//! that the widest legal body (255 bytes) still serializes and re-parses, so a
//! guard that rejects everything cannot pass. The lexical scan in
//! `serializer_length_truncation.rs` covers the remaining sites.

use broadcast_common::{Parse, Serialize};
use dvb_si::descriptors::announcement_support::{
    AnnouncementEntry, AnnouncementReference, AnnouncementSupportDescriptor, AnnouncementType,
    ReferenceType,
};
use dvb_si::descriptors::carousel_identifier::{CarouselIdentifierDescriptor, FormatSpecifier};
use dvb_si::descriptors::cell_frequency_link::{
    CellFrequencyLinkDescriptor, CellFrequencyLinkEntry, CellFrequencyLinkSubcell,
};
use dvb_si::descriptors::cell_list::{CellListDescriptor, CellListEntry, CellListSubcell};
use dvb_si::descriptors::dts::DtsDescriptor;
use dvb_si::descriptors::iso_639_language::{AudioType, Iso639LanguageDescriptor, LanguageEntry};
use dvb_si::descriptors::network_name::NetworkNameDescriptor;
use dvb_si::descriptors::registration::RegistrationDescriptor;
use dvb_si::descriptors::vbi_data::{VbiDataDescriptor, VbiDataEntry, VbiService};
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
    assert!(matches!(
        &(ser(&bad)),
        Err(dvb_si::error::Error::FieldOverflow(_))
    ));
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
    assert!(matches!(ser(&bad), Err(dvb_si::Error::FieldOverflow(..))));
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
    assert!(matches!(ser(&bad), Err(dvb_si::Error::FieldOverflow(..))));
    let info = vec![0u8; 251]; // 255
    let ok = RegistrationDescriptor {
        format_identifier: *b"CUEI",
        additional_identification_info: &info,
    };
    let bytes = ser(&ok).unwrap();
    assert_eq!(bytes[1], 255);
}

#[test]
fn dts_255_byte_body_is_the_widest_legal() {
    // 5 fixed bytes + 250 additional_info = 255.
    let info = vec![0x5Au8; 250];
    let ok = DtsDescriptor {
        sample_rate_code: 0x0D,
        bit_rate_code: 0x0A,
        nblks: 8,
        fsize: 1024,
        surround_mode: 0,
        lfe_flag: false,
        extended_surround_flag: 0,
        additional_info: &info,
    };
    let bytes = ser(&ok).unwrap();
    assert_eq!(bytes[1], 255);
    assert_eq!(DtsDescriptor::parse(&bytes).unwrap(), ok);

    let info = vec![0x5Au8; 251];
    let bad = DtsDescriptor {
        additional_info: &info,
        ..ok
    };
    assert!(matches!(
        ser(&bad),
        Err(dvb_si::Error::InvalidDescriptor { .. })
    ));
}

#[test]
fn announcement_support_255_byte_body_is_the_widest_legal() {
    let with_ref = || {
        AnnouncementEntry::new(
            AnnouncementType::EmergencyAlarm,
            ReferenceType::SeparateAudioStream,
            Some(AnnouncementReference {
                original_network_id: 1,
                transport_stream_id: 2,
                service_id: 3,
                component_tag: 4,
            }),
        )
    };
    let without_ref = || {
        AnnouncementEntry::new(
            AnnouncementType::NewsFlash,
            ReferenceType::UsualAudioStream,
            None,
        )
    };
    // indicator(2) + 31*8 + 5*1 = 255.
    let mut entries: Vec<_> = (0..31).map(|_| with_ref()).collect();
    entries.extend((0..5).map(|_| without_ref()));
    let ok = AnnouncementSupportDescriptor {
        announcement_support_indicator: 0x0001,
        entries,
    };
    let bytes = ser(&ok).unwrap();
    assert_eq!(bytes[1], 255);
    assert_eq!(AnnouncementSupportDescriptor::parse(&bytes).unwrap(), ok);

    // indicator(2) + 31*8 + 6*1 = 256.
    let mut entries: Vec<_> = (0..31).map(|_| with_ref()).collect();
    entries.extend((0..6).map(|_| without_ref()));
    let bad = AnnouncementSupportDescriptor {
        announcement_support_indicator: 0x0001,
        entries,
    };
    assert!(matches!(
        ser(&bad),
        Err(dvb_si::Error::InvalidDescriptor { .. })
    ));
}

#[test]
fn carousel_identifier_255_byte_body_is_the_widest_legal() {
    // prefix(5) + absent specifier(0) + 250 private bytes = 255.
    let private = vec![0xA5u8; 250];
    let ok = CarouselIdentifierDescriptor {
        carousel_id: 7,
        format: FormatSpecifier::Absent,
        private_data: &private,
    };
    let bytes = ser(&ok).unwrap();
    assert_eq!(bytes[1], 255);
    assert_eq!(CarouselIdentifierDescriptor::parse(&bytes).unwrap(), ok);

    let private = vec![0xA5u8; 251];
    let bad = CarouselIdentifierDescriptor {
        carousel_id: 7,
        format: FormatSpecifier::Absent,
        private_data: &private,
    };
    assert!(matches!(
        ser(&bad),
        Err(dvb_si::Error::InvalidDescriptor { .. })
    ));
}

#[test]
fn cell_frequency_link_255_byte_body_is_the_widest_legal() {
    let plain = |cell_id: u16| CellFrequencyLinkEntry {
        cell_id,
        frequency: 474_000_000 / 10,
        subcells: vec![],
    };
    // 34*7 + (7 + 2*5) = 238 + 17 = 255.
    let mut entries: Vec<_> = (0u16..34).map(plain).collect();
    entries.push(CellFrequencyLinkEntry {
        cell_id: 0x1234,
        frequency: 666_000_000 / 10,
        subcells: vec![
            CellFrequencyLinkSubcell {
                cell_id_extension: 1,
                transposer_frequency: 474_000_000 / 10,
            },
            CellFrequencyLinkSubcell {
                cell_id_extension: 2,
                transposer_frequency: 482_000_000 / 10,
            },
        ],
    });
    let ok = CellFrequencyLinkDescriptor { entries };
    let bytes = ser(&ok).unwrap();
    assert_eq!(bytes[1], 255);
    assert_eq!(CellFrequencyLinkDescriptor::parse(&bytes).unwrap(), ok);

    // 33*7 + 5*5 = 231 + 25 = 256.
    let mut entries: Vec<_> = (0u16..33).map(plain).collect();
    entries.push(CellFrequencyLinkEntry {
        cell_id: 0x1234,
        frequency: 666_000_000 / 10,
        subcells: (0u8..5)
            .map(|i| CellFrequencyLinkSubcell {
                cell_id_extension: i,
                transposer_frequency: 474_000_000 / 10,
            })
            .collect(),
    });
    let bad = CellFrequencyLinkDescriptor { entries };
    assert!(matches!(
        ser(&bad),
        Err(dvb_si::Error::InvalidDescriptor { .. })
    ));
}

#[test]
fn cell_list_widest_legal_body_is_serialized() {
    let cell = |cell_id: u16, subcells: Vec<CellListSubcell>| CellListEntry {
        cell_id,
        cell_latitude: 0,
        cell_longitude: 0,
        cell_extent_of_latitude: 0,
        cell_extent_of_longitude: 0,
        subcells,
    };
    let subcell = |i: u8| CellListSubcell {
        cell_id_extension: i,
        subcell_latitude: 0,
        subcell_longitude: 0,
        subcell_extent_of_latitude: 0,
        subcell_extent_of_longitude: 0,
    };
    // 10a + 8b; the body is always even, so 254 is the widest: 3*10 + 28*8.
    let entries = vec![
        cell(1, (0u8..10).map(subcell).collect()),
        cell(2, (0u8..9).map(subcell).collect()),
        cell(3, (0u8..9).map(subcell).collect()),
    ];
    let ok = CellListDescriptor { entries };
    let bytes = ser(&ok).unwrap();
    assert_eq!(bytes[1], 254);
    assert_eq!(CellListDescriptor::parse(&bytes).unwrap(), ok);

    // 8*10 + 22*8 = 256: the next reachable size is over the limit.
    let entries = vec![
        cell(1, (0u8..11).map(subcell).collect()),
        cell(2, (0u8..11).map(subcell).collect()),
        cell(3, vec![]),
        cell(4, vec![]),
        cell(5, vec![]),
        cell(6, vec![]),
        cell(7, vec![]),
        cell(8, vec![]),
    ];
    let bad = CellListDescriptor { entries };
    assert!(matches!(
        ser(&bad),
        Err(dvb_si::Error::InvalidDescriptor { .. })
    ));
}

#[test]
fn vbi_data_255_byte_body_is_the_widest_legal() {
    // entry header(2) + 253 service bytes = 255.
    let reserved = vec![0x11u8; 253];
    let ok = VbiDataDescriptor {
        entries: vec![VbiDataEntry {
            data_service_id: 0x00,
            service_descriptor: VbiService::Reserved(&reserved),
        }],
    };
    let bytes = ser(&ok).unwrap();
    assert_eq!(bytes[1], 255);
    assert_eq!(VbiDataDescriptor::parse(&bytes).unwrap(), ok);

    let reserved = vec![0x11u8; 254];
    let bad = VbiDataDescriptor {
        entries: vec![VbiDataEntry {
            data_service_id: 0x00,
            service_descriptor: VbiService::Reserved(&reserved),
        }],
    };
    assert!(matches!(
        ser(&bad),
        Err(dvb_si::Error::InvalidDescriptor { .. })
    ));
}
