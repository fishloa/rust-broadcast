//! Real-fixture test — byte-level structural assertions against the genuine
//! ATSC 3.0 ROUTE/LCT capture fixtures in `fixtures/atsc3/route-*.bin`
//! (issue #943 milestone 2/3). Full field-by-field provenance is in
//! `fixtures/atsc3/PROVENANCE.md`.
//!
//! `atsc3` currently implements only LLS (`atsc3::lls`) and SLT
//! (`atsc3::slt`) — there is **no** ROUTE/ALC/LCT/FLUTE parser in this crate
//! (that is issue #943's job, not this task's). So unlike
//! `fixture_slt.rs` (which exercises `atsc3::lls::LlsEnvelope` and
//! `atsc3::slt::Slt` end to end), this file cannot call into any crate API
//! for the LCT-framed packets — it hand-decodes the LCT header
//! (RFC 5651 §5.1, transcribed independently in `rmt-flute/docs/lct.md` and
//! `rmt-flute/docs/alc.md`, neither of which `atsc3` depends on) directly
//! against the documented byte layout, the same discipline
//! `webrtc-runtime/tests/whip_smoke_pcap_stun.rs` uses for its hand-rolled
//! pcap walker. It does **not** implement a ROUTE parser (no `Parse`/
//! `Serialize` impl, no public API) — only test-local, one-shot field
//! extraction to prove the fixture's documented byte layout is real.
//!
//! The one piece of the SLS package this crate's *own* XML-aware API
//! (`atsc3::slt::Slt::parse`) can legitimately be pointed at is checked in
//! `sls_package_usbd_is_not_a_valid_slt_document` below — proving the
//! parser correctly recognises a real, non-SLT ATSC 3.0 document as not an
//! SLT document, rather than silently mis-accepting it.

use atsc3::slt::Slt;

fn fixture_path(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("atsc3")
        .join(name)
}

fn read_fixture(name: &str) -> Vec<u8> {
    let path = fixture_path(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read fixture {path:?}: {e}"))
}

// ---------------------------------------------------------------------------
// Hand-decode of the LCT header (RFC 5651 §5.1) — test-local only, not a
// crate API. Field names/widths match `rmt-flute/docs/lct.md` exactly.
// ---------------------------------------------------------------------------

/// The fixed 16-bit first word's flag fields, plus `HDR_LEN`/`CP`.
#[derive(Debug, PartialEq, Eq)]
struct LctFlags {
    v: u8,
    c: u8,
    psi: u8,
    s: u8,
    o: u8,
    h: u8,
    res: u8,
    a: u8,
    b: u8,
    hdr_len_words: u8,
    cp: u8,
}

fn decode_lct_flags(bytes: &[u8]) -> LctFlags {
    let word = u16::from_be_bytes([bytes[0], bytes[1]]);
    LctFlags {
        v: ((word >> 12) & 0xF) as u8,
        c: ((word >> 10) & 0x3) as u8,
        psi: ((word >> 8) & 0x3) as u8,
        s: ((word >> 7) & 0x1) as u8,
        o: ((word >> 5) & 0x3) as u8,
        h: ((word >> 4) & 0x1) as u8,
        res: ((word >> 2) & 0x3) as u8,
        a: ((word >> 1) & 0x1) as u8,
        b: (word & 0x1) as u8,
        hdr_len_words: bytes[2],
        cp: bytes[3],
    }
}

/// One decoded LCT packet: the fixed flags, the variable CCI/TSI/TOI
/// fields, and the byte offset where the LCT header ends (`HDR_LEN * 4`).
struct DecodedLct<'a> {
    flags: LctFlags,
    cci: &'a [u8],
    tsi: Option<u64>,
    toi: Option<u64>,
    header_end: usize,
    /// The raw extension region, `bytes[.. header_end]` minus fixed+CCI+TSI+TOI.
    extensions: &'a [u8],
}

fn decode_lct(bytes: &[u8]) -> DecodedLct<'_> {
    let flags = decode_lct_flags(bytes);
    let mut off = 4usize;

    let cci_len = 4 * (usize::from(flags.c) + 1);
    let cci = &bytes[off..off + cci_len];
    off += cci_len;

    let tsi_len = 4 * usize::from(flags.s) + 2 * usize::from(flags.h);
    let tsi = if tsi_len > 0 {
        Some(be_uint(&bytes[off..off + tsi_len]))
    } else {
        None
    };
    off += tsi_len;

    let toi_len = 4 * usize::from(flags.o) + 2 * usize::from(flags.h);
    let toi = if toi_len > 0 {
        Some(be_uint(&bytes[off..off + toi_len]))
    } else {
        None
    };
    off += toi_len;

    let header_end = usize::from(flags.hdr_len_words) * 4;
    let extensions = &bytes[off..header_end];

    DecodedLct {
        flags,
        cci,
        tsi,
        toi,
        header_end,
        extensions,
    }
}

