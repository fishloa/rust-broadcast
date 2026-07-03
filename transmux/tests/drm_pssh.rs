//! Biting tests for multi-DRM `pssh` init-data generation (issue #480).
//!
//! Spec basis: `transmux/docs/drm/pssh.md`.

use transmux::cenc::ProtectionSystemSpecificHeaderBox;
use transmux::drm::{
    PLAYREADY_SYSTEM_ID, WIDEVINE_SYSTEM_ID, cenc_kid_to_playready, playready_kid_to_cenc,
    playready_pro, playready_pssh, playready_wrmheader, widevine_pssh, widevine_pssh_data,
};
use transmux::rtp::base64_decode;

/// Documented vector (docs/drm/pssh.md §3.3 pattern):
/// UUID 01020304-0506-0708-090a-0b0c0d0e0f10.
const KID_A: [u8; 16] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
];
/// PlayReady LE-GUID of KID_A: reverse [0:4], [4:6], [6:8], keep [8:16].
const KID_A_PLAYREADY: [u8; 16] = [
    0x04, 0x03, 0x02, 0x01, 0x06, 0x05, 0x08, 0x07, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
];

const KID_B: [u8; 16] = [
    0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20,
];

// ---------------------------------------------------------------------------
// Test 1 — PlayReady KID byte-swap bites.
// ---------------------------------------------------------------------------

#[test]
fn playready_kid_swap_matches_documented_vector() {
    // Known vector from docs/drm/pssh.md §3.3.
    assert_eq!(cenc_kid_to_playready(KID_A), KID_A_PLAYREADY);
    // Inverse round-trips to the original CENC UUID.
    assert_eq!(playready_kid_to_cenc(cenc_kid_to_playready(KID_A)), KID_A);
    // Also verify the spec's own worked example (VALUE PV1LM/VEVk+kEOB8qqcWDg==
    // → CENC 334B5D3D-44F5-4F56-A410-E07CAAA7160E).
    let pr_bytes = base64_decode("PV1LM/VEVk+kEOB8qqcWDg==").unwrap();
    let mut guid = [0u8; 16];
    guid.copy_from_slice(&pr_bytes);
    let cenc = playready_kid_to_cenc(guid);
    assert_eq!(
        cenc,
        [
            0x33, 0x4B, 0x5D, 0x3D, 0x44, 0xF5, 0x4F, 0x56, 0xA4, 0x10, 0xE0, 0x7C, 0xAA, 0xA7,
            0x16, 0x0E
        ]
    );

    // The WRMHEADER <KID VALUE=...> base64 decodes to the swapped LE-GUID bytes.
    let xml = playready_wrmheader(&[KID_A], None);
    let value = extract_attr(&xml, "VALUE");
    let decoded = base64_decode(&value).unwrap();
    assert_eq!(&decoded[..], &KID_A_PLAYREADY[..]);
}

/// Extract the value of the first `attr="..."` occurrence from an XML string.
fn extract_attr(xml: &str, attr: &str) -> String {
    let needle = format!("{attr}=\"");
    let start = xml.find(&needle).expect("attr present") + needle.len();
    let end = xml[start..].find('"').expect("closing quote") + start;
    xml[start..end].to_string()
}

// ---------------------------------------------------------------------------
// Test 2 — PlayReady PRO structure bites.
// ---------------------------------------------------------------------------

#[test]
fn playready_pro_structure_parses_back() {
    let la = "https://la.example.com/rights";
    let pro = playready_pro(&[KID_A], Some(la));

    // u32 LE length == buffer length.
    let declared_len = u32::from_le_bytes([pro[0], pro[1], pro[2], pro[3]]) as usize;
    assert_eq!(declared_len, pro.len());

    // u16 LE record count == 1.
    let count = u16::from_le_bytes([pro[4], pro[5]]);
    assert_eq!(count, 1);

    // Record type == 0x0001 (PlayReady Header).
    let rec_type = u16::from_le_bytes([pro[6], pro[7]]);
    assert_eq!(rec_type, 0x0001);

    // Record length matches remaining bytes.
    let rec_len = u16::from_le_bytes([pro[8], pro[9]]) as usize;
    assert_eq!(rec_len, pro.len() - 10);

    // UTF-16LE WRMHEADER decodes and contains KID VALUE + LA_URL.
    let units: Vec<u16> = pro[10..]
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let wrm = String::from_utf16(&units).unwrap();
    assert!(wrm.contains("<WRMHEADER"));
    assert!(wrm.contains(la), "LA_URL present");
    let expected_value = transmux::rtp::base64_encode(&KID_A_PLAYREADY);
    assert!(wrm.contains(&expected_value), "KID VALUE present");

    // Mutating a KID changes the output bytes.
    let pro2 = playready_pro(&[KID_B], Some(la));
    assert_ne!(pro, pro2);
}

// ---------------------------------------------------------------------------
// Test 3 — Widevine protobuf bites.
// ---------------------------------------------------------------------------

