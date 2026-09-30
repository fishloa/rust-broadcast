//! `srat` / `AudioSampleEntryV1` carriage for sampling rates above 65535 Hz
//! (audit r05-W31).
//!
//! An `AudioSampleEntry`'s `samplerate` field is 16.16 fixed point, so its
//! integer part tops out at 65535 Hz; `192000 << 16` is truncated to
//! `0xEE000000` and reads back as 60928 Hz. ISO/IEC 14496-12:2015 §12.2.3.2
//! (as amended by Amd 1:2017) defines the `AudioSampleEntryV1` form for exactly
//! this case — `entry_version = 1`, the real rate in a `SamplingRateBox`
//! (`srat`, §12.2.3.1), inside an `stsd` whose version is 1.
//!
//! Fixtures and their generator commands are described in
//! `tests/fixtures/audio_srat/README.md`.
//!
//! The strongest test here is
//! [`our_muxed_192k_output_is_read_back_by_external_tools`]: it muxes our own
//! output, writes it to disk, and has **ffprobe** and **MP4Box** read it — an
//! external tool parsing our builder's bytes, not our parser agreeing with our
//! writer.

use std::path::{Path, PathBuf};
use std::process::Command;

use broadcast_common::{Package, Parse, Serialize, Unpackage};
use transmux::init_segment::{
    AudioSampleEntryV1, FlacSampleEntry, Mp4aSampleEntry, SampleDescriptionBox, SampleEntryVariant,
    SamplingRateBox,
};
use transmux::ir::CodecConfig;
use transmux::{FlacSpecificBox, ProgressiveDemux, TrackSpec};

// ── Fixture plumbing ────────────────────────────────────────────────────────

