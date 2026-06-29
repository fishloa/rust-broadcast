//! Live-capture validation of the BIOP object-carousel layer.
//!
//! Uses the same `tests/fixtures/m6-single.ts` (French TNT, M6 HbbTV object
//! carousel, PID 0x00AB) as `carousel_fixture.rs`.
//!
//! Test `m6_sgi_oracle` is the primary correctness gate: it parses the DSI
//! `private_data` as a `ServiceGatewayInfo` and asserts every field against
//! independently-derived ground truth, then verifies byte-exact round-trip.
#![cfg(feature = "ts")]

use broadcast_common::{Parse, Serialize};
use dvb_si::carousel::biop::message::BindingType;
use dvb_si::carousel::biop::{
    Binding, BiopMessage, BiopProfileBody, CarouselFs, ConnBinder, DirectoryMessage, FileMessage,
    Ior, ModuleInfo, NameComponent, ObjectKind, ObjectLocation, ServiceGatewayInfo, TaggedProfile,
    Tap, BIOP_DELIVERY_PARA_USE,
};
use dvb_si::carousel::UnMessage;
use dvb_si::tables::dsmcc::DsmccSection;
use mpeg_ts::ts::{SectionReassembler, TsPacket, TS_PACKET_SIZE};

/// Extract reassembled sections for PID 0x00AB from the m6 fixture.
fn m6_sections_pid_ab() -> Vec<Vec<u8>> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/m6-single.ts");
    let data = std::fs::read(path).expect("read m6 fixture");
    let mut reassembler = SectionReassembler::default();
    let mut sections = Vec::new();

    for chunk in data.chunks(TS_PACKET_SIZE) {
        if chunk.len() != TS_PACKET_SIZE {
            continue;
        }
        let pkt = match TsPacket::parse(chunk) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if pkt.header.pid != 0x00AB {
            continue;
        }
        if let Some(payload) = pkt.payload {
            reassembler.feed(payload, pkt.header.pusi);
            while let Some(sec) = reassembler.pop_section() {
                sections.push(sec.to_vec());
            }
        }
    }
    sections
}

/// Return the first DSI private_data bytes from the m6 fixture.
fn m6_dsi_private_data() -> Vec<u8> {
    for sec in &m6_sections_pid_ab() {
        if sec.first() != Some(&0x3B) {
            continue;
        }
        let section = DsmccSection::parse(sec).expect("DSM-CC section parse");
        if let Ok(UnMessage::Dsi(dsi)) = UnMessage::parse(section.payload) {
            return dsi.private_data.to_vec();
        }
    }
    panic!("no DSI found in m6 fixture");
}

// ── Primary oracle test ───────────────────────────────────────────────────────

