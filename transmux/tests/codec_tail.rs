//! Codec-tail gate (issue #467): complete `Fmp4Demux` codec-config
//! reconstruction + `CodecConfig::Hevc`.
//!
//! For each fragmented fixture the demux→remux round-trip is a *self-oracle*:
//! the fixture's own config box is the truth. All assertions are computed
//! against the source bytes (no hardcoded offsets): a box-tree walker extracts
//! the config box body straight from the fixture and it is compared, byte for
//! byte, against the record `Fmp4Demux` reconstructed.
//!
//! EXIT CRITERIA (all bite):
//! 1. Per-codec config round-trip: reconstructed `CodecConfig` variant matches
//!    the fixture, and the serialized config record is byte-identical to the
//!    config box body extracted from the source (hvcC/av1C/vpcC/dOps/dfLa/avcC).
//! 2. Sample fidelity: demux → IR → CmafMux → re-demux keeps the coded sample
//!    bytes byte-identical (same count) for every fixture.
//! 3. Hevc output path: the Hevc IR muxes to an `hvc1` sample entry whose
//!    `hvcC` box is byte-identical to the source.
//! 4. Regression: avc1/mp4a still reconstruct (h264_high → Avc).
//! 5. Enumeration: track count + `CodecConfig` variants via `matches!`.

use std::fs;
use std::path::PathBuf;

use broadcast_common::{Package, Serialize, Unpackage};
use transmux::media::{CmafMux, Fmp4Demux};
use transmux::pipeline::CodecConfig;

fn fixture(name: &str) -> Vec<u8> {
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "..",
        "fixtures",
        "mp4",
        "frag",
        name,
    ]
    .iter()
    .collect();
    fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Box-tree walker — extracts a config box body from a fixture with no
// hardcoded offsets, purely by walking the moov → trak → … → stsd → sample
// entry → config box hierarchy.
// ---------------------------------------------------------------------------

/// Iterate `(fourcc, full_box_bytes)` over the child boxes in `region`.
fn boxes(region: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 8 <= region.len() {
        let size = u32::from_be_bytes([
            region[off],
            region[off + 1],
            region[off + 2],
            region[off + 3],
        ]) as usize;
        if size < 8 || off + size > region.len() {
            break;
        }
        let mut fc = [0u8; 4];
        fc.copy_from_slice(&region[off + 4..off + 8]);
        out.push((fc, &region[off..off + size]));
        off += size;
    }
    out
}

/// Find a child box by FourCC in `region`, returning its full bytes.
fn find<'a>(region: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(region)
        .into_iter()
        .find(|(fc, _)| fc == fourcc)
        .map(|(_, b)| b)
}

/// Descend into a container box body (strip the 8-byte box header).
fn body(b: &[u8]) -> &[u8] {
    &b[8..]
}

/// The stsd entry region (skip its 8-byte box header + version/flags/count).
fn stsd_entries(stsd: &[u8]) -> &[u8] {
    &stsd[16..]
}

/// The config-box region of a sample entry, given the sample-entry box bytes
/// and its fixed-field length (78 for visual, 28 for audio).
fn sample_entry_config_region(entry: &[u8], fixed: usize) -> &[u8] {
    &entry[8 + fixed..]
}

/// Extract, from a fragmented fixture, the FIRST track's config box (full box
/// bytes, header included). `sample_entry_fourcc` selects the entry, `fixed`
/// its fixed-field size, `config_fourcc` the box.
fn extract_config_box(
    file: &[u8],
    sample_entry_fourcc: &[u8; 4],
    fixed: usize,
    config_fourcc: &[u8; 4],
) -> Vec<u8> {
    let moov = find(file, b"moov").expect("moov");
    let trak = find(body(moov), b"trak").expect("trak");
    let mdia = find(body(trak), b"mdia").expect("mdia");
    let minf = find(body(mdia), b"minf").expect("minf");
    let stbl = find(body(minf), b"stbl").expect("stbl");
    let stsd = find(body(stbl), b"stsd").expect("stsd");
    let entry = find(stsd_entries(stsd), sample_entry_fourcc).unwrap_or_else(|| {
        panic!(
            "sample entry {:?}",
            core::str::from_utf8(sample_entry_fourcc)
        )
    });
    let region = sample_entry_config_region(entry, fixed);
    find(region, config_fourcc)
        .unwrap_or_else(|| panic!("config box {:?}", core::str::from_utf8(config_fourcc)))
        .to_vec()
}

/// Config box **body** only (the bytes after the config box's 8-byte header).
fn extract_config_body(
    file: &[u8],
    sample_entry_fourcc: &[u8; 4],
    fixed: usize,
    config_fourcc: &[u8; 4],
) -> Vec<u8> {
    let full = extract_config_box(file, sample_entry_fourcc, fixed, config_fourcc);
    full[8..].to_vec()
}

