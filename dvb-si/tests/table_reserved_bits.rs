//! Reserved-bit preservation for every table section (reserved-bits audit,
//! `docs/reserved-bits-edition-audit.md` §3.6 "Section headers").
//!
//! Policy: parse is tolerant, serialize re-emits what was parsed (typed
//! `reserved*` / `private_indicator` fields, never a raw blob), and values
//! built with `new()` / `Default` carry the spec fill.
//!
//! Three kinds of evidence:
//!
//! 1. `defaults_carry_spec_fill_*`: `new()` output has the edition's spec bits.
//! 2. `single_bit_flip_*`: for a section of every table type (real captured
//!    sections from the committed fixtures where the capture has one, built
//!    sections for the rest) flip **every** bit of the section in turn,
//!    recompute the CRC, and require `parse -> serialize` to give the flipped
//!    bytes back. A reserved bit the serializer overwrites (the old behaviour)
//!    shows up as a lost bit. Only `section_syntax_indicator` (a structural
//!    decoded bit) and `section_length` are exempt.
//! 3. `real_sender_sections_*`: the exact section bytes of the real-capture
//!    deviations that motivated the change (TDT `0x00` first bytes, NIT
//!    `0xB0`, PAT `reserved(3)=000`, EIT `reserved(2)=00`, SDT
//!    `reserved_future_use(6)`), taken from the local capture archive where it
//!    is present.
#![cfg(feature = "ts")]

use broadcast_common::{Parse, Serialize};
use dvb_si::demux::SiDemux;
use dvb_si::descriptors::DescriptorLoop;
use dvb_si::tables::AnyTableSection;
use mpeg_ts::ts::TS_PACKET_SIZE;
use std::collections::BTreeSet;

fn crc(bytes: &[u8]) -> [u8; 4] {
    broadcast_common::crc32_mpeg2::compute(bytes).to_be_bytes()
}

fn ser<T: Serialize>(t: &T) -> Vec<u8>
where
    <T as Serialize>::Error: std::fmt::Debug,
{
    let mut b = vec![0u8; t.serialized_len()];
    t.serialize_into(&mut b).expect("serialize");
    b
}

/// Re-serialize any parsed section (the survey's `ser!` list).
fn reserialize(t: &AnyTableSection<'_>) -> Option<Vec<u8>> {
    macro_rules! s {
        ($t:expr) => {
            Some(ser($t))
        };
    }
    match t {
        AnyTableSection::PatSection(t) => s!(t),
        AnyTableSection::CatSection(t) => s!(t),
        AnyTableSection::PmtSection(t) => s!(t),
        AnyTableSection::TsdtSection(t) => s!(t),
        AnyTableSection::DsmccSection(t) => s!(t),
        AnyTableSection::NitSection(t) => s!(t),
        AnyTableSection::SdtSection(t) => s!(t),
        AnyTableSection::BatSection(t) => s!(t),
        AnyTableSection::UntSection(t) => s!(t),
        AnyTableSection::IntSection(t) => s!(t),
        AnyTableSection::SatSection(t) => s!(t),
        AnyTableSection::EitSection(t) => s!(t),
        AnyTableSection::TdtSection(t) => s!(t),
        AnyTableSection::RstSection(t) => s!(t),
        AnyTableSection::StSection(t) => s!(t),
        AnyTableSection::TotSection(t) => s!(t),
        AnyTableSection::AitSection(t) => s!(t),
        AnyTableSection::ContainerSection(t) => s!(t),
        AnyTableSection::RctSection(t) => s!(t),
        AnyTableSection::CitSection(t) => s!(t),
        AnyTableSection::MpeFecSection(t) => s!(t),
        AnyTableSection::RntSection(t) => s!(t),
        AnyTableSection::MpeIfecSection(t) => s!(t),
        AnyTableSection::ProtectionMessage(t) => s!(t),
        AnyTableSection::DownloadableFontInfo(t) => s!(t),
        AnyTableSection::DitSection(t) => s!(t),
        AnyTableSection::SitSection(t) => s!(t),
        AnyTableSection::MpeDatagram(t) => s!(t),
        _ => None,
    }
}