/// Parse the live DSI private_data as a ServiceGatewayInfo and assert every
/// field against independently-verified ground truth from the wire bytes.
/// Also verifies byte-exact round-trip: `sgi.to_bytes() == dsi.private_data`.
#[test]
fn m6_sgi_oracle() {
    let pd = m6_dsi_private_data();
    assert_eq!(pd.len(), 64, "DSI private_data must be 64 bytes");

    let sgi = ServiceGatewayInfo::parse(&pd).expect("ServiceGatewayInfo parse");

    // IOR type_id == "srg\0"
    assert_eq!(sgi.ior.type_id, b"srg\0", "IOR type_id must be \"srg\\0\"");
    assert_eq!(
        sgi.ior.object_kind(),
        ObjectKind::ServiceGateway,
        "IOR object_kind must be ServiceGateway"
    );

    // 1 profile, a BIOP profile
    assert_eq!(sgi.ior.profiles.len(), 1, "IOR must have exactly 1 profile");
    let bp = sgi
        .ior
        .biop_profile()
        .expect("IOR must have a BIOP profile");

    // ObjectLocation
    assert_eq!(
        bp.object_location.carousel_id, 0xAB,
        "carousel_id must be 0xAB (171)"
    );
    assert_eq!(bp.object_location.module_id, 1, "module_id must be 1");
    assert_eq!(
        bp.object_location.version_major, 1,
        "version_major must be 1"
    );
    assert_eq!(
        bp.object_location.version_minor, 0,
        "version_minor must be 0"
    );
    assert_eq!(
        bp.object_location.object_key,
        &[0x01],
        "object_key must be [0x01]"
    );

    // ConnBinder: 1 tap
    assert_eq!(bp.conn_binder.taps.len(), 1, "ConnBinder must have 1 tap");
    let tap = &bp.conn_binder.taps[0];
    assert_eq!(
        tap.use_, BIOP_DELIVERY_PARA_USE,
        "tap use_ must be BIOP_DELIVERY_PARA_USE (0x0016)"
    );
    assert_eq!(
        tap.association_tag, 0x47,
        "tap association_tag must be 0x47 (71)"
    );
    assert_eq!(
        tap.selector,
        &[0x00, 0x01, 0x80, 0x00, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF],
        "tap selector must match broadcast bytes"
    );
    assert_eq!(
        tap.transaction_id(),
        Some(0x80000002),
        "tap transaction_id() must be Some(0x80000002)"
    );
    assert_eq!(
        tap.timeout(),
        Some(0xFFFF_FFFF),
        "tap timeout() must be Some(0xFFFFFFFF)"
    );

    // Byte-exact round-trip — the hardest gate.
    let out = sgi.to_bytes();
    assert_eq!(out.len(), 64, "SGI serialized length must be 64 bytes");
    assert_eq!(
        out.as_slice(),
        pd.as_slice(),
        "sgi.to_bytes() must equal dsi.private_data byte-for-byte"
    );
}

// ── Synthetic CarouselFs test ─────────────────────────────────────────────────

/// Build a synthetic carousel with a ServiceGateway directory binding
/// "index.html" to a File, serialize into module buffers, build a
/// `CarouselFs`, and assert `file_bytes(&["index.html"])` returns the content.
#[test]
fn synthetic_carousel_fs_file_lookup() {
    let file_content: &[u8] = b"Hello, DVB object carousel!";

    // The file object: module 2, key [0x02]
    let file_ior = Ior {
        type_id: b"fil\0",
        profiles: vec![TaggedProfile::Biop(BiopProfileBody {
            object_location: ObjectLocation {
                carousel_id: 0xAB,
                module_id: 2,
                version_major: 1,
                version_minor: 0,
                object_key: &[0x02],
            },
            conn_binder: ConnBinder { taps: vec![] },
            extra: vec![],
        })],
    };

    // The service gateway: module 1, key [0x01], binds "index.html" to the file
    let sgw_msg = BiopMessage::ServiceGateway(DirectoryMessage {
        object_kind: *b"srg\0",
        object_key: &[0x01],
        object_info: &[],
        service_context: vec![], // serviceContextList_count=0
        bindings: vec![Binding {
            name: vec![NameComponent {
                id: b"index.html",
                kind: b"fil\0",
            }],
            binding_type: BindingType::NObject,
            ior: file_ior,
            object_info: &[],
        }],
    });

    let file_msg = BiopMessage::File(FileMessage {
        object_key: &[0x02],
        content_size: file_content.len() as u64,
        object_info_extra: &[],
        service_context: vec![],
        content: file_content,
    });

    // Serialize into module byte buffers
    let mut mod1_buf = vec![0u8; sgw_msg.serialized_len()];
    sgw_msg.serialize_into(&mut mod1_buf).unwrap();

    let mut mod2_buf = vec![0u8; file_msg.serialized_len()];
    file_msg.serialize_into(&mut mod2_buf).unwrap();

    // Build the filesystem
    let fs = CarouselFs::from_modules(&[(1, mod1_buf.as_slice()), (2, mod2_buf.as_slice())]);

    // Verify root exists
    assert!(
        fs.service_gateway().is_some(),
        "CarouselFs must find a ServiceGateway root"
    );

    // Resolve the file
    let bytes = fs.file_bytes(&["index.html"]);
    assert_eq!(
        bytes,
        Some(file_content),
        "file_bytes([\"index.html\"]) must return the file content"
    );

    // Non-existent path returns None
    assert!(fs.file_bytes(&["not-there.html"]).is_none());
}