const VISUAL_FIXED: usize = 78;
const AUDIO_FIXED: usize = 28;

/// Serialize any config record and return only its body (records serialize the
/// bare record, not a boxed form; the box wrappers add an 8-byte header we skip
/// when the serializer emits one).
fn record_body<S: Serialize>(record: &S) -> Vec<u8>
where
    S::Error: core::fmt::Debug,
{
    let mut buf = vec![0u8; record.serialized_len()];
    let n = record.serialize_into(&mut buf).expect("serialize record");
    buf.truncate(n);
    buf
}

// ---------------------------------------------------------------------------
// 1. Per-codec config round-trip + 5. enumeration
// ---------------------------------------------------------------------------

#[test]
fn hevc_config_reconstructs_byte_identical() {
    let file = fixture("hevc_main.frag.mp4");
    // The fixture's HEVC sample entry is `hev1` (in-band-capable); config in hvcC.
    let want = extract_config_box(&file, b"hev1", VISUAL_FIXED, b"hvcC");

    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(&file).expect("unpackage hevc");
    assert_eq!(media.tracks.len(), 1, "hevc fixture has one track");
    let cfg = media.tracks[0].config();
    let CodecConfig::Hevc { config, .. } = cfg else {
        panic!("expected CodecConfig::Hevc, got {cfg:?}");
    };
    // `HEVCConfigurationBox` serializes to the full `hvcC` box (header + record).
    assert_eq!(
        record_body(config),
        want,
        "reconstructed hvcC box must equal the source fixture's hvcC box"
    );
}

#[test]
fn av1_config_reconstructs_byte_identical() {
    let file = fixture("av1.frag.mp4");
    let want = extract_config_body(&file, b"av01", VISUAL_FIXED, b"av1C");
    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(&file).expect("unpackage av1");
    let cfg = media.tracks[0].config();
    let CodecConfig::Av1 { config, .. } = cfg else {
        panic!("expected CodecConfig::Av1, got {cfg:?}");
    };
    assert_eq!(record_body(config), want, "av1C body must match source");
}

#[test]
fn vp9_config_reconstructs_byte_identical() {
    let file = fixture("vp9.frag.mp4");
    let want = extract_config_body(&file, b"vp09", VISUAL_FIXED, b"vpcC");
    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(&file).expect("unpackage vp9");
    let cfg = media.tracks[0].config();
    let CodecConfig::Vp9 { config, .. } = cfg else {
        panic!("expected CodecConfig::Vp9, got {cfg:?}");
    };
    assert_eq!(record_body(config), want, "vpcC body must match source");
}

#[test]
fn opus_config_reconstructs_byte_identical() {
    let file = fixture("opus.frag.mp4");
    let want = extract_config_body(&file, b"Opus", AUDIO_FIXED, b"dOps");
    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(&file).expect("unpackage opus");
    let cfg = media.tracks[0].config();
    let CodecConfig::Opus { config, .. } = cfg else {
        panic!("expected CodecConfig::Opus, got {cfg:?}");
    };
    assert_eq!(record_body(config), want, "dOps body must match source");
}

#[test]
fn flac_config_reconstructs_byte_identical() {
    let file = fixture("flac.frag.mp4");
    let want = extract_config_body(&file, b"fLaC", AUDIO_FIXED, b"dfLa");
    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(&file).expect("unpackage flac");
    let cfg = media.tracks[0].config();
    let CodecConfig::Flac { config, .. } = cfg else {
        panic!("expected CodecConfig::Flac, got {cfg:?}");
    };
    assert_eq!(record_body(config), want, "dfLa body must match source");
}

#[test]
fn avc_config_reconstructs_byte_identical() {
    // Regression: avc1 reconstruction must still work.
    let file = fixture("h264_high.frag.mp4");
    // `AVCConfigurationBox` serializes to the full `avcC` box (header + record).
    let want = extract_config_box(&file, b"avc1", VISUAL_FIXED, b"avcC");
    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(&file).expect("unpackage avc");
    let cfg = media.tracks[0].config();
    let CodecConfig::Avc { config, .. } = cfg else {
        panic!("expected CodecConfig::Avc, got {cfg:?}");
    };
    assert_eq!(record_body(config), want, "avcC box must match source");
}

// ---------------------------------------------------------------------------
// 2. Sample fidelity: demux → IR → CmafMux → re-demux is sample-identical
// ---------------------------------------------------------------------------