fn be_uint(bytes: &[u8]) -> u64 {
    let mut v = 0u64;
    for &b in bytes {
        v = (v << 8) | u64::from(b);
    }
    v
}

/// One HET-64 (variable-length, `EXT_FTI`) extension: `HET`, `HEL` (32-bit
/// words), and the `HEC` content (`rmt-flute/docs/lct.md` "Variable-length
/// extension").
struct VarExt<'a> {
    het: u8,
    hel: u8,
    hec: &'a [u8],
}

fn decode_var_ext(bytes: &[u8]) -> VarExt<'_> {
    let het = bytes[0];
    let hel = bytes[1];
    let total = usize::from(hel) * 4;
    VarExt {
        het,
        hel,
        hec: &bytes[2..total],
    }
}

/// One HET-192+ (fixed-length) extension: exactly one 32-bit word, `HET` +
/// 24-bit `HEC` (`rmt-flute/docs/lct.md` "Fixed-length extension").
struct FixedExt<'a> {
    het: u8,
    hec: &'a [u8],
}

fn decode_fixed_ext(bytes: &[u8]) -> FixedExt<'_> {
    FixedExt {
        het: bytes[0],
        hec: &bytes[1..4],
    }
}

// ---------------------------------------------------------------------------
// route-fdt-instance-2020-11-05.bin
// ---------------------------------------------------------------------------

const FDT_FIXTURE: &str = "route-fdt-instance-2020-11-05.bin";

