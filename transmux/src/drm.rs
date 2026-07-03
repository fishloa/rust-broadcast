//! Multi-DRM `pssh` init-data payload generation.
//!
//! The [`ProtectionSystemSpecificHeaderBox`] container (`pssh`,
//! ISO/IEC 23001-7 §12.1) is reused from [`crate::cenc`]; this module builds
//! the **system-specific `Data` payloads** and the convenience builders that
//! assemble a complete `pssh` box per DRM system.
//!
//! Byte layouts are transcribed in `transmux/docs/drm/pssh.md`:
//!
//! - **PlayReady** — the PlayReady Object (PRO) framing (`u32` LE length,
//!   `u16` LE record count, type-`0x0001` PlayReady Header record) wrapping a
//!   WRMHEADER XML encoded as UTF-16LE. Sources: Microsoft PlayReady Header
//!   Specification (`docs/drm/pssh.md` §3). The KID inside the WRMHEADER is the
//!   base64 of the 16-byte key-id in PlayReady little-endian GUID layout — see
//!   [`cenc_kid_to_playready`] and `docs/drm/pssh.md` §3.3.
//! - **Widevine** — the `WidevineCencHeader` protobuf (`docs/drm/pssh.md` §4).
//!   Source: shaka-packager `widevine_pssh_data.proto`. Field tags and wire
//!   types are hand-encoded (no protobuf crate).
//! - **FairPlay** — the `skd://` URI convention (`docs/drm/pssh.md` §5). This
//!   is a packager convention, **not** a formal Apple specification.
//! - **DRM system-ID UUIDs** — `docs/drm/pssh.md` §1 (DASH-IF registry).

use alloc::string::String;
use alloc::vec::Vec;

use crate::cenc::ProtectionSystemSpecificHeaderBox;
use crate::error::{Error, Result};
use crate::rtp::{base64_decode, base64_encode};

// ---------------------------------------------------------------------------
// DRM System IDs — docs/drm/pssh.md §1 (DASH-IF content-protection registry)
// ---------------------------------------------------------------------------

/// Widevine DRM system ID: `edef8ba9-79d6-4ace-a3c8-27dcd51d21ed`.
///
/// docs/drm/pssh.md §1.
pub const WIDEVINE_SYSTEM_ID: [u8; 16] = [
    0xED, 0xEF, 0x8B, 0xA9, 0x79, 0xD6, 0x4A, 0xCE, 0xA3, 0xC8, 0x27, 0xDC, 0xD5, 0x1D, 0x21, 0xED,
];

/// PlayReady DRM system ID: `9a04f079-9840-4286-ab92-e65be0885f95`.
///
/// docs/drm/pssh.md §1.
pub const PLAYREADY_SYSTEM_ID: [u8; 16] = [
    0x9A, 0x04, 0xF0, 0x79, 0x98, 0x40, 0x42, 0x86, 0xAB, 0x92, 0xE6, 0x5B, 0xE0, 0x88, 0x5F, 0x95,
];

/// FairPlay (Apple) DRM system ID: `94ce86fb-07ff-4f43-adb8-93d2fa968ca2`.
///
/// docs/drm/pssh.md §1.
pub const FAIRPLAY_SYSTEM_ID: [u8; 16] = [
    0x94, 0xCE, 0x86, 0xFB, 0x07, 0xFF, 0x4F, 0x43, 0xAD, 0xB8, 0x93, 0xD2, 0xFA, 0x96, 0x8C, 0xA2,
];

/// W3C Common / ClearKey (`pssh` box) system ID: `1077efec-c0b2-4d02-ace3-3c1e52e2fb4b`.
///
/// docs/drm/pssh.md §1.
pub const COMMON_SYSTEM_ID: [u8; 16] = [
    0x10, 0x77, 0xEF, 0xEC, 0xC0, 0xB2, 0x4D, 0x02, 0xAC, 0xE3, 0x3C, 0x1E, 0x52, 0xE2, 0xFB, 0x4B,
];

// ---------------------------------------------------------------------------
// PlayReady KID byte-order — docs/drm/pssh.md §3.3
// ---------------------------------------------------------------------------