// ── Round-trip tests ──────────────────────────────────────────────────────────

#[test]
fn ior_biop_round_trip() {
    use broadcast_common::Parse;
    // Build an IOR, serialize, re-parse, re-serialize — must be byte-identical.
    let ior = Ior {
        type_id: b"dir\0",
        profiles: vec![TaggedProfile::Biop(BiopProfileBody {
            object_location: ObjectLocation {
                carousel_id: 0x0001_0001,
                module_id: 3,
                version_major: 1,
                version_minor: 0,
                object_key: &[0x03],
            },
            conn_binder: ConnBinder {
                taps: vec![Tap {
                    id: 0,
                    use_: BIOP_DELIVERY_PARA_USE,
                    association_tag: 0x50,
                    selector: &[0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0xFF, 0xFF, 0xFF, 0xFF],
                }],
            },
            extra: vec![],
        })],
    };
    let mut buf = vec![0u8; ior.serialized_len()];
    ior.serialize_into(&mut buf).unwrap();
    let ior2 = Ior::parse(&buf).unwrap();
    assert_eq!(ior2, ior);
    let mut buf2 = vec![0u8; ior2.serialized_len()];
    ior2.serialize_into(&mut buf2).unwrap();
    assert_eq!(buf, buf2, "IOR byte-exact re-serialize");
}

#[test]
fn module_info_round_trip() {
    let info = ModuleInfo {
        module_timeout: 0x00FFFFFF,
        block_timeout: 0x00FFFFFF,
        min_block_time: 0x00000064,
        taps: vec![Tap {
            id: 0,
            use_: 0x0017,
            association_tag: 0x0042,
            selector: &[],
        }],
        user_info: &[],
    };
    let mut buf = vec![0u8; info.serialized_len()];
    info.serialize_into(&mut buf).unwrap();
    let parsed = ModuleInfo::parse(&buf).unwrap();
    assert_eq!(parsed, info);
    let mut buf2 = vec![0u8; parsed.serialized_len()];
    parsed.serialize_into(&mut buf2).unwrap();
    assert_eq!(buf, buf2, "ModuleInfo byte-exact re-serialize");
}

#[test]
fn sgi_round_trip() {
    // Build a synthetic SGI with a known IOR and round-trip it.
    let raw = {
        let ior = Ior {
            type_id: b"srg\0",
            profiles: vec![TaggedProfile::Biop(BiopProfileBody {
                object_location: ObjectLocation {
                    carousel_id: 0xAB,
                    module_id: 1,
                    version_major: 1,
                    version_minor: 0,
                    object_key: &[0x01],
                },
                conn_binder: ConnBinder {
                    taps: vec![Tap {
                        id: 0,
                        use_: BIOP_DELIVERY_PARA_USE,
                        association_tag: 0x47,
                        selector: &[0x00, 0x01, 0x80, 0x00, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF],
                    }],
                },
                extra: vec![],
            })],
        };
        let mut ior_buf = vec![0u8; ior.serialized_len()];
        ior.serialize_into(&mut ior_buf).unwrap();
        // Append SGI trailer: downloadTaps_count=0, serviceContextList_count=0, userInfoLength=0
        ior_buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        ior_buf
    };

    let sgi = ServiceGatewayInfo::parse(&raw).unwrap();
    let out = sgi.to_bytes();
    assert_eq!(out, raw, "SGI round-trip byte-exact");
}