fn assert_sample_fidelity(name: &str) {
    let file = fixture(name);
    let mut demux = Fmp4Demux::new();
    let media = demux
        .unpackage(&file)
        .unwrap_or_else(|e| panic!("{name} unpackage: {e:?}"));
    assert!(!media.tracks.is_empty(), "{name} has tracks");
    let first: Vec<Vec<u8>> = media.tracks[0]
        .samples
        .iter()
        .map(|s| s.data.clone())
        .collect();
    assert!(!first.is_empty(), "{name} track 0 has samples");

    let mut mux = CmafMux::new(1);
    let repacked = mux
        .package(&media)
        .unwrap_or_else(|e| panic!("{name} package: {e:?}"));

    let mut demux2 = Fmp4Demux::new();
    let media2 = demux2
        .unpackage(&repacked)
        .unwrap_or_else(|e| panic!("{name} re-unpackage: {e:?}"));

    assert_eq!(
        media.tracks.len(),
        media2.tracks.len(),
        "{name} track count preserved"
    );
    let second: Vec<Vec<u8>> = media2.tracks[0]
        .samples
        .iter()
        .map(|s| s.data.clone())
        .collect();
    assert_eq!(
        first.len(),
        second.len(),
        "{name} track 0 sample count preserved"
    );
    for (i, (a, b)) in first.iter().zip(&second).enumerate() {
        assert_eq!(a, b, "{name} track 0 sample {i} coded bytes preserved");
    }
}

#[test]
fn sample_fidelity_all_codecs() {
    for name in [
        "hevc_main.frag.mp4",
        "av1.frag.mp4",
        "vp9.frag.mp4",
        "opus.frag.mp4",
        "flac.frag.mp4",
        "h264_high.frag.mp4",
    ] {
        assert_sample_fidelity(name);
    }
}

// ---------------------------------------------------------------------------
// 3. Hevc output path: Hevc IR → CmafMux → hvc1 entry with hvcC == source
// ---------------------------------------------------------------------------

#[test]
fn hevc_output_path_emits_hvc1_with_source_hvcc() {
    let file = fixture("hevc_main.frag.mp4");
    let want_hvcc_body = extract_config_body(&file, b"hev1", VISUAL_FIXED, b"hvcC");

    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(&file).expect("unpackage hevc");
    assert!(
        matches!(media.tracks[0].config(), CodecConfig::Hevc { .. }),
        "track 0 is Hevc"
    );

    let mut mux = CmafMux::new(1);
    let out = mux.package(&media).expect("package hevc");

    // The emitted init segment must carry an `hvc1` sample entry (parameter sets
    // in the sample entry) whose `hvcC` body is byte-identical to the source.
    let moov = find(&out, b"moov").expect("moov in output");
    let trak = find(body(moov), b"trak").expect("trak");
    let mdia = find(body(trak), b"mdia").expect("mdia");
    let minf = find(body(mdia), b"minf").expect("minf");
    let stbl = find(body(minf), b"stbl").expect("stbl");
    let stsd = find(body(stbl), b"stsd").expect("stsd");
    let entry = find(stsd_entries(stsd), b"hvc1").expect("output emits an hvc1 sample entry");
    let region = sample_entry_config_region(entry, VISUAL_FIXED);
    let hvcc = find(region, b"hvcC").expect("hvcC in output hvc1 entry");
    assert_eq!(
        body(hvcc).to_vec(),
        want_hvcc_body,
        "output hvcC body must be byte-identical to the source fixture's hvcC"
    );
}

// ---------------------------------------------------------------------------
// 5. Enumeration: right track count + right CodecConfig variant.
// ---------------------------------------------------------------------------

/// A fixture name paired with the `CodecConfig`-variant predicate it must match.
type VariantCase = (&'static str, fn(&CodecConfig) -> bool);

#[test]
fn enumeration_yields_expected_variants() {
    let cases: &[VariantCase] = &[
        ("hevc_main.frag.mp4", |c| {
            matches!(c, CodecConfig::Hevc { .. })
        }),
        ("av1.frag.mp4", |c| matches!(c, CodecConfig::Av1 { .. })),
        ("vp9.frag.mp4", |c| matches!(c, CodecConfig::Vp9 { .. })),
        ("opus.frag.mp4", |c| matches!(c, CodecConfig::Opus { .. })),
        ("flac.frag.mp4", |c| matches!(c, CodecConfig::Flac { .. })),
        ("h264_high.frag.mp4", |c| {
            matches!(c, CodecConfig::Avc { .. })
        }),
    ];
    for (name, is_variant) in cases {
        let file = fixture(name);
        let mut demux = Fmp4Demux::new();
        let media = demux
            .unpackage(&file)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(media.tracks.len(), 1, "{name} exposes one track");
        assert!(
            is_variant(media.tracks[0].config()),
            "{name} track 0 has the expected CodecConfig variant, got {:?}",
            media.tracks[0].config()
        );
    }
}