/// Convert a CENC big-endian UUID key-id into PlayReady little-endian GUID byte
/// layout (docs/drm/pssh.md §3.3).
///
/// Windows GUID memory layout stores `Data1` (`[0:4]`) and `Data2`/`Data3`
/// (`[4:6]`/`[6:8]`) little-endian, and `Data4` (`[8:16]`) as-is:
/// reverse `[0:4]`, reverse `[4:6]`, reverse `[6:8]`, keep `[8:16]`.
///
/// This transform is its own inverse composed with [`playready_kid_to_cenc`].
pub const fn cenc_kid_to_playready(uuid: [u8; 16]) -> [u8; 16] {
    [
        uuid[3], uuid[2], uuid[1], uuid[0], // Data1 (DWORD) reversed
        uuid[5], uuid[4], // Data2 (WORD) reversed
        uuid[7], uuid[6], // Data3 (WORD) reversed
        uuid[8], uuid[9], uuid[10], uuid[11], uuid[12], uuid[13], uuid[14], uuid[15], // Data4
    ]
}

/// Inverse of [`cenc_kid_to_playready`]: PlayReady LE-GUID bytes → CENC BE UUID.
///
/// The swap pattern is symmetric (reverse `[0:4]`, `[4:6]`, `[6:8]`; keep
/// `[8:16]`), so this is the same permutation applied again.
pub const fn playready_kid_to_cenc(guid: [u8; 16]) -> [u8; 16] {
    [
        guid[3], guid[2], guid[1], guid[0], guid[5], guid[4], guid[7], guid[6], guid[8], guid[9],
        guid[10], guid[11], guid[12], guid[13], guid[14], guid[15],
    ]
}

// ---------------------------------------------------------------------------
// PlayReady — WRMHEADER XML + PlayReady Object (PRO) — docs/drm/pssh.md §3
// ---------------------------------------------------------------------------

/// WRMHEADER XML namespace (docs/drm/pssh.md §3.2).
const WRMHEADER_NAMESPACE: &str = "http://schemas.microsoft.com/DRM/2007/03/PlayReadyHeader";

/// PlayReady content-encryption algorithm id used in the WRMHEADER `ALGID`
/// attribute (docs/drm/pssh.md §3.2). AES-128-CTR is the CENC default.
const PLAYREADY_ALGID_AESCTR: &str = "AESCTR";

/// PlayReady Object Record type: PlayReady Header (PRH), holding the WRMHEADER
/// XML (docs/drm/pssh.md §3.1).
const PRO_RECORD_TYPE_HEADER: u16 = 0x0001;

/// Build a WRMHEADER v4.2.0.0 XML string (docs/drm/pssh.md §3.2).
///
/// v4.2.0.0 supports multiple KIDs via a `<KIDS>` container. The `VALUE`
/// attribute of each `<KID>` is base64 of the key-id in PlayReady LE-GUID
/// layout (see [`cenc_kid_to_playready`]). Attributes are emitted in the
/// required alphabetical order (`ALGID` before `VALUE`); `CHECKSUM` is omitted.
///
/// `kids` are CENC big-endian UUID key-ids. `la_url`, if present, becomes the
/// `<LA_URL>` element.
pub fn playready_wrmheader(kids: &[[u8; 16]], la_url: Option<&str>) -> String {
    let version = "4.2.0.0";
    let mut xml = String::new();
    xml.push_str("<WRMHEADER xmlns=\"");
    xml.push_str(WRMHEADER_NAMESPACE);
    xml.push_str("\" version=\"");
    xml.push_str(version);
    xml.push_str("\"><DATA><PROTECTINFO><KIDS>");
    for kid in kids {
        let guid = cenc_kid_to_playready(*kid);
        let value = base64_encode(&guid);
        // Attributes alphabetical: ALGID before VALUE (docs/drm/pssh.md §3.2).
        xml.push_str("<KID ALGID=\"");
        xml.push_str(PLAYREADY_ALGID_AESCTR);
        xml.push_str("\" VALUE=\"");
        xml.push_str(&value);
        xml.push_str("\"></KID>");
    }
    xml.push_str("</KIDS></PROTECTINFO>");
    if let Some(url) = la_url {
        xml.push_str("<LA_URL>");
        xml.push_str(&xml_escape(url));
        xml.push_str("</LA_URL>");
    }
    xml.push_str("</DATA></WRMHEADER>");
    xml
}

