//! Ungameable gate test for colr / pasp / clap visual sample-entry extension boxes (#434).
//!
//! Extracts `colr` + `pasp` from the real ffmpeg-generated `colr_hdr.mp4` fixture,
//! parses them, asserts oracle values (FMP4-GAPS-ORACLE.md), and byte-exact
//! round-trips each box. Builds a `clap` from a spec vector and round-trips it.
//!
//! EXIT CRITERIA:
//! 1. colr from fixture is nclx with matrix_coefficients=9 (bt2020nc)
//! 2. pasp from fixture round-trips byte-identical
//! 3. colr from fixture round-trips byte-identical
//! 4. Mutating a field → bytes change (proves no raw-passthrough in parser)
//! 5. clap from spec vector build + round-trip

use broadcast_common::{Parse, Serialize};
use transmux::{CleanApertureBox, ColourInformationBox, PixelAspectRatioBox};

// Oracle bytes from FMP4-GAPS-ORACLE.md
const COLR_BODY_ORACLE: [u8; 11] = [
    0x6E, 0x63, 0x6C, 0x78, // 'nclx'
    0x00, 0x02, // colour_primaries = 2
    0x00, 0x02, // transfer_characteristics = 2
    0x00, 0x09, // matrix_coefficients = 9 (bt2020nc)
    0x00, // full_range_flag=0, reserved=0
];

const PASP_BODY_ORACLE: [u8; 8] = [
    0x00, 0x00, 0x00, 0x01, // hSpacing = 1
    0x00, 0x00, 0x00, 0x01, // vSpacing = 1
];

fn load_fixture() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/colr_hdr.mp4");
    std::fs::read(path).expect("fixture must exist")
}

/// Find a direct child box by four-CC (shallow — only walks one level).
fn find_child_box<'a>(region: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    let mut off = 0usize;
    while off + 8 <= region.len() {
        let sz = u32::from_be_bytes([
            region[off],
            region[off + 1],
            region[off + 2],
            region[off + 3],
        ]) as usize;
        if sz < 8 || off + sz > region.len() {
            break;
        }
        if &region[off + 4..off + 8] == fourcc {
            return Some(&region[off..off + sz]);
        }
        off += sz;
    }
    None
}

/// Navigate the box tree: moov → trak → mdia → minf → stbl → stsd → avc1.
/// Then search avc1's config region (after 78-byte VisualSampleEntry fixed
/// fields) for child boxes matching `child_fourcc`. Returns the full box bytes
/// (8-byte header + body).
fn find_visual_ext_box<'a>(data: &'a [u8], child_fourcc: &[u8; 4]) -> &'a [u8] {
    let moov = find_child_box(data, b"moov").expect("moov");
    let moov_body = &moov[8..];
    let trak = find_child_box(moov_body, b"trak").expect("trak");
    let trak_body = &trak[8..];
    let mdia = find_child_box(trak_body, b"mdia").expect("mdia");
    let mdia_body = &mdia[8..];
    let minf = find_child_box(mdia_body, b"minf").expect("minf");
    let minf_body = &minf[8..];
    let stbl = find_child_box(minf_body, b"stbl").expect("stbl");
    let stbl_body = &stbl[8..];

    // stsd is a FullBox: version(8)+flags(24)+entry_count(32) = 8 bytes before entries
    let stsd = find_child_box(stbl_body, b"stsd").expect("stsd");
    let entry_count = u32::from_be_bytes([stsd[12], stsd[13], stsd[14], stsd[15]]) as usize;
    assert!(entry_count >= 1, "stsd must have at least 1 entry");

    // First sample entry starts at stsd body offset 16 (8 box header + 8 FullBox data)
    let entries_start = 16usize;

    // Find avc1 among entries
    let mut off = entries_start;
    for _ in 0..entry_count {
        let sz =
            u32::from_be_bytes([stsd[off], stsd[off + 1], stsd[off + 2], stsd[off + 3]]) as usize;
        let ty = &stsd[off + 4..off + 8];
        if ty == b"avc1" || ty == b"hvc1" || ty == b"encv" {
            // VisualSampleEntry fixed fields = 78 bytes (same module docs say)
            let config_start = off + 8 + 78;
            let entry_end = off + sz;
            let config_region = &stsd[config_start..entry_end];

            // Walk child boxes in the config region
            let mut cfg_off = 0usize;
            while cfg_off + 8 <= config_region.len() {
                let cfg_sz = u32::from_be_bytes([
                    config_region[cfg_off],
                    config_region[cfg_off + 1],
                    config_region[cfg_off + 2],
                    config_region[cfg_off + 3],
                ]) as usize;
                if cfg_sz < 8 || cfg_off + cfg_sz > config_region.len() {
                    break;
                }
                if &config_region[cfg_off + 4..cfg_off + 8] == child_fourcc {
                    return &config_region[cfg_off..cfg_off + cfg_sz];
                }
                cfg_off += cfg_sz;
            }
            panic!(
                "box {:?} not found in visual sample entry config region",
                std::str::from_utf8(child_fourcc).unwrap()
            );
        }
        off += sz;
    }
    panic!("visual sample entry not found in stsd");
}

