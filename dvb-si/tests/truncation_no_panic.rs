//! Every strict prefix of a valid wire structure must yield `Err`, never a
//! panic, for the parsers whose big-endian reads went through
//! `wire::be_u16`/`be_u32` (replacing `first_chunk::<N>().unwrap()`).

use broadcast_common::{Parse, Serialize};
use dvb_si::descriptors::ca_identifier::CaIdentifierDescriptor;
use dvb_si::descriptors::local_time_offset::LocalTimeOffsetDescriptor;
use dvb_si::descriptors::nvod_reference::NvodReferenceDescriptor;
use dvb_si::descriptors::service_list::ServiceListDescriptor;
use dvb_si::descriptors::subtitling::SubtitlingDescriptor;
use dvb_si::tables::pat::{PatEntry, PatSection};

/// Parse every strict prefix of `full` (which itself must parse) and require Err.
fn all_prefixes_err<'a, T: Parse<'a>>(full: &'a [u8]) {
    assert!(T::parse(full).is_ok(), "full buffer must parse");
    for n in 0..full.len() {
        assert!(T::parse(&full[..n]).is_err(), "prefix len {n} must be Err");
    }
}

#[test]
fn descriptors_truncated_is_err() {
    // tag, length, body
    all_prefixes_err::<ServiceListDescriptor>(&[0x41, 6, 0, 1, 1, 0, 2, 2]);
    all_prefixes_err::<CaIdentifierDescriptor>(&[0x53, 4, 0x0B, 0x00, 0x18, 0x11]);
    all_prefixes_err::<NvodReferenceDescriptor>(&[0x4B, 6, 0, 1, 0, 2, 0, 3]);
    all_prefixes_err::<SubtitlingDescriptor>(&[0x59, 8, b'e', b'n', b'g', 0x10, 0, 1, 0, 2]);
    let mut lto = vec![0x58, 13, b'G', b'B', b'R', 0x02, 0x01, 0x00];
    lto.extend_from_slice(&[0, 0, 0, 0, 0, 0x01, 0x00]);
    all_prefixes_err::<LocalTimeOffsetDescriptor>(&lto);
}

#[test]
fn pat_truncated_is_err() {
    let pat = PatSection {
        transport_stream_id: 1,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        entries: vec![
            PatEntry {
                program_number: 1,
                pid: 0x100,
            },
            PatEntry {
                program_number: 2,
                pid: 0x101,
            },
        ],
    };
    let mut buf = vec![0u8; pat.serialized_len()];
    pat.serialize_into(&mut buf).unwrap();
    all_prefixes_err::<PatSection>(&buf);
}