/// Minimal XML text escaping for element content (`&`, `<`, `>`).
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// Encode a `&str` as UTF-16LE bytes (no BOM), as required for the WRMHEADER
/// record value (docs/drm/pssh.md §3.2).
fn utf16le_bytes(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2);
    for unit in s.encode_utf16() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

/// Build a PlayReady Object (PRO) wrapping a single PlayReady Header record
/// (docs/drm/pssh.md §3.1).
///
/// Layout: `u32` LE total length, `u16` LE record count (= 1), then one record:
/// `u16` LE type (`0x0001`), `u16` LE value length, UTF-16LE WRMHEADER value.
///
/// `kids` are CENC big-endian UUID key-ids; the WRMHEADER stores them in
/// PlayReady LE-GUID layout.
pub fn playready_pro(kids: &[[u8; 16]], la_url: Option<&str>) -> Vec<u8> {
    let wrm = playready_wrmheader(kids, la_url);
    let value = utf16le_bytes(&wrm);
    let record_count: u16 = 1;
    // total = 4 (length) + 2 (count) + 2 (type) + 2 (value length) + value
    let total_len = 4 + 2 + 2 + 2 + value.len();
    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(&(total_len as u32).to_le_bytes());
    out.extend_from_slice(&record_count.to_le_bytes());
    out.extend_from_slice(&PRO_RECORD_TYPE_HEADER.to_le_bytes());
    out.extend_from_slice(&(value.len() as u16).to_le_bytes());
    out.extend_from_slice(&value);
    out
}

/// Assemble a complete PlayReady `pssh` box (version 0).
///
/// `kids` are CENC big-endian UUID key-ids; the `Data` payload is the PRO from
/// [`playready_pro`]. docs/drm/pssh.md §2–3.
pub fn playready_pssh(
    kids: &[[u8; 16]],
    la_url: Option<&str>,
) -> ProtectionSystemSpecificHeaderBox {
    ProtectionSystemSpecificHeaderBox {
        version: 0,
        system_id: PLAYREADY_SYSTEM_ID,
        kids: Vec::new(),
        data: playready_pro(kids, la_url),
    }
}

// ---------------------------------------------------------------------------
// Widevine — WidevineCencHeader protobuf — docs/drm/pssh.md §4
// ---------------------------------------------------------------------------

/// Protobuf wire type for length-delimited fields (wire type 2).
const PROTOBUF_WIRETYPE_LEN: u8 = 2;
/// Protobuf wire type for varint fields (wire type 0).
const PROTOBUF_WIRETYPE_VARINT: u8 = 0;

/// Widevine `WidevineCencHeader` field number: `key_id` (repeated bytes).
const WV_FIELD_KEY_ID: u8 = 2;
/// Widevine `WidevineCencHeader` field number: `provider` (string).
const WV_FIELD_PROVIDER: u8 = 3;
/// Widevine `WidevineCencHeader` field number: `content_id` (bytes).
const WV_FIELD_CONTENT_ID: u8 = 4;
/// Widevine `WidevineCencHeader` field number: `protection_scheme` (uint32).
const WV_FIELD_PROTECTION_SCHEME: u8 = 9;

/// Compute a protobuf field tag byte: `(field_number << 3) | wire_type`
/// (docs/drm/pssh.md §4).
const fn pb_tag(field: u8, wire_type: u8) -> u8 {
    (field << 3) | wire_type
}