/// Section byte 1 bit 7: `section_syntax_indicator` (decoded structure).
const SSI_BYTE: usize = 1;
const SSI_MASK: u8 = 0x80;
/// Bytes 1 (low nibble) and 2 carry `section_length`.
const LEN_HI_MASK: u8 = 0x0F;

/// Flip every bit of `section` (but SSI, section_length and the table_id) and
/// return the list of `(byte, bit)` positions whose flip is lost on
/// `parse -> serialize` (the serializer wrote the original value back). `has_crc`: recompute the trailing CRC_32 after each flip.
fn lost_bits(section: &[u8], has_crc: bool) -> Vec<(usize, u8)> {
    let body_end = if has_crc {
        section.len() - 4
    } else {
        section.len()
    };
    let mut lost = Vec::new();
    for byte in 1..body_end {
        for bit in 0..8u8 {
            let mask = 1u8 << bit;
            if byte == SSI_BYTE && mask & SSI_MASK != 0 {
                continue;
            }
            if (byte == SSI_BYTE && mask & LEN_HI_MASK != 0) || byte == 2 {
                continue;
            }
            let mut f = section.to_vec();
            f[byte] ^= mask;
            if has_crc {
                let c = crc(&f[..body_end]);
                f[body_end..].copy_from_slice(&c);
            }
            let Ok(t) = AnyTableSection::parse(&f) else {
                continue;
            };
            // A *lost* bit is the one we flipped coming back unflipped with
            // nothing else changed (same length, single differing bit). A flip
            // that restructures the section (a length field, a discriminant
            // selecting another body layout) changes more than that bit.
            if let Some(re) = reserialize(&t)
                && re.len() == f.len()
                && re != f
            {
                // The CRC_32 legitimately differs whenever the content does.
                let diffs: Vec<usize> = (0..body_end).filter(|&i| re[i] != f[i]).collect();
                if diffs == [byte] && (re[byte] ^ f[byte]) == mask {
                    lost.push((byte, bit));
                }
            }
        }
    }
    lost
}

/// Real sections: first section of each distinct table_id from the committed
/// fixture captures.
fn real_sections() -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for (dir, name) in [
        ("ts", "m6-single.ts"),
        ("dvb-si", "tnt-5w-12732v-isi6-10s.ts"),
    ] {
        let path = format!("{}/../fixtures/{dir}/{name}", env!("CARGO_MANIFEST_DIR"));
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        let mut demux = SiDemux::builder().build();
        let mut per_tid = std::collections::BTreeMap::<u8, u32>::new();
        for chunk in data.chunks(TS_PACKET_SIZE) {
            if chunk.len() != TS_PACKET_SIZE || chunk[0] != 0x47 {
                continue;
            }
            for ev in demux.feed(chunk) {
                let bytes: &[u8] = ev.bytes().as_ref();
                let tid = bytes[0];
                let n = per_tid.entry(tid).or_default();
                if *n >= 2 || !ev.crc_ok() || !seen.insert(bytes.to_vec()) {
                    continue;
                }
                *n += 1;
                out.push((format!("{name} tid 0x{tid:02X}"), bytes.to_vec()));
            }
        }
    }
    out
}

fn is_short_no_crc(tid: u8) -> bool {
    // TDT, RST, ST, DIT carry no CRC_32 (EN 300 468 §5.2).
    matches!(tid, 0x70 | 0x71 | 0x72 | 0x7E)
}

#[test]
fn single_bit_flip_real_captured_sections_keep_every_bit() {
    let sections = real_sections();
    assert!(
        sections.len() >= 8,
        "committed fixtures must yield real sections, got {}",
        sections.len()
    );
    let mut tids = BTreeSet::new();
    for (name, sec) in &sections {
        tids.insert(sec[0]);
        let lost = lost_bits(sec, !is_short_no_crc(sec[0]));
        assert!(
            lost.is_empty(),
            "{name}: bits lost on parse->serialize (byte,bit): {lost:?}"
        );
    }
    // The fixtures cover the core SI/PSI tables.
    for tid in [0x00u8, 0x02, 0x42, 0x40] {
        assert!(tids.contains(&tid), "fixtures lack table_id 0x{tid:02X}");
    }
}