/// LCT header + EXT_FTI + EXT_FDT byte layout matches `PROVENANCE.md`'s
/// documented decode exactly.
#[test]
fn fdt_instance_lct_header_matches_documented_layout() {
    let bytes = read_fixture(FDT_FIXTURE);
    assert_eq!(bytes.len(), 439);

    let lct = decode_lct(&bytes);
    assert_eq!(
        lct.flags,
        LctFlags {
            v: 1,
            c: 0,
            psi: 0b10, // X=SPI=1, Y=0
            s: 1,
            o: 1,
            h: 0,
            res: 0,
            a: 0,
            b: 1,             // Close-Object flag set
            hdr_len_words: 9, // 36 bytes
            cp: 4,            // ROUTE Table A.3.6: Signed Package Mode
        }
    );
    assert_eq!(lct.cci, [0, 0, 0, 0]);
    assert_eq!(lct.tsi, Some(0));
    assert_eq!(lct.toi, Some(0));
    assert_eq!(lct.header_end, 36);
    assert_eq!(lct.extensions.len(), 20);

    // Ext 1: EXT_FTI, HET=64, HEL=4 (16-byte extension, 14-byte HEC).
    let fti = decode_var_ext(lct.extensions);
    assert_eq!(fti.het, 64);
    assert_eq!(fti.hel, 4);
    assert_eq!(
        fti.hec,
        [
            0x00, 0x00, 0x00, 0x00, 0x01, 0x8f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00
        ]
    );
    // Bytes 4-5 of the HEC = 0x018F = 399, matching this exact packet's own
    // FDT-Instance XML payload length (verified below) -- corroborating
    // evidence this is a Transfer-Length-bearing field.
    let transfer_length_hint = u16::from_be_bytes([fti.hec[4], fti.hec[5]]);
    assert_eq!(transfer_length_hint, 399);

    // Ext 2: EXT_FDT, HET=192 (fixed-length): V(4 bits, high nibble)=2,
    // FDT Instance ID (20 bits)=0.
    let fdt_ext = decode_fixed_ext(&lct.extensions[16..20]);
    assert_eq!(fdt_ext.het, 192);
    assert_eq!(fdt_ext.hec, [0x20, 0x00, 0x00]);
    assert_eq!(fdt_ext.hec[0] >> 4, 2, "EXT_FDT V (FLUTE version)");

    // FEC Payload ID (4 bytes, Compact No-Code start_offset) = 0, then the
    // 399-byte FDT-Instance XML payload.
    let rest = &bytes[lct.header_end..];
    let fec_payload_id = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]);
    assert_eq!(fec_payload_id, 0);
    let payload = &rest[4..];
    assert_eq!(payload.len(), 399);
    assert_eq!(payload.len(), usize::from(transfer_length_hint));

    let xml = std::str::from_utf8(payload).expect("FDT-Instance payload is valid UTF-8");
    assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
    assert!(xml.contains("<FDT-Instance"));
    assert!(xml.contains(r#"Expires="4294967295""#));
    assert!(xml.contains(r#"afdt:efdtVersion="74""#));
    assert!(xml.contains(r#"TOI="458826""#));
    assert!(xml.contains(r#"Content-Location="sls""#));
    assert!(xml.contains(r#"Content-Length="6758""#));
    assert!(xml.contains(r#"Content-Type="multipart/signed""#));
}

// ---------------------------------------------------------------------------
// route-media-{video,audio}-fragment-2020-11-05.bin
// ---------------------------------------------------------------------------

struct MediaFragmentExpectation {
    fixture: &'static str,
    tsi: u64,
    fti_hec: [u8; 14],
    start_offset: u32,
}

const VIDEO_FRAGMENT: MediaFragmentExpectation = MediaFragmentExpectation {
    fixture: "route-media-video-fragment-2020-11-05.bin",
    tsi: 3000,
    fti_hec: [
        0x00, 0x00, 0x00, 0x06, 0xff, 0xf3, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ],
    start_offset: 107_008,
};

const AUDIO_FRAGMENT: MediaFragmentExpectation = MediaFragmentExpectation {
    fixture: "route-media-audio-fragment-2020-11-05.bin",
    tsi: 3003,
    fti_hec: [
        0x00, 0x00, 0x00, 0x00, 0x80, 0xee, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ],
    start_offset: 26_752,
};

/// LCT header of each media LCT channel matches `PROVENANCE.md`'s
/// documented decode: TSI 3000 (video) / 3003 (audio), shared TOI 6034,
/// `CP=128`, and the exact (unresolved-semantics, per `PROVENANCE.md`)
/// EXT_FTI HEC bytes.
#[test]
fn media_fragment_lct_headers_match_documented_layout() {
    for expect in [VIDEO_FRAGMENT, AUDIO_FRAGMENT] {
        let bytes = read_fixture(expect.fixture);
        assert_eq!(bytes.len(), 1444, "{}", expect.fixture);

        let lct = decode_lct(&bytes);
        assert_eq!(lct.flags.v, 1, "{}", expect.fixture);
        assert_eq!(lct.flags.s, 1, "{}", expect.fixture);
        assert_eq!(lct.flags.o, 1, "{}", expect.fixture);
        assert_eq!(lct.flags.h, 0, "{}", expect.fixture);
        assert_eq!(lct.flags.hdr_len_words, 8, "{}", expect.fixture); // 32 bytes
        assert_eq!(lct.flags.cp, 128, "{}", expect.fixture);
        assert_eq!(lct.tsi, Some(expect.tsi), "{}", expect.fixture);
        assert_eq!(lct.toi, Some(6034), "{}", expect.fixture);
        assert_eq!(lct.header_end, 32, "{}", expect.fixture);

        let fti = decode_var_ext(lct.extensions);
        assert_eq!(fti.het, 64, "{}", expect.fixture);
        assert_eq!(fti.hel, 4, "{}", expect.fixture);
        assert_eq!(fti.hec, expect.fti_hec, "{}", expect.fixture);

        let rest = &bytes[lct.header_end..];
        let start_offset = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]);
        assert_eq!(start_offset, expect.start_offset, "{}", expect.fixture);

        // Opaque DASH-segment payload -- correctly not decoded by this crate.
        let payload = &rest[4..];
        assert_eq!(payload.len(), 1408, "{}", expect.fixture);
    }
}

/// Cross-validates the byte-decoded TSI values against the real S-TSID XML
/// embedded in the reassembled SLS package, matching
/// `route-sls-signed-package-2020-11-05.bin`'s `<RS tsi="...">` entries --
/// two independent angles on the same session (raw LCT bytes vs. the
/// signalling XML that describes them) agreeing, per `PROVENANCE.md`.
#[test]
fn media_fragment_tsi_matches_s_tsid_in_sls_package() {
    let video = read_fixture(VIDEO_FRAGMENT.fixture);
    let audio = read_fixture(AUDIO_FRAGMENT.fixture);
    let video_tsi = decode_lct(&video).tsi.unwrap();
    let audio_tsi = decode_lct(&audio).tsi.unwrap();
    assert_eq!(video_tsi, 3000);
    assert_eq!(audio_tsi, 3003);

    let sls = read_fixture("route-sls-signed-package-2020-11-05.bin");
    let sls_text = std::str::from_utf8(&sls).expect("SLS package is valid UTF-8");
    assert!(sls_text.contains(&format!(r#"tsi="{video_tsi}""#)));
    assert!(sls_text.contains(&format!(r#"tsi="{audio_tsi}""#)));
}

// ---------------------------------------------------------------------------
// route-sls-signed-package-2020-11-05.bin
// ---------------------------------------------------------------------------

const SLS_PACKAGE_FIXTURE: &str = "route-sls-signed-package-2020-11-05.bin";

/// The reassembled SLS package's declared `Content-Length` (carried in the
/// FDT-Instance fixture above) matches the actual byte length of the
/// committed, separately-reassembled package fixture.
#[test]
fn sls_package_length_matches_fdt_declared_content_length() {
    let sls = read_fixture(SLS_PACKAGE_FIXTURE);
    assert_eq!(sls.len(), 6758);

    let fdt = read_fixture(FDT_FIXTURE);
    let lct = decode_lct(&fdt);
    let xml = std::str::from_utf8(&fdt[lct.header_end + 4..]).unwrap();
    assert!(xml.contains(&format!(r#"Content-Length="{}""#, sls.len())));
}

/// It's a real `multipart/signed` MIME package (RFC 1847), containing the
/// S-TSID/USBD/MPD XML `PROVENANCE.md` documents, plus a PKCS#7 signature
/// part -- verified by simple text search, since `atsc3` has no MIME/S-TSID/
/// USBD parser to hand this to.
#[test]
fn sls_package_contains_documented_mime_parts() {
    let sls = read_fixture(SLS_PACKAGE_FIXTURE);
    let text = std::str::from_utf8(&sls).expect("SLS package is valid UTF-8");

    assert!(text.starts_with("MIME-Version:1.0"));
    assert!(text.contains(r#"Content-Type: multipart/signed"#));
    assert!(text.contains(r#"protocol="application/pkcs7-signature""#));
    assert!(text.contains("stsid.xml") || text.contains("route-s-tsid"));
    assert!(text.contains("usbd.xml") || text.to_lowercase().contains("usbd"));
    assert!(text.contains("<MPD"));
    assert!(text.contains("BundleDescriptionROUTE"));
    assert!(text.to_lowercase().contains("pkcs7"));
    // No RepairFlow element: PROVENANCE.md independently confirms every
    // wire packet in this session has PSI/SPI=1 (source-data), i.e. no FEC
    // repair flow -- cross-checked against the LCT-level PSI bit decoded
    // above (`psi: 0b10`, X=SPI=1).
    assert!(!text.contains("RepairFlow"));
}

/// `atsc3::slt::Slt::parse` is the crate's only XML-aware ROUTE-adjacent
/// API. Pointed at the real USBD (`BundleDescriptionROUTE`) document
/// embedded in the SLS package -- which is genuinely not an SLT document --
/// it correctly rejects it as non-SLT, rather than silently mis-accepting
/// unrelated ATSC 3.0 XML. This is the one place this crate's real parsing
/// code can legitimately touch this fixture's content.
#[test]
fn sls_package_usbd_is_not_a_valid_slt_document() {
    let sls = read_fixture(SLS_PACKAGE_FIXTURE);
    let text = std::str::from_utf8(&sls).unwrap();

    let start = text
        .find("<BundleDescriptionROUTE")
        .expect("USBD root element present");
    let end = text[start..]
        .find("</BundleDescriptionROUTE>")
        .expect("USBD root element closed");
    let usbd_xml = &text[start..start + end + "</BundleDescriptionROUTE>".len()];

    let err = Slt::parse(usbd_xml).expect_err("a real USBD document must not parse as SLT");
    assert!(matches!(
        err,
        atsc3::Error::MissingElement { element: "SLT", .. }
    ));
}

// ---------------------------------------------------------------------------
// Bite-proof
// ---------------------------------------------------------------------------

/// Corrupting the real FDT-instance fixture's `HDR_LEN` byte (in memory --
/// the committed fixture file is never touched) desyncs where the LCT
/// header is believed to end, breaking the FEC-Payload-ID/payload split and
/// thus the recovered XML -- proving the byte-offset assertions above are
/// load-bearing against genuine wire bytes.
#[test]
fn corrupting_hdr_len_breaks_the_documented_decode() {
    let mut bytes = read_fixture(FDT_FIXTURE);
    assert_eq!(bytes[2], 9, "pristine HDR_LEN is 9 words (36 bytes)");
    bytes[2] = 5; // claim a much shorter LCT header (20 bytes)

    let lct = decode_lct(&bytes);
    assert_eq!(lct.header_end, 20);
    // The FEC Payload ID/payload split now starts 16 bytes too early, deep
    // inside what was actually the EXT_FTI extension -- so the recovered
    // "payload" is garbage, not the real FDT-Instance XML.
    let rest = &bytes[lct.header_end..];
    let payload = &rest[4..];
    let xml = std::str::from_utf8(&payload[..payload.len().min(399)]);
    assert!(
        xml.is_err() || !xml.unwrap().starts_with("<?xml"),
        "corrupting HDR_LEN must break recovery of the real FDT-Instance XML"
    );
}