// ---- EXIT CRITERION 2: pasp from fixture, byte-exact round-trip --------

#[test]
fn pasp_from_fixture_round_trip() {
    let data = load_fixture();
    let pasp_box = find_visual_ext_box(&data, b"pasp");
    let pasp_body = &pasp_box[8..];

    assert_eq!(
        pasp_body, PASP_BODY_ORACLE,
        "pasp body must match oracle (ffmpeg output)"
    );

    let pasp = PixelAspectRatioBox::parse(pasp_body).expect("pasp parse");
    assert_eq!(pasp.h_spacing, 1);
    assert_eq!(pasp.v_spacing, 1);

    let bytes = pasp.to_bytes();
    assert_eq!(
        &bytes, &PASP_BODY_ORACLE,
        "pasp round-trip must be byte-exact"
    );
}

// ---- EXIT CRITERION 1+3: colr from fixture, assert nclx + round-trip ----

#[test]
fn colr_nclx_from_fixture_round_trip() {
    let data = load_fixture();
    let colr_box = find_visual_ext_box(&data, b"colr");
    let colr_body = &colr_box[8..];

    assert_eq!(
        colr_body, COLR_BODY_ORACLE,
        "colr body must match oracle (ffmpeg output)"
    );

    let colr = ColourInformationBox::parse(colr_body).expect("colr parse");
    assert_eq!(&colr.colour_type, b"nclx", "colr must be nclx type");

    let nclx = colr.nclx.as_ref().expect("colr must have nclx params");
    assert_eq!(nclx.colour_primaries, 2, "colour_primaries oracle");
    assert_eq!(
        nclx.transfer_characteristics, 2,
        "transfer_characteristics oracle"
    );
    assert_eq!(
        nclx.matrix_coefficients, 9,
        "matrix_coefficients must be 9 (bt2020nc)"
    );
    assert!(!nclx.full_range_flag, "full_range_flag must be false");

    let bytes = colr.to_bytes();
    assert_eq!(
        &bytes, &COLR_BODY_ORACLE,
        "colr round-trip must be byte-exact"
    );
}

// ---- EXIT CRITERION 4: mutate a field → bytes change -------------------

#[test]
fn colr_mutate_field_changes_bytes() {
    let colr = ColourInformationBox::parse(&COLR_BODY_ORACLE).unwrap();

    let mut colr2 = colr.clone();
    colr2.nclx.as_mut().unwrap().matrix_coefficients = 1;
    assert_ne!(
        colr.to_bytes(),
        colr2.to_bytes(),
        "mutating matrix_coefficients must change serialized bytes"
    );
}

#[test]
fn pasp_mutate_field_changes_bytes() {
    let pasp = PixelAspectRatioBox::parse(&PASP_BODY_ORACLE).unwrap();

    let mut pasp2 = pasp;
    pasp2.h_spacing = 2;
    assert_ne!(
        pasp.to_bytes(),
        pasp2.to_bytes(),
        "mutating h_spacing must change serialized bytes"
    );
}

// ---- EXIT CRITERION 5: clap from spec vector, build + round-trip -------

#[test]
fn clap_spec_vector_round_trip() {
    // 1920x1080 clean aperture, centered
    let clap = CleanApertureBox {
        clean_aperture_width_n: 1920,
        clean_aperture_width_d: 1,
        clean_aperture_height_n: 1080,
        clean_aperture_height_d: 1,
        horiz_off_n: 0,
        horiz_off_d: 1,
        vert_off_n: 0,
        vert_off_d: 1,
    };

    assert_eq!(clap.clean_aperture_width_n, 1920);
    assert_eq!(clap.clean_aperture_height_n, 1080);

    let bytes = clap.to_bytes();
    assert_eq!(bytes.len(), 32);

    // Re-parse
    let clap2 = CleanApertureBox::parse(&bytes).expect("clap re-parse");
    assert_eq!(clap2.clean_aperture_width_n, 1920);
    assert_eq!(clap2.clean_aperture_height_n, 1080);
    assert_eq!(clap2.horiz_off_n, 0);
    assert_eq!(clap2.vert_off_n, 0);

    // Byte-identical round-trip
    assert_eq!(clap2.to_bytes(), bytes);

    // Mutate → bytes change
    let clap3 = CleanApertureBox {
        clean_aperture_width_n: 1280,
        ..clap
    };
    assert_ne!(clap3.to_bytes(), bytes, "mutation must change bytes");
}