// ---------------------------------------------------------------------------
// Built sections for every table type that has no committed capture.
// ---------------------------------------------------------------------------

fn dl(raw: &'static [u8]) -> DescriptorLoop<'static> {
    DescriptorLoop::new(raw)
}

/// Descriptor-shaped bytes (tag, len, body) so loops look plausible.
const DESC: &[u8] = &[0x48, 0x02, 0xAA, 0xBB];

fn built_sections() -> Vec<(&'static str, Vec<u8>, bool)> {
    use dvb_si::compatibility::CompatibilityDescriptor;
    use dvb_si::tables::ait::*;
    use dvb_si::tables::cat::CatSection;
    use dvb_si::tables::cit::{CitSection, CridEntry};
    use dvb_si::tables::container::ContainerSection;
    use dvb_si::tables::dit::DitSection;
    use dvb_si::tables::downloadable_font_info::{DownloadableFontInfoSection, FontInfo};
    use dvb_si::tables::dsmcc::DsmccSection;
    use dvb_si::tables::int::{IntActionType, IntSection};
    use dvb_si::tables::mpe::{MacAddress, MpeDatagramSection};
    use dvb_si::tables::mpe_fec::{MpeFecSection, RealTimeParameters as FecRtp};
    use dvb_si::tables::mpe_ifec::{MpeIfecSection, RealTimeParameters as IfecRtp};
    use dvb_si::tables::protection_message::*;
    use dvb_si::tables::rct::*;
    use dvb_si::tables::rnt::*;
    use dvb_si::tables::rst::{RstEntry, RstSection};
    use dvb_si::tables::sat::{SatBody, SatSection};
    use dvb_si::tables::sit::{SitSection, SitService};
    use dvb_si::tables::st::StSection;
    use dvb_si::tables::tsdt::TsdtSection;
    use dvb_si::tables::unt::{UntActionType, UntPlatform, UntSection};
    use dvb_si::tables::{RunningStatus, TargetOperationalLoop};
    use dvb_si::text::{DvbText, LangCode};

    let mut v: Vec<(&'static str, Vec<u8>, bool)> = Vec::new();
    v.push((
        "TSDT",
        ser(&TsdtSection::new(1, 2, true, 0, 0, dl(DESC))),
        true,
    ));
    v.push(("CAT", ser(&CatSection::new(2, true, 0, 0, dl(DESC))), true));
    v.push(("DIT", ser(&DitSection::new(true)), false));
    v.push((
        "RST",
        ser(&RstSection::new(vec![
            RstEntry::new(1, 2, 3, 4, RunningStatus::Running),
            RstEntry::new(5, 6, 7, 8, RunningStatus::Pausing),
        ])),
        false,
    ));
    v.push((
        "ST",
        ser(&StSection::new(vec![0xFF, 0xFF, 0x12, 0x34])),
        false,
    ));
    v.push((
        "SIT",
        ser(&SitSection::new(
            0xFFFF,
            3,
            true,
            0,
            0,
            dl(DESC),
            vec![
                SitService::new(1, RunningStatus::Running, dl(DESC)),
                SitService::new(2, RunningStatus::Pausing, dl(&[])),
            ],
        )),
        true,
    ));
    v.push((
        "AIT",
        ser(&AitSection::new(
            ApplicationType::HbbTv,
            false,
            4,
            true,
            0,
            0,
            dl(DESC),
            vec![
                AitApplication::new(
                    ApplicationIdentifier {
                        organisation_id: 0x1234_5678,
                        application_id: 7,
                    },
                    ControlCode::Autostart,
                    dl(DESC),
                ),
                AitApplication::new(
                    ApplicationIdentifier {
                        organisation_id: 1,
                        application_id: 2,
                    },
                    ControlCode::Present,
                    dl(&[]),
                ),
            ],
        )),
        true,
    ));
    let pair = || TargetOperationalLoop::new(dl(DESC), dl(DESC));
    v.push((
        "INT",
        ser(&IntSection::new(
            IntActionType::IpMacStreamLocation,
            0x12,
            1,
            true,
            0,
            0,
            0x0001_5A00,
            0,
            dl(DESC),
            vec![pair(), pair()],
        )),
        true,
    ));
    v.push((
        "UNT",
        ser(&UntSection::new(
            UntActionType::SystemSoftwareUpdate,
            0x5B,
            2,
            true,
            0,
            0,
            0x0001_5A00,
            0,
            dl(DESC),
            vec![UntPlatform::new(
                CompatibilityDescriptor {
                    descriptors: vec![],
                },
                vec![pair(), pair()],
            )],
        )),
        true,
    ));
    v.push((
        "RNT",
        ser(&RntSection::new(
            0x0102,
            1,
            true,
            0,
            0,
            ContextIdType::from_u8(1),
            dl(DESC),
            vec![ResolutionProvider::new(
                DvbText::new(b"rp"),
                dl(DESC),
                vec![
                    CridAuthority::new(
                        DvbText::new(b"auth"),
                        CridAuthorityPolicy::from_u8(1),
                        dl(DESC),
                    ),
                    CridAuthority::new(
                        DvbText::new(b"b"),
                        CridAuthorityPolicy::from_u8(2),
                        dl(&[]),
                    ),
                ],
            )],
        )),
        true,
    ));
    let loc = |full: bool| {
        DvbBinaryLocator::new(
            true,
            0x123,
            if full {
                DvbLocatorService::Full {
                    transport_stream_id: 1,
                    original_network_id: 2,
                    service_id: 3,
                }
            } else {
                DvbLocatorService::Triplet {
                    dvb_service_triplet_id: 0x155,
                }
            },
            0x1234,
            0x0042,
            DvbLocatorIdentifier::None,
            Some(DvbLocatorWindows {
                early_start_window: 3,
                late_end_window: 9,
            }),
        )
    };
    v.push((
        "RCT",
        ser(&RctSection::new(
            true,
            0x0001,
            2,
            true,
            0,
            0,
            2026,
            vec![
                LinkInfo::new(
                    LinkType::Both,
                    HowRelated::from_u8(1),
                    0x234,
                    2,
                    3,
                    Some(b"dvb://x"),
                    Some(loc(true)),
                    vec![LinkItem {
                        language_code: LangCode(*b"eng"),
                        promotional_text: DvbText::new(b"hi"),
                    }],
                    true,
                    5,
                    dl(DESC),
                ),
                LinkInfo::new(
                    LinkType::BinaryLocator,
                    HowRelated::from_u8(2),
                    1,
                    0,
                    0,
                    None,
                    Some(loc(false)),
                    vec![],
                    false,
                    0,
                    dl(&[]),
                ),
            ],
            dl(DESC),
        )),
        true,
    ));
    v.push((
        "CIT",
        ser(&CitSection::new(
            true,
            0x0001,
            1,
            true,
            0,
            0,
            3,
            4,
            DvbText::new(b"crid://x\0"),
            vec![CridEntry {
                crid_ref: 1,
                prepend_string_index: 0,
                unique_string: DvbText::new(b"uniq"),
            }],
        )),
        true,
    ));
    v.push((
        "CONTAINER",
        ser(&ContainerSection::new(true, 5, 1, true, 0, 0, &[1, 2, 3])),
        true,
    ));
    v.push((
        "DSMCC",
        ser(&DsmccSection::new(
            0x3B,
            0x1234,
            1,
            true,
            0,
            0,
            &[9, 8, 7, 6],
        )),
        true,
    ));
    v.push((
        "MPE",
        ser(&MpeDatagramSection::new(
            MacAddress([1, 2, 3, 4, 5, 6]),
            0,
            0,
            &[0x45, 0, 0, 20],
        )),
        true,
    ));
    v.push((
        "MPE-FEC",
        ser(&MpeFecSection::new(
            true,
            3,
            true,
            0,
            0,
            FecRtp {
                delta_t: 5,
                table_boundary: true,
                frame_boundary: false,
                address: 77,
            },
            &[1, 2, 3, 4],
        )),
        true,
    ));
    v.push((
        "MPE-IFEC",
        ser(&MpeIfecSection::new(
            true,
            1,
            2,
            3,
            true,
            0,
            0,
            IfecRtp {
                delta_t: 5,
                mpe_boundary: true,
                frame_boundary: false,
                prev_burst_size: 77,
            },
            &[1, 2, 3, 4],
        )),
        true,
    ));
    v.push((
        "PROTECTION-AUTH",
        ser(&ProtectionMessageSection::new(
            0x0001,
            1,
            true,
            0,
            0,
            ProtectionMessageBody::AuthenticationMessage {
                section_hash_algorithm_identifier: 0,
                section_hash_length: 2,
                signature_algorithm_identifier: 1,
                hashes: vec![SectionHashEntry {
                    reference_type: ReferenceType::SameEs,
                    reference: &[0x01],
                    hash: &[0xAA, 0xBB],
                }],
                extension_bytes: &[0xDE],
                signature_key_identifier: &[0x11],
                signature: &[0x90, 0x91],
                reserved_section_hashes_loop_length: 0x0F,
            },
        )),
        true,
    ));
    v.push((
        "PROTECTION-CERTS",
        ser(&ProtectionMessageSection::new(
            0x0100,
            1,
            true,
            0,
            0,
            ProtectionMessageBody::certificate_collection(vec![&[1, 2], &[3]]),
        )),
        true,
    ));
    v.push((
        "FONT-INFO",
        ser(&DownloadableFontInfoSection::new(
            0,
            0x42,
            1,
            true,
            0,
            0,
            vec![
                FontInfo::style_weight(1, 2),
                FontInfo::file_uri(1, b"u"),
                FontInfo::FontSize {
                    size: 12,
                    info: b"a",
                },
            ],
        )),
        true,
    ));
    v.push((
        "SAT",
        ser(&SatSection::new(
            9,
            true,
            1,
            1,
            true,
            0,
            0,
            SatBody::Raw(vec![1, 2, 3]),
        )),
        true,
    ));
    v
}

#[test]
fn single_bit_flip_built_sections_keep_every_bit() {
    for (name, sec, has_crc) in built_sections() {
        // A built section must itself re-parse byte-identically first.
        let t = AnyTableSection::parse(&sec).unwrap_or_else(|e| panic!("{name}: parse: {e}"));
        assert_eq!(reserialize(&t).as_deref(), Some(&sec[..]), "{name}: base");
        let lost = lost_bits(&sec, has_crc);
        assert!(
            lost.is_empty(),
            "{name}: bits lost on parse->serialize (byte,bit): {lost:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Spec fill of constructed values (audit §3.6 + EN 300 468 §5.1.1 / H.222.0).
// ---------------------------------------------------------------------------

#[test]
fn defaults_carry_spec_fill_psi_private_indicator_zero() {
    use dvb_si::tables::cat::CatSection;
    use dvb_si::tables::pat::{PatEntry, PatSection};
    use dvb_si::tables::pmt::{PmtSection, PmtStream, StreamType};
    use dvb_si::tables::tsdt::TsdtSection;
    // PAT: H.222.0 §2.4.4.3: section_syntax_indicator '1', '0', reserved '11'
    // => byte1 0xB0|len_hi; reserved(2)=11 before version; reserved(3)=111 on PIDs.
    let pat = ser(&PatSection::new(
        1,
        0,
        true,
        0,
        0,
        vec![PatEntry::new(1, 0x0100)],
    ));
    assert_eq!(pat[1] & 0xF0, 0xB0, "PAT: ssi=1, '0', reserved=11");
    assert_eq!(pat[5] & 0xC0, 0xC0, "PAT: reserved(2) before version");
    assert_eq!(pat[10] & 0xE0, 0xE0, "PAT: reserved(3) before PID");
    let pmt = ser(&PmtSection::new(
        1,
        0,
        true,
        0,
        0,
        0x0101,
        dl(&[]),
        vec![PmtStream::new(StreamType::H264, 0x0101, dl(&[]))],
    ));
    assert_eq!(pmt[1] & 0xF0, 0xB0);
    assert_eq!(pmt[8] & 0xE0, 0xE0, "PMT: reserved(3) before PCR_PID");
    assert_eq!(
        pmt[10] & 0xF0,
        0xF0,
        "PMT: reserved(4) before program_info_length"
    );
    assert_eq!(
        pmt[13] & 0xE0,
        0xE0,
        "PMT: reserved(3) before elementary_PID"
    );
    assert_eq!(
        pmt[15] & 0xF0,
        0xF0,
        "PMT: reserved(4) before ES_info_length"
    );
    let cat = ser(&CatSection::new(1, true, 0, 0, dl(&[])));
    assert_eq!(cat[1] & 0xF0, 0xB0);
    assert_eq!(&cat[3..5], &[0xFF, 0xFF], "CAT: reserved(18) fill");
    let tsdt = ser(&TsdtSection::new(0xFFFF, 1, true, 0, 0, dl(&[])));
    assert_eq!(tsdt[1] & 0xF0, 0xB0);
}

#[test]
fn defaults_carry_spec_fill_dvb_reserved_future_use_one() {
    use dvb_si::tables::bat::BatSection;
    use dvb_si::tables::eit::{EitKind, EitSection};
    use dvb_si::tables::nit::{NitKind, NitSection};
    use dvb_si::tables::sdt::{SdtKind, SdtSection, SdtService};
    // EN 300 468 §5.1.1: reserved_future_use(1) = 1 after SSI => 0xF0 top nibble.
    let sdt = ser(&SdtSection::new(
        SdtKind::Actual,
        1,
        0,
        true,
        0,
        0,
        0x20,
        vec![SdtService::new(
            1,
            false,
            true,
            RunningStatus::Running,
            false,
            dl(&[]),
        )],
    ));
    assert_eq!(sdt[1] & 0xF0, 0xF0, "SDT byte 1 top nibble");
    assert_eq!(sdt[5] & 0xC0, 0xC0);
    assert_eq!(sdt[10], 0xFF, "SDT reserved_future_use(8)");
    assert_eq!(sdt[13] & 0xFC, 0xFC, "SDT reserved_future_use(6)");
    let nit = ser(&NitSection::new(
        NitKind::Actual,
        1,
        0,
        true,
        0,
        0,
        dl(&[]),
        vec![],
    ));
    assert_eq!(nit[1] & 0xF0, 0xF0);
    assert_eq!(nit[8] & 0xF0, 0xF0, "NIT reserved_future_use(4)");
    let bat = ser(&BatSection::new(1, 0, true, 0, 0, dl(&[]), vec![]));
    assert_eq!(bat[1] & 0xF0, 0xF0);
    let eit = ser(&EitSection::new(
        EitKind::PresentFollowingActual,
        0x4E,
        1,
        0,
        true,
        0,
        0,
        1,
        1,
        0,
        0x4E,
        vec![],
    ));
    assert_eq!(eit[1] & 0xF0, 0xF0);
    assert_eq!(eit[5] & 0xC0, 0xC0);
}

use dvb_si::tables::RunningStatus;

#[test]
fn defaults_carry_spec_fill_short_sections() {
    use dvb_si::tables::tdt::TdtSection;
    use dvb_si::tables::tot::TotSection;
    // EN 300 468 §5.2.5: TDT byte 1 = ssi 0 | rfu 1 | reserved 11 => 0x70.
    let tdt = ser(&TdtSection::new([0xE9, 0xC4, 0x15, 0x33, 0x07]));
    assert_eq!(tdt[1] & 0xF0, 0x70);
    let tot = ser(&TotSection::new([0xE9, 0xC4, 0x15, 0x33, 0x07], dl(&[])));
    assert_eq!(tot[1] & 0xF0, 0x70);
    assert_eq!(
        tot[8] & 0xF0,
        0xF0,
        "TOT reserved(4) before descriptors_loop_length"
    );
}

// ---------------------------------------------------------------------------
// Typed fields: off-spec values set explicitly come back out.
// ---------------------------------------------------------------------------

#[test]
fn typed_header_bits_round_trip_off_spec_values() {
    use dvb_si::tables::SectionBits;
    use dvb_si::tables::pat::PatSection;
    let mut pat = PatSection::new(1, 0, true, 0, 0, vec![]);
    let mut bits = SectionBits::dvb();
    bits.private_indicator = false;
    bits.reserved_b1 = 0;
    bits.reserved_b5 = 0;
    pat.bits = bits;
    let b = ser(&pat);
    assert_eq!(
        b[1] & 0x70,
        0x00,
        "private_indicator and reserved(2) cleared"
    );
    assert_eq!(b[5] & 0xC0, 0x00);
    let re = PatSection::parse(&b).unwrap();
    assert_eq!(re.bits, bits);
    assert_eq!(ser(&re), b);
    // PSI default is private_indicator = 0, DVB default is 1.
    assert!(!SectionBits::psi().private_indicator);
    assert!(SectionBits::dvb().private_indicator);
}

// ---------------------------------------------------------------------------
// Real-sender sections (local capture archive; skipped when absent).
// ---------------------------------------------------------------------------

/// `(stride, offset of the 188-byte packet in a stride, first sync)`: plain TS,
/// M2TS/BDAV (4-byte prefix) or RS-protected packets.
fn detect(data: &[u8]) -> Option<(usize, usize, usize)> {
    for stride in [188usize, 192, 204] {
        let pre = if stride == 192 { 4 } else { 0 };
        for start in 0..stride.min(data.len()) {
            if (0..5).all(|k| data.get(start + pre + k * stride) == Some(&0x47)) {
                return Some((stride, pre, start));
            }
        }
    }
    None
}

/// `None` when the local archive (or this capture) is absent: the test skips.
fn archive_sections(file: &str, tid: u8, want: impl Fn(&[u8]) -> bool) -> Option<Vec<Vec<u8>>> {
    let stem = file.trim_end_matches(".ts");
    let root = format!(
        "{}/../.test-streams/tsduck-test",
        env!("CARGO_MANIFEST_DIR")
    );
    let candidates = [
        format!("{root}/input/{file}"),
        format!("{root}/reference/{stem}/{file}"),
    ];
    let mut all: Option<Vec<Vec<u8>>> = None;
    for path in candidates {
        if let Some(found) = archive_sections_in(&path, tid, &want) {
            for s in found {
                let v = all.get_or_insert_with(Vec::new);
                if !v.contains(&s) {
                    v.push(s);
                }
            }
            all.get_or_insert_with(Vec::new);
        }
    }
    all
}

fn archive_sections_in(path: &str, tid: u8, want: &dyn Fn(&[u8]) -> bool) -> Option<Vec<Vec<u8>>> {
    let Ok(data) = std::fs::read(path) else {
        eprintln!("SKIP: {path} absent (local-only real-capture archive)");
        return None;
    };
    let mut out = Vec::new();
    let Some((stride, pre, start)) = detect(&data) else {
        return Some(out);
    };
    // Pass 1: PIDs that carry private sections / DSM-CC (PMT stream_type
    // 0x05, 0x0A..=0x0D) are not in the default SI filter; learn them first.
    let packets = |f: &mut dyn FnMut(&[u8])| {
        let mut pos = start;
        while pos + pre + TS_PACKET_SIZE <= data.len() {
            let chunk = &data[pos + pre..pos + pre + TS_PACKET_SIZE];
            pos += stride;
            if chunk[0] == 0x47 {
                f(chunk);
            }
        }
    };
    let mut extra = BTreeSet::new();
    let mut learn = SiDemux::builder().build();
    packets(&mut |chunk| {
        for ev in learn.feed(chunk) {
            if let Ok(AnyTableSection::PmtSection(pmt)) = ev.table_section() {
                for st in &pmt.streams {
                    if matches!(st.stream_type.to_u8(), 0x05 | 0x0A | 0x0B | 0x0C | 0x0D) {
                        extra.insert(st.elementary_pid);
                    }
                }
            }
        }
    });
    let mut builder = SiDemux::builder().emit_repeats(true);
    for pid in &extra {
        builder = builder.pid(mpeg_ts::pid::Pid::new(*pid));
    }
    let mut demux = builder.build();
    let mut pos = start;
    while pos + pre + TS_PACKET_SIZE <= data.len() {
        let chunk = &data[pos + pre..pos + pre + TS_PACKET_SIZE];
        pos += stride;
        if chunk[0] != 0x47 {
            continue;
        }
        for ev in demux.feed(chunk) {
            let b: &[u8] = ev.bytes().as_ref();
            if b[0] == tid && ev.crc_ok() && want(b) && !out.iter().any(|o: &Vec<u8>| o == b) {
                out.push(b.to_vec());
            }
        }
    }
    Some(out)
}

/// Every section found must re-serialize byte-identically; the capture must
/// actually contain the off-spec section the test is about (so the test
/// cannot silently stop testing).
fn assert_identical(name: &str, sections: Option<Vec<Vec<u8>>>, expect_at_least: usize) {
    let Some(sections) = sections else {
        return; // archive absent
    };
    assert!(
        sections.len() >= expect_at_least,
        "{name}: found {} off-spec sections, expected >= {expect_at_least}",
        sections.len()
    );
    for s in &sections {
        let t = AnyTableSection::parse(s).unwrap_or_else(|e| panic!("{name}: parse {e}"));
        assert_eq!(
            reserialize(&t).as_deref(),
            Some(&s[..]),
            "{name}: not byte-identical"
        );
    }
}

#[test]
fn real_sender_sections_tdt_byte1_zero() {
    // test-120: TDT whose byte 1 is 0x00 (private_indicator 0, reserved(2) 00).
    let s = archive_sections("test-120.ts", 0x70, |b| b[1] == 0x00);
    if let Some(v) = &s {
        assert!(v.iter().all(|b| b[1] == 0x00));
    }
    assert_identical("TDT test-120", s, 1);
}

#[test]
fn real_sender_sections_nit_private_indicator_zero() {
    // test-016: NIT with byte 1 = 0xB0 (reserved_future_use bit cleared).
    let s = archive_sections("test-016.ts", 0x40, |b| b[1] & 0xF0 == 0xB0);
    assert_identical("NIT test-016", s, 1);
}

#[test]
fn real_sender_sections_pat_reserved3_zero() {
    // test-012: PAT whose PMT-PID `reserved(3)` is 000.
    let s = archive_sections("test-012.ts", 0x00, |b| b[10] & 0xE0 == 0x00);
    assert_identical("PAT test-012", s, 1);
}

#[test]
fn real_sender_sections_eit_reserved2_zero() {
    // test-109: EIT whose version byte has reserved(2) = 00.
    let s = archive_sections("test-109.ts", 0x50, |b| b[5] & 0xC0 == 0x00);
    assert_identical("EIT test-109", s, 1);
}

#[test]
fn real_sender_sections_sdt_reserved_future_use6() {
    // test-170: SDT whose per-service reserved_future_use(6) is not all ones.
    let s = archive_sections("test-170.ts", 0x42, |b| {
        b.len() > 13 && b[13] & 0xFC != 0xFC
    });
    assert_identical("SDT test-170", s, 1);
}