/// Append a base-128 varint (protobuf, little-endian groups).
fn pb_put_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// Append a length-delimited protobuf field (tag, length varint, then bytes).
fn pb_put_len_delimited(out: &mut Vec<u8>, field: u8, bytes: &[u8]) {
    out.push(pb_tag(field, PROTOBUF_WIRETYPE_LEN));
    pb_put_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

/// Build a Widevine `WidevineCencHeader` protobuf payload (docs/drm/pssh.md §4).
///
/// - `key_ids` — repeated field 2, each a 16-byte CENC big-endian UUID.
/// - `provider` — field 3 string (e.g. `"widevine_test"`).
/// - `protection_scheme` — field 9 varint FourCC (`'cenc'` = `0x63656E63`,
///   `'cbcs'` = `0x63626373`).
///
/// The protobuf is hand-encoded; fields are emitted in ascending field-number
/// order (2, 3, 4, 9).
pub fn widevine_pssh_data(
    key_ids: &[[u8; 16]],
    provider: Option<&str>,
    protection_scheme: Option<[u8; 4]>,
) -> Vec<u8> {
    let mut out = Vec::new();
    for kid in key_ids {
        pb_put_len_delimited(&mut out, WV_FIELD_KEY_ID, kid);
    }
    if let Some(p) = provider {
        pb_put_len_delimited(&mut out, WV_FIELD_PROVIDER, p.as_bytes());
    }
    // content_id is not populated by this builder but the field const is
    // exported for completeness / decode-side symmetry.
    let _ = WV_FIELD_CONTENT_ID;
    if let Some(scheme) = protection_scheme {
        out.push(pb_tag(WV_FIELD_PROTECTION_SCHEME, PROTOBUF_WIRETYPE_VARINT));
        pb_put_varint(&mut out, u32::from_be_bytes(scheme) as u64);
    }
    out
}

/// Assemble a complete Widevine `pssh` box (version 0).
///
/// `kids` are CENC big-endian UUID key-ids; the `Data` payload is the
/// `WidevineCencHeader` protobuf from [`widevine_pssh_data`]. docs/drm/pssh.md §2, §4.
pub fn widevine_pssh(
    kids: &[[u8; 16]],
    provider: Option<&str>,
) -> ProtectionSystemSpecificHeaderBox {
    ProtectionSystemSpecificHeaderBox {
        version: 0,
        system_id: WIDEVINE_SYSTEM_ID,
        kids: Vec::new(),
        data: widevine_pssh_data(kids, provider, None),
    }
}

// ---------------------------------------------------------------------------
// FairPlay — skd:// URI convention — docs/drm/pssh.md §5
// ---------------------------------------------------------------------------

/// FairPlay `pssh` `Data`: the UTF-8 bytes of the `skd://` URI.
///
/// FairPlay Streaming has **no public PSSH payload specification**; the common
/// packager convention is to carry the `skd://<asset-id>` URI as UTF-8 bytes
/// with no additional framing (docs/drm/pssh.md §5). The actual SPC/CKC key
/// exchange is Apple's NDA-gated protocol.
pub fn fairplay_pssh_data(skd_uri: &str) -> Vec<u8> {
    skd_uri.as_bytes().to_vec()
}

/// Assemble a FairPlay `pssh` box (version 0) carrying the `skd://` URI as its
/// `Data` payload (docs/drm/pssh.md §5, convention — not a formal spec).
pub fn fairplay_pssh(skd_uri: &str) -> ProtectionSystemSpecificHeaderBox {
    ProtectionSystemSpecificHeaderBox {
        version: 0,
        system_id: FAIRPLAY_SYSTEM_ID,
        kids: Vec::new(),
        data: fairplay_pssh_data(skd_uri),
    }
}

// ---------------------------------------------------------------------------
// Base64 helpers scoped to the WRMHEADER VALUE round-trip (thin re-exports).
// ---------------------------------------------------------------------------

/// Base64-decode the WRMHEADER `<KID>` `VALUE`/element content back to the
/// 16-byte PlayReady LE-GUID key-id (docs/drm/pssh.md §3.2). Errors on invalid
/// base64 or a length other than 16 bytes.
pub fn playready_kid_value_decode(value: &str) -> Result<[u8; 16]> {
    let bytes = base64_decode(value)?;
    if bytes.len() != 16 {
        return Err(Error::InvalidValue {
            field: "playready KID VALUE",
            value: bytes.len() as u64,
            reason: "expected 16 decoded bytes",
        });
    }
    let mut out = [0u8; 16];
    out.copy_from_slice(&bytes);
    Ok(out)
}