/// Minimal protobuf walker returning (field_number, wire_type, payload) tuples.
/// For varints, `payload` is the raw varint value bytes re-encoded; simpler to
/// decode inline in the test, so we return raw bytes for len-delimited and the
/// varint value for wire type 0.
enum PbField {
    LenDelim { field: u8, bytes: Vec<u8> },
    Varint { field: u8, value: u64 },
}

fn pb_walk(mut data: &[u8]) -> Vec<PbField> {
    let mut out = Vec::new();
    while !data.is_empty() {
        let tag = data[0];
        data = &data[1..];
        let field = tag >> 3;
        let wire = tag & 0x07;
        match wire {
            2 => {
                let (len, rest) = pb_read_varint(data);
                let len = len as usize;
                out.push(PbField::LenDelim {
                    field,
                    bytes: rest[..len].to_vec(),
                });
                data = &rest[len..];
            }
            0 => {
                let (val, rest) = pb_read_varint(data);
                out.push(PbField::Varint { field, value: val });
                data = rest;
            }
            _ => panic!("unexpected wire type {wire}"),
        }
    }
    out
}

fn pb_read_varint(data: &[u8]) -> (u64, &[u8]) {
    let mut value = 0u64;
    let mut shift = 0;
    let mut i = 0;
    loop {
        let b = data[i];
        value |= ((b & 0x7F) as u64) << shift;
        i += 1;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    (value, &data[i..])
}

#[test]
fn widevine_proto_encoding_bites() {
    let scheme = *b"cenc"; // 0x63656E63
    let data = widevine_pssh_data(&[KID_A, KID_B], Some("widevine_test"), Some(scheme));
    let fields = pb_walk(&data);

    // key_ids: field 2, in order, exact bytes.
    let key_ids: Vec<&Vec<u8>> = fields
        .iter()
        .filter_map(|f| match f {
            PbField::LenDelim { field: 2, bytes } => Some(bytes),
            _ => None,
        })
        .collect();
    assert_eq!(key_ids.len(), 2, "two field-2 entries");
    assert_eq!(key_ids[0][..], KID_A[..]);
    assert_eq!(key_ids[1][..], KID_B[..]);

    // provider: field 3 string.
    let provider = fields.iter().find_map(|f| match f {
        PbField::LenDelim { field: 3, bytes } => Some(String::from_utf8(bytes.clone()).unwrap()),
        _ => None,
    });
    assert_eq!(provider.as_deref(), Some("widevine_test"));

    // protection_scheme: field 9 varint == FourCC big-endian value.
    let ps = fields.iter().find_map(|f| match f {
        PbField::Varint { field: 9, value } => Some(*value),
        _ => None,
    });
    assert_eq!(ps, Some(u32::from_be_bytes(scheme) as u64));

    // Mutating a KID changes the bytes.
    let data2 = widevine_pssh_data(&[KID_B, KID_B], Some("widevine_test"), Some(scheme));
    assert_ne!(data, data2);
}

#[test]
fn widevine_minimal_matches_spec_vector() {
    // docs/drm/pssh.md §4 "Minimum valid PSSH Data" — one key_id, no algorithm
    // set by our builder (we omit field 1). Expect exactly: 12 10 <16 bytes>.
    let data = widevine_pssh_data(&[KID_A], None, None);
    let mut expected = Vec::new();
    expected.push(0x12); // field 2, wire type 2
    expected.push(0x10); // length 16
    expected.extend_from_slice(&KID_A);
    assert_eq!(data, expected);
}

// ---------------------------------------------------------------------------
// Test 4 — pssh box round-trip (serialize computes lengths from fields).
// ---------------------------------------------------------------------------

#[test]
fn widevine_pssh_box_round_trip() {
    let pssh = widevine_pssh(&[KID_A], Some("widevine_test"));
    let bytes = pssh.to_vec().unwrap();

    // First 4 bytes = total size == buffer length (lengths from fields).
    let size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    assert_eq!(size, bytes.len());
    assert_eq!(&bytes[4..8], b"pssh");

    let parsed = ProtectionSystemSpecificHeaderBox::parse_box(&bytes).unwrap();
    assert_eq!(parsed.system_id, WIDEVINE_SYSTEM_ID);
    assert_eq!(parsed.data, pssh.data);
    assert_eq!(parsed, pssh);
}

#[test]
fn playready_pssh_box_round_trip() {
    let pssh = playready_pssh(&[KID_A, KID_B], Some("https://la.example.com"));
    let bytes = pssh.to_vec().unwrap();

    let size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    assert_eq!(size, bytes.len());

    let parsed = ProtectionSystemSpecificHeaderBox::parse_box(&bytes).unwrap();
    assert_eq!(parsed.system_id, PLAYREADY_SYSTEM_ID);
    assert_eq!(parsed.data, pssh.data);
    assert_eq!(parsed, pssh);
}