fn fixture(rel: &str) -> Vec<u8> {
    let path = format!(
        "{}/tests/fixtures/audio_srat/{rel}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// A fixture kept at the workspace root (`fixtures/`), not under `transmux/`.
fn repo_fixture(rel: &str) -> Vec<u8> {
    let path = format!("{}/../fixtures/{rel}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// The box's first byte offset (four-CC position minus the 4-byte size).
fn box_start(bytes: &[u8], four_cc: &[u8; 4]) -> usize {
    bytes
        .windows(4)
        .position(|w| w == four_cc.as_slice())
        .expect("box four-CC present")
        - 4
}

/// A real `dfLa` body from a committed FLAC fixture, so the mux tests exercise
/// a genuine config record rather than inline hand-made bytes.
fn real_flac_config() -> FlacSpecificBox {
    let data = repo_fixture("mp4/flac.mp4");
    let at = box_start(&data, b"dfLa");
    let size = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
    FlacSpecificBox::parse(&data[at + 8..at + size]).expect("parse dfLa")
}

/// Mux `rate`-Hz FLAC to fragmented MP4 bytes (init + one media segment).
fn mux_one(rate: u32) -> Vec<u8> {
    let track = transmux::Track::new(
        TrackSpec::new(
            1,
            rate,
            CodecConfig::Flac {
                config: real_flac_config(),
                channel_count: 1,
                sample_rate: rate,
                sample_size: 16,
            },
        ),
        vec![transmux::Sample::new(
            vec![0u8; 16],
            Some(0),
            Some(0),
            Some(1024),
            true,
        )],
    );
    transmux::CmafMux::new(1)
        .package(&transmux::Media::new(vec![track], 1000))
        .expect("mux flac")
}

/// Demux a real ffmpeg-written FLAC fixture and retarget the IR to the
/// decoder's true rate.
///
/// ffmpeg's FLAC muxer does not emit a `srat` even at 192 kHz, so the entry it
/// writes wraps to a wrong rate and the demuxed IR carries that wrapped value.
/// The fixture's `fLaC` STREAMINFO carries the true rate — which the
/// external-tool test confirms with ffprobe — so the mux tests set it here;
/// the samples are the fixture's real coded bytes.
fn retargeted_flac_source(name: &str, rate: u32) -> transmux::Media {
    let mut media = ProgressiveDemux::new(16 * 1024 * 1024)
        .expect("demuxer")
        .unpackage(&fixture(name))
        .expect("demux the ffmpeg-made FLAC fixture");
    assert_eq!(media.tracks.len(), 1, "one audio track in {name}");
    assert_eq!(media.skipped.len(), 0, "nothing skipped in {name}");
    media.tracks[0].spec.timescale = rate;
    match &mut media.tracks[0].spec.config {
        CodecConfig::Flac { sample_rate, .. } => *sample_rate = rate,
        _ => panic!("expected FLAC in {name}"),
    }
    media
}

/// Mux one of the retargeted sources with our own muxer.
fn mux_retargeted(name: &str, rate: u32) -> Vec<u8> {
    transmux::CmafMux::new(1)
        .package(&retargeted_flac_source(name, rate))
        .expect("mux the retargeted FLAC source")
}

// ── External-tool oracle ────────────────────────────────────────────────────

/// A unique temp file path (no `tempfile` dependency in this crate's dev-deps).
fn temp_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("transmux-srat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.join(name)
}

fn tool_available(tool: &str) -> bool {
    Command::new(tool)
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// `ffprobe -show_entries stream=sample_rate` for the first audio stream.
fn ffprobe_sample_rate(path: &Path) -> String {
    let out = Command::new("ffprobe")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=sample_rate",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .expect("run ffprobe");
    assert!(out.status.success(), "ffprobe failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// `MP4Box -info` output, which prints "Sample Rate N" for an audio track.
fn mp4box_info(path: &Path) -> String {
    let out = Command::new("MP4Box")
        .arg("-info")
        .arg(path)
        .output()
        .expect("run MP4Box");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The decoded PCM md5 ffmpeg computes for a file (lossless codecs only).
fn decoded_md5(path: &Path) -> String {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-f", "md5", "-"])
        .output()
        .expect("run ffmpeg");
    assert!(out.status.success(), "ffmpeg failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The builder's **own** 192 kHz output, written to disk, is read correctly by
/// two external tools, and its decoded PCM is md5-identical to the source.
///
/// This is the oracle the crate cannot provide for itself: ffprobe and MP4Box
/// parse the bytes our writer produced, and ffmpeg decodes them.
#[test]
fn our_muxed_192k_output_is_read_back_by_external_tools() {
    if !tool_available("ffprobe") || !tool_available("ffmpeg") {
        eprintln!(
            "SKIP: ffprobe/ffmpeg not on PATH — cannot run the external-tool oracle \
             (install ffmpeg and re-run)"
        );
        return;
    }

    let source_path = temp_path("source-192k.mp4");
    std::fs::write(&source_path, fixture("flac_192000.mp4")).expect("write source");
    let ours_path = temp_path("ours-192k.mp4");
    std::fs::write(&ours_path, mux_retargeted("flac_192000.mp4", 192_000)).expect("write ours");

    // MP4Box reads the sample entry itself, so it is the tool that actually
    // discriminates the v1+srat form from the wrapping v0 form: on the v0
    // entry it reports 60928, on ours 192000. It is therefore *required* here
    // rather than best-effort.
    assert!(
        tool_available("MP4Box"),
        "MP4Box is not on PATH — it is the only local tool that reads the sample          entry's own rate (ffprobe prefers the FLAC STREAMINFO), so this oracle          cannot run"
    );
    let info = mp4box_info(&ours_path);
    // Assert on MP4Box's *sample-entry* line, not the `TimeScale` line: the
    // mdhd timescale is 192000 either way, so only the "Sample Rate" value
    // distinguishes the v1+srat entry from the wrapping v0 one.
    assert!(
        info.contains("Sample Rate 192000"),
        "MP4Box must read our muxer's sample entry at 192000 Hz, got:
{info}"
    );
    assert!(
        !info.contains("Sample Rate 60928"),
        "MP4Box must not read the wrapped 16.16 rate, got:
{info}"
    );
    assert_eq!(ffprobe_sample_rate(&ours_path), "192000");

    // The decoded PCM is identical: FLAC is lossless, so a mismatched rate or a
    // corrupted sample entry would change this.
    assert_eq!(
        decoded_md5(&ours_path),
        decoded_md5(&source_path),
        "decoded PCM must be md5-identical to the ffmpeg source"
    );
}

/// The 96 kHz case, which wraps to 30464 Hz in the 16.16 field.
#[test]
fn our_muxed_96k_output_is_read_back_by_external_tools() {
    if !tool_available("ffprobe") || !tool_available("ffmpeg") {
        eprintln!("SKIP: ffprobe/ffmpeg not on PATH — cannot run the external-tool oracle");
        return;
    }

    let source_path = temp_path("source-96k.mp4");
    std::fs::write(&source_path, fixture("flac_96000.mp4")).expect("write source");
    let ours_path = temp_path("ours-96k.mp4");
    std::fs::write(&ours_path, mux_retargeted("flac_96000.mp4", 96_000)).expect("write ours");

    assert_eq!(ffprobe_sample_rate(&ours_path), "96000");
    assert_eq!(decoded_md5(&ours_path), decoded_md5(&source_path));
}

/// 44.1 kHz is the control: it fits the 16.16 field, so the entry stays v0 and
/// the output is still read correctly by the external tools.
#[test]
fn our_muxed_44k_output_stays_v0_and_reads_back_correctly() {
    if !tool_available("ffprobe") || !tool_available("ffmpeg") {
        eprintln!("SKIP: ffprobe/ffmpeg not on PATH — cannot run the external-tool oracle");
        return;
    }

    let source_path = temp_path("source-44k.mp4");
    std::fs::write(&source_path, fixture("flac_44100.mp4")).expect("write source");
    let ours = mux_retargeted("flac_44100.mp4", 44_100);
    let ours_path = temp_path("ours-44k.mp4");
    std::fs::write(&ours_path, &ours).expect("write ours");

    assert_eq!(ffprobe_sample_rate(&ours_path), "44100");
    assert_eq!(decoded_md5(&ours_path), decoded_md5(&source_path));

    // And it really did stay the plain form: stsd v0, no srat anywhere.
    let stsd_at = box_start(&ours, b"stsd");
    let stsd = SampleDescriptionBox::parse(&ours[stsd_at..]).expect("parse stsd");
    assert_eq!(stsd.version, 0, "44.1 kHz fits the 16.16 field");
    let SampleEntryVariant::Flac(flac) = stsd.entries.first().expect("entry") else {
        panic!("expected fLaC");
    };
    assert_eq!(flac.entry_version, 0);
    assert_eq!(flac.samplerate, 44_100 << 16);
    assert!(
        ours.windows(4).all(|w| w != b"srat"),
        "a fitting rate needs no srat"
    );
}

// ── The real fixture pair ───────────────────────────────────────────────────

/// The ffmpeg-written 192 kHz fixture is the v1 form: `stsd` version 1,
/// `entry_version` 1, and an `srat` child holding the true 192000.
///
/// ffmpeg keeps the 16.16 field at its own (wrapped) 48000 rather than the
/// spec's `1 << 16` placeholder, so the load-bearing assertion is that `srat`
/// carries 192000 — and that the 16.16 field alone does not.
#[test]
fn real_fixture_entry_is_v1_with_srat_192000() {
    let bytes = fixture("ipcm_v1_192k.mp4");
    let stsd_at = box_start(&bytes, b"stsd");
    let stsd = SampleDescriptionBox::parse(&bytes[stsd_at..]).expect("parse stsd");
    assert_eq!(stsd.version, AudioSampleEntryV1::STSD_VERSION);

    let stsd_size = u32::from_be_bytes(bytes[stsd_at..stsd_at + 4].try_into().unwrap()) as usize;
    let entry = &bytes[stsd_at + 16..stsd_at + stsd_size];
    assert_eq!(&entry[4..8], b"ipcm");
    assert_eq!(
        u16::from_be_bytes(entry[16..18].try_into().unwrap()),
        AudioSampleEntryV1::ENTRY_VERSION
    );
    let srat = SamplingRateBox::parse(&entry[box_start(entry, b"srat")..]).expect("srat");
    assert_eq!(srat.sampling_rate, 192_000);
    // The literal 16.16 field ffmpeg wrote, and what it decodes to.
    assert_eq!(
        u32::from_be_bytes(entry[32..36].try_into().unwrap()),
        0xBB80_0000
    );
    assert_eq!(
        u32::from_be_bytes(entry[32..36].try_into().unwrap()) >> 16,
        48_000
    );
}

/// The wrapped v0 fixture loses the rate: `entry_version` 0, no `srat`, and the
/// 16.16 field truncating `192000 << 16` to 60928 Hz.
#[test]
fn wrapped_v0_fixture_loses_the_rate_to_the_16_bit_field() {
    let bytes = fixture("ipcm_v0_192k_wrapped.mp4");
    let stsd_at = box_start(&bytes, b"stsd");
    let stsd = SampleDescriptionBox::parse(&bytes[stsd_at..]).expect("parse stsd");
    assert_eq!(stsd.version, 0);
    let stsd_size = u32::from_be_bytes(bytes[stsd_at..stsd_at + 4].try_into().unwrap()) as usize;
    let entry = &bytes[stsd_at + 16..stsd_at + stsd_size];
    assert_eq!(u16::from_be_bytes(entry[16..18].try_into().unwrap()), 0);
    assert!(entry.windows(4).all(|w| w != b"srat"));
    assert_eq!(
        u32::from_be_bytes(entry[32..36].try_into().unwrap()),
        0xEE00_0000
    );
    assert_eq!(
        u32::from_be_bytes(entry[32..36].try_into().unwrap()) >> 16,
        60_928
    );
}

// ── QuickTime sound descriptions round-trip verbatim ────────────────────────

/// A real QuickTime **version 1** sound description carries a non-zero
/// `compression_ID` (QuickTime's VBR marker `-2`) that the crate used to
/// discard on re-serialize.
#[test]
fn quicktime_v1_fixture_has_a_non_zero_compression_id() {
    let bytes = fixture("qt_alac_v1.mov");
    let stsd_at = box_start(&bytes, b"stsd");
    let stsd_size = u32::from_be_bytes(bytes[stsd_at..stsd_at + 4].try_into().unwrap()) as usize;
    let entry = &bytes[stsd_at + 16..stsd_at + stsd_size];
    assert_eq!(&entry[4..8], b"alac");
    assert_eq!(u16::from_be_bytes(entry[16..18].try_into().unwrap()), 1);
    assert_eq!(
        &entry[28..32],
        &[0xFF, 0xFE, 0x00, 0x00],
        "QuickTime compression_ID = -2 (VBR) + packet_size"
    );
    // A typed entry round-trips those bytes instead of zeroing them.
    let typed = FlacSampleEntry {
        entry_version: 1,
        reserved_1: entry[18..24].try_into().unwrap(),
        data_reference_index: 1,
        channelcount: 1,
        samplesize: 16,
        compression_id_and_packet_size: [0xFF, 0xFE, 0x00, 0x00],
        samplerate: 44_100 << 16,
        config_boxes: vec![],
    };
    let mut buf = vec![0u8; typed.serialized_len()];
    typed.serialize_into(&mut buf).expect("serialize");
    // The fixed-field region is byte-identical to the real QuickTime file
    // (offsets 16..36: entry_version, vendor, channelcount, samplesize,
    // compression_ID/packet_size, samplerate).
    assert_eq!(&buf[16..36], &entry[16..36]);
    assert_eq!(FlacSampleEntry::parse(&buf).expect("re-parse"), typed);
}

/// A synthetic QuickTime **version 2** entry (`entry_version = 2`) with
/// non-zero `revision_level`/`vendor` bytes round-trips through the typed
/// entry: the version is no longer re-derived as 0-or-1 on serialize.
#[test]
fn quicktime_v2_entry_version_round_trips_verbatim() {
    let bytes = fixture("qt_alac_v2_synthetic.mov");
    let stsd_at = box_start(&bytes, b"stsd");
    let stsd_size = u32::from_be_bytes(bytes[stsd_at..stsd_at + 4].try_into().unwrap()) as usize;
    let entry = &bytes[stsd_at + 16..stsd_at + stsd_size];
    assert_eq!(u16::from_be_bytes(entry[16..18].try_into().unwrap()), 2);

    let typed = FlacSampleEntry {
        entry_version: 2,
        reserved_1: [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF],
        data_reference_index: 1,
        channelcount: 1,
        samplesize: 16,
        compression_id_and_packet_size: [0xFF, 0xFE, 0x00, 0x00],
        samplerate: 44_100 << 16,
        config_boxes: vec![],
    };
    let mut buf = vec![0u8; typed.serialized_len()];
    let n = typed.serialize_into(&mut buf).expect("serialize");
    buf.truncate(n);

    let back = FlacSampleEntry::parse(&buf).expect("re-parse");
    assert_eq!(back, typed, "every fixed field must round-trip verbatim");
    assert_eq!(back.entry_version, 2, "not re-derived to 0 or 1");
    assert_eq!(
        &buf[18..24],
        &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF],
        "revision_level/vendor preserved in the bytes"
    );
    assert_eq!(
        &buf[28..32],
        &[0xFF, 0xFE, 0x00, 0x00],
        "compression_ID/packet_size preserved in the bytes"
    );
}

// ── The mux path ────────────────────────────────────────────────────────────

/// A 192 kHz FLAC track builds an init segment whose `stsd` is version 1, whose
/// entry carries `entry_version` 1 and the spec's placeholder rate, and whose
/// config children include an `srat` of 192000.
#[test]
fn built_init_segment_emits_v1_and_srat_for_192k() {
    let spec = TrackSpec::new(
        1,
        192_000,
        CodecConfig::Flac {
            config: real_flac_config(),
            channel_count: 1,
            sample_rate: 192_000,
            sample_size: 16,
        },
    );
    let init = transmux::build_init_segment(&[spec], 1000).expect("build init");
    let stsd_at = box_start(&init, b"stsd");
    let stsd = SampleDescriptionBox::parse(&init[stsd_at..]).expect("parse stsd");
    assert_eq!(stsd.version, AudioSampleEntryV1::STSD_VERSION);
    let entry = stsd.entries.first().expect("one entry");
    assert_eq!(
        entry.required_stsd_version(),
        AudioSampleEntryV1::STSD_VERSION
    );
    let SampleEntryVariant::Flac(flac) = entry else {
        panic!("expected a fLaC entry, got {entry:?}");
    };
    assert_eq!(flac.entry_version, AudioSampleEntryV1::ENTRY_VERSION);
    assert_eq!(flac.samplerate, AudioSampleEntryV1::SAMPLERATE_PLACEHOLDER);
    let srat_at = box_start(&init, b"srat");
    let srat = SamplingRateBox::parse(&init[srat_at..]).expect("srat");
    assert_eq!(srat.sampling_rate, 192_000);

    // And the entry itself round-trips, `srat` and all.
    let mut buf = vec![0u8; flac.serialized_len()];
    let n = flac.serialize_into(&mut buf).expect("re-serialize");
    buf.truncate(n);
    let back = FlacSampleEntry::parse(&buf).expect("re-parse");
    assert_eq!(
        &back,
        flac.as_ref(),
        "the entry must round-trip byte-exactly"
    );
    assert_eq!(back.samplerate, AudioSampleEntryV1::SAMPLERATE_PLACEHOLDER);
}

/// A rate that fits the field stays v0: no `srat`, no version bump.
#[test]
fn built_init_segment_keeps_v0_for_a_fitting_rate() {
    let spec = TrackSpec::new(
        1,
        48_000,
        CodecConfig::Flac {
            config: real_flac_config(),
            channel_count: 1,
            sample_rate: 48_000,
            sample_size: 16,
        },
    );
    let init = transmux::build_init_segment(&[spec], 1000).expect("build init");
    let stsd_at = box_start(&init, b"stsd");
    let stsd = SampleDescriptionBox::parse(&init[stsd_at..]).expect("parse stsd");
    assert_eq!(stsd.version, 0);
    let SampleEntryVariant::Flac(flac) = stsd.entries.first().expect("entry") else {
        panic!("expected fLaC");
    };
    assert_eq!(flac.entry_version, 0);
    assert_eq!(flac.samplerate, 48_000 << 16);
    assert!(init.windows(4).all(|w| w != b"srat"));
}

/// The v1 threshold is exactly the 16-bit field width.
#[test]
fn v1_threshold_is_the_16_bit_field_width() {
    assert!(AudioSampleEntryV1::rate_fits_v0(44_100));
    assert!(AudioSampleEntryV1::rate_fits_v0(65_535));
    assert!(!AudioSampleEntryV1::rate_fits_v0(65_536));
    assert!(!AudioSampleEntryV1::rate_fits_v0(88_200));
    assert!(!AudioSampleEntryV1::rate_fits_v0(96_000));
    assert!(!AudioSampleEntryV1::rate_fits_v0(176_400));
    assert!(!AudioSampleEntryV1::rate_fits_v0(192_000));
}

// ── The box itself ──────────────────────────────────────────────────────────

/// `srat` is the 16-byte FullBox §12.2.3.1 defines, and round-trips.
#[test]
fn srat_round_trips_byte_exact() {
    let box_ = SamplingRateBox::new(192_000);
    assert_eq!(box_.serialized_len(), 16);
    let mut buf = vec![0u8; box_.serialized_len()];
    assert_eq!(box_.serialize_into(&mut buf).expect("serialize"), 16);
    assert_eq!(
        buf,
        [
            0, 0, 0, 16, b's', b'r', b'a', b't', 0, 0, 0, 0, 0x00, 0x02, 0xEE, 0x00
        ]
    );
    assert_eq!(SamplingRateBox::parse(&buf).expect("parse"), box_);
}

/// A truncated `srat` is refused, never sliced past its end.
#[test]
fn truncated_srat_is_rejected() {
    let short = [0u8, 0, 0, 16, b's', b'r', b'a', b't', 0, 0, 0];
    assert!(SamplingRateBox::parse(&short).is_err());
    // Every shorter prefix too, down to empty.
    let full = {
        let b = SamplingRateBox::new(192_000);
        let mut v = vec![0u8; b.serialized_len()];
        b.serialize_into(&mut v).unwrap();
        v
    };
    for n in 0..full.len() {
        assert!(
            SamplingRateBox::parse(&full[..n]).is_err(),
            "a {n}-byte prefix must be rejected"
        );
    }
}

/// The parser checks the four-CC: bytes of any other box are refused rather
/// than read as a rate.
#[test]
fn srat_parser_rejects_a_foreign_four_cc() {
    let mut buf = vec![0u8; 16];
    buf[0..4].copy_from_slice(&16u32.to_be_bytes());
    buf[4..8].copy_from_slice(b"free");
    buf[12..16].copy_from_slice(&192_000u32.to_be_bytes());
    assert!(SamplingRateBox::parse(&buf).is_err());
    // The same bytes with the right four-CC do parse.
    buf[4..8].copy_from_slice(b"srat");
    assert_eq!(
        SamplingRateBox::parse(&buf).expect("srat").sampling_rate,
        192_000
    );
}

/// A malformed or zero `srat` is ignored, leaving the entry's own 16.16 field
/// in force — never replacing a good rate with 0.
#[test]
fn a_zero_or_malformed_srat_falls_back_to_the_16_16_field() {
    use transmux::init_segment::sampling_rate_override;

    // A zero-rate `srat`: ignored.
    let zero = {
        let b = SamplingRateBox::new(0);
        let mut v = vec![0u8; b.serialized_len()];
        b.serialize_into(&mut v).unwrap();
        transmux::init_segment::OpaqueBox::new(*b"srat", v[8..].to_vec())
    };
    assert_eq!(sampling_rate_override(&[zero]), None);

    // A truncated `srat` body: ignored, not sliced.
    let short = transmux::init_segment::OpaqueBox::new(*b"srat", vec![0u8; 3]);
    assert_eq!(sampling_rate_override(&[short]), None);

    // A well-formed one is honoured.
    let good = {
        let b = SamplingRateBox::new(192_000);
        let mut v = vec![0u8; b.serialized_len()];
        b.serialize_into(&mut v).unwrap();
        transmux::init_segment::OpaqueBox::new(*b"srat", v[8..].to_vec())
    };
    assert_eq!(sampling_rate_override(&[good]), Some(192_000));

    // And the entry's own field still stands when the override is ignored:
    // a 48 kHz 16.16 field with a zero `srat` reads back as 48000, not 0.
    let entry = Mp4aSampleEntry {
        codec_type: *b"mp4a",
        entry_version: 1,
        reserved_1: [0u8; 6],
        data_reference_index: 1,
        channelcount: 2,
        samplesize: 16,
        compression_id_and_packet_size: [0u8; 4],
        samplerate: 48_000 << 16,
        config_boxes: vec![transmux::init_segment::OpaqueBox::new(
            *b"srat",
            vec![0, 0, 0, 0, 0, 0, 0, 0],
        )],
    };
    let mut buf = vec![0u8; entry.serialized_len()];
    entry.serialize_into(&mut buf).unwrap();
    let back = Mp4aSampleEntry::parse(&buf).expect("parse");
    assert_eq!(sampling_rate_override(&back.config_boxes), None);
    assert_eq!(back.samplerate >> 16, 48_000);
}

/// An `mp4a` entry round-trips at a fitting rate and writes the v0 reserved
/// bytes as zeros (no stray `entry_version`).
#[test]
fn mp4a_v0_entry_round_trips() {
    let entry = Mp4aSampleEntry {
        codec_type: *b"mp4a",
        entry_version: 0,
        reserved_1: [0u8; 6],
        data_reference_index: 1,
        channelcount: 2,
        samplesize: 16,
        compression_id_and_packet_size: [0u8; 4],
        samplerate: 48_000 << 16,
        config_boxes: vec![],
    };
    assert_eq!(entry.serialized_len(), 36);
    let mut buf = vec![0u8; entry.serialized_len()];
    entry.serialize_into(&mut buf).expect("serialize");
    assert_eq!(&buf[8..14], &[0u8; 6], "SampleEntry reserved");
    assert_eq!(&buf[16..24], &[0u8; 8], "v0 entry_version + reserved");
    assert_eq!(&buf[28..32], &[0u8; 4], "pre_defined + reserved");
    assert_eq!(buf[32..36], (48_000u32 << 16).to_be_bytes());
    assert_eq!(Mp4aSampleEntry::parse(&buf).expect("parse"), entry);
}

/// The batch mux path hits the v1 branch end-to-end (not just `build_trak`):
/// a demuxed-then-muxed 192 kHz track is read back by our own demuxer at the
/// true rate.
#[test]
fn muxed_192k_track_demuxes_back_to_the_true_rate() {
    let ours = mux_one(192_000);
    let demuxed = ProgressiveDemux::new(16 * 1024 * 1024)
        .expect("demuxer")
        .unpackage(&ours)
        .expect("demux our own 192k output");
    let rate = demuxed.tracks.iter().find_map(|t| match &t.spec.config {
        CodecConfig::Flac { sample_rate, .. } => Some(*sample_rate),
        _ => None,
    });
    assert_eq!(rate, Some(192_000));
}

/// The same round trip at a fitting rate is unchanged.
#[test]
fn muxed_48k_track_demuxes_back_to_48000() {
    let ours = mux_one(48_000);
    let demuxed = ProgressiveDemux::new(16 * 1024 * 1024)
        .expect("demuxer")
        .unpackage(&ours)
        .expect("demux our own 48k output");
    let rate = demuxed.tracks.iter().find_map(|t| match &t.spec.config {
        CodecConfig::Flac { sample_rate, .. } => Some(*sample_rate),
        _ => None,
    });
    assert_eq!(rate, Some(48_000));
}