/// Byte-anchor test for a hand-built Directory message.
#[test]
fn directory_message_byte_anchor() {
    // A ServiceGateway with one ncontext binding "subdir" -> module 3, key [0x03]
    let ior = Ior {
        type_id: b"dir\0",
        profiles: vec![TaggedProfile::Biop(BiopProfileBody {
            object_location: ObjectLocation {
                carousel_id: 0xAB,
                module_id: 3,
                version_major: 1,
                version_minor: 0,
                object_key: &[0x03],
            },
            conn_binder: ConnBinder { taps: vec![] },
            extra: vec![],
        })],
    };
    let sgw = BiopMessage::ServiceGateway(DirectoryMessage {
        object_kind: *b"srg\0",
        object_key: &[0x01],
        object_info: &[],
        service_context: vec![],
        bindings: vec![Binding {
            name: vec![NameComponent {
                id: b"subdir",
                kind: b"dir\0",
            }],
            binding_type: BindingType::NContext,
            ior,
            object_info: &[],
        }],
    });
    let mut buf = vec![0u8; sgw.serialized_len()];
    sgw.serialize_into(&mut buf).unwrap();
    let (parsed, consumed) = BiopMessage::parse_at(&buf).unwrap();
    assert_eq!(consumed, buf.len());
    assert_eq!(parsed, sgw);
    let mut buf2 = vec![0u8; parsed.serialized_len()];
    parsed.serialize_into(&mut buf2).unwrap();
    assert_eq!(buf, buf2, "Directory/SGW message byte-exact re-serialize");
}

#[test]
fn file_message_byte_anchor() {
    let content = b"BIOP file content for anchor test";
    let fm = BiopMessage::File(FileMessage {
        object_key: &[0x04],
        content_size: content.len() as u64,
        object_info_extra: &[],
        service_context: vec![],
        content,
    });
    let mut buf = vec![0u8; fm.serialized_len()];
    fm.serialize_into(&mut buf).unwrap();

    // Verify the BIOP magic at the start
    assert_eq!(&buf[0..4], b"BIOP", "BIOP magic must be present");
    assert_eq!(buf[4], 0x01, "version major must be 1");
    assert_eq!(buf[5], 0x00, "version minor must be 0");
    assert_eq!(buf[6], 0x00, "byte_order must be 0");
    assert_eq!(buf[7], 0x00, "message_type must be 0");

    let (parsed, consumed) = BiopMessage::parse_at(&buf).unwrap();
    assert_eq!(consumed, buf.len());
    assert_eq!(parsed, fm);
    let mut buf2 = vec![0u8; parsed.serialized_len()];
    parsed.serialize_into(&mut buf2).unwrap();
    assert_eq!(buf, buf2, "FileMessage byte-exact re-serialize");
}

// ── serde round-trip ──────────────────────────────────────────────────────────

#[cfg(feature = "serde")]
#[test]
fn sgi_serde_json() {
    let pd = m6_dsi_private_data();
    let sgi = ServiceGatewayInfo::parse(&pd).unwrap();
    let json = serde_json::to_string(&sgi).unwrap();
    // type_id is serialized as a byte array; carousel_id is a named field
    assert!(
        json.contains("carousel_id"),
        "JSON must contain carousel_id field"
    );
    // The IOR's Biop profile variant name should appear
    assert!(
        json.contains("\"Biop\""),
        "JSON must contain Biop profile variant"
    );
}

// ── flate2 decompression ──────────────────────────────────────────────────────

#[cfg(feature = "flate2")]
#[test]
fn zlib_decompress_round_trip() {
    use dvb_si::carousel::biop::message::decompress_zlib;
    use flate2::{write::ZlibEncoder, Compression};
    use std::io::Write;

    let original = b"BIOP compressed module data ".repeat(20);
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&original).unwrap();
    let compressed = enc.finish().unwrap();

    let decompressed = decompress_zlib(&compressed).unwrap();
    assert_eq!(
        decompressed.as_slice(),
        original.as_slice(),
        "zlib round-trip must be byte-exact"
    );
}
