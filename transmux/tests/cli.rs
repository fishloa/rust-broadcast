//! `transmux` CLI front-end gate (issue #482).
//!
//! Exercises the CLI core ([`transmux::cli`]) that wires the existing demux
//! spokes through the hub IR into the existing mux spokes. Every test **bites**:
//!
//! 1. **TS → CMAF, validator-gated**: a real `.ts` fixture through `run_bytes`
//!    to CMAF, then split into init + media and run the crate's own
//!    `validate_init_segment` / `validate_media_segment`; assert **zero**
//!    `Severity::Error`. A broken pipeline fails validation.
//! 2. **MP4 → HLS**: a fragmented mp4 through `run_bytes` to HLS; assert a valid
//!    `#EXTM3U` playlist (with `#EXT-X-TARGETDURATION`) and that every referenced
//!    `.m4s` segment validates as a CMAF init+media with no errors.
//! 3. **Autodetect bites**: the detector returns the correct container for TS,
//!    MP4, WebM, and PS fixtures, and `Err` on garbage. A detector that always
//!    picks one fails.
//! 4. **Format selection bites**: the same TS input with `-f ts` vs `-f cmaf`
//!    produces structurally different leading bytes (TS `0x47` vs an ISO-BMFF
//!    box), so a mux that ignored the format flag fails.
//! 5. **clap parsing**: `Args::try_parse_from` accepts representative argv and
//!    the command builds (`--help`/`--version` wire up) without panic.

#![cfg(feature = "cli")]

use std::path::PathBuf;

use transmux::cli::{Args, Container, Opts, Output, OutputFormat, detect_container, run_bytes};
use transmux::{Severity, validate_init_segment, validate_media_segment};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures")
}

fn read(rel: &str) -> Vec<u8> {
    let p = fixtures().join(rel);
    std::fs::read(&p).unwrap_or_else(|e| panic!("read fixture {}: {e}", p.display()))
}

/// Split a self-contained CMAF artifact (`ftyp` + `moov` + `styp` + `moof` +
/// `mdat`) at the first `styp` box into `(init, media)`.
fn split_cmaf(bytes: &[u8]) -> (&[u8], &[u8]) {
    let mut off = 0usize;
    while off + 8 <= bytes.len() {
        let size = u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
            as usize;
        let ty = &bytes[off + 4..off + 8];
        if ty == b"styp" {
            return (&bytes[..off], &bytes[off..]);
        }
        if size < 8 || off + size > bytes.len() {
            break;
        }
        off += size;
    }
    panic!("no styp box found — CMAF media segment missing");
}

fn errors(issues: &[transmux::ConformanceIssue]) -> Vec<String> {
    issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| format!("{}: {}", i.code, i.message))
        .collect()
}

// --------------------------------------------------------------------------
// 1. TS → CMAF, validator-gated
// --------------------------------------------------------------------------

#[test]
fn ts_to_cmaf_validates_clean() {
    let ts = read("ts/h264_aac.ts");
    let opts = Opts {
        format: OutputFormat::Cmaf,
        ..Opts::default()
    };
    let out = run_bytes(&ts, &opts).expect("TS → CMAF must succeed");
    let cmaf = match out {
        Output::Bytes(b) => b,
        _ => panic!("CMAF output must be Bytes"),
    };
    // A real fMP4: leading box is `ftyp`.
    assert_eq!(&cmaf[4..8], b"ftyp", "CMAF must open with an ftyp box");

    let (init, media) = split_cmaf(&cmaf);
    let init_errs = errors(&validate_init_segment(init));
    let media_errs = errors(&validate_media_segment(media));
    assert!(
        init_errs.is_empty(),
        "init segment has validator errors: {init_errs:?}"
    );
    assert!(
        media_errs.is_empty(),
        "media segment has validator errors: {media_errs:?}"
    );
}

// --------------------------------------------------------------------------
// 2. MP4 → HLS
// --------------------------------------------------------------------------

#[test]
fn mp4_to_hls_playlist_and_segments() {
    let mp4 = read("mp4/frag/h264_high.frag.mp4");
    let opts = Opts {
        format: OutputFormat::Hls,
        segment_duration: 4,
        ..Opts::default()
    };
    let out = run_bytes(&mp4, &opts).expect("MP4 → HLS must succeed");
    let (text, segments) = match out {
        Output::Manifest { text, segments } => (text, segments),
        _ => panic!("HLS output must be a Manifest"),
    };
    assert!(
        text.starts_with("#EXTM3U"),
        "playlist must start with #EXTM3U"
    );
    assert!(
        text.contains("#EXT-X-TARGETDURATION"),
        "playlist must carry #EXT-X-TARGETDURATION"
    );
    assert!(
        !segments.is_empty(),
        "playlist must reference at least one segment"
    );
    // Every referenced .m4s segment must be named in the playlist and validate.
    for (name, bytes) in &segments {
        assert!(
            text.contains(name.as_str()),
            "playlist must reference {name}"
        );
        let (init, media) = split_cmaf(bytes);
        assert!(
            errors(&validate_init_segment(init)).is_empty(),
            "segment {name} init has errors"
        );
        assert!(
            errors(&validate_media_segment(media)).is_empty(),
            "segment {name} media has errors"
        );
    }
}

// --------------------------------------------------------------------------
// 3. Autodetect bites
// --------------------------------------------------------------------------

#[test]
fn autodetect_recognises_each_container() {
    assert_eq!(
        detect_container(&read("ts/h264_aac.ts")).unwrap(),
        Container::MpegTs
    );
    assert_eq!(
        detect_container(&read("mp4/frag/h264_high.frag.mp4")).unwrap(),
        Container::Mp4
    );
    assert_eq!(
        detect_container(&read("webm/vp9_opus.webm")).unwrap(),
        Container::WebM
    );
    assert_eq!(
        detect_container(&read("ps/h264_ac3.ps")).unwrap(),
        Container::MpegPs
    );
    // Garbage must not be mistaken for any container.
    assert!(detect_container(&[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x11, 0x22, 0x33, 0x44]).is_err());
    assert!(detect_container(&[]).is_err());
}

// --------------------------------------------------------------------------
// 4. Format selection bites
// --------------------------------------------------------------------------

#[test]
fn format_selection_changes_output_shape() {
    let ts = read("ts/h264_aac.ts");

    let cmaf = match run_bytes(
        &ts,
        &Opts {
            format: OutputFormat::Cmaf,
            ..Opts::default()
        },
    )
    .expect("TS → CMAF")
    {
        Output::Bytes(b) => b,
        _ => panic!("CMAF must be Bytes"),
    };
    let ts_out = match run_bytes(
        &ts,
        &Opts {
            format: OutputFormat::Ts,
            ..Opts::default()
        },
    )
    .expect("TS → TS")
    {
        Output::Bytes(b) => b,
        _ => panic!("TS must be Bytes"),
    };

    // CMAF: first bytes are an ISO-BMFF box (size then `ftyp`), byte 0 is 0x00.
    assert_eq!(&cmaf[4..8], b"ftyp", "CMAF must lead with ftyp box");
    assert_ne!(cmaf[0], 0x47, "CMAF must not start with the TS sync byte");
    // TS: first byte is the 0x47 sync byte, whole 188-byte packets.
    assert_eq!(ts_out[0], 0x47, "TS output must start with the sync byte");
    assert_eq!(
        ts_out.len() % 188,
        0,
        "TS output must be whole 188-byte packets"
    );
    // The two outputs are genuinely different container shapes.
    assert_ne!(
        cmaf[0], ts_out[0],
        "the two formats must differ at the leading byte"
    );
}

// --------------------------------------------------------------------------
// 5. clap parsing
// --------------------------------------------------------------------------

#[test]
fn args_parse_representative_argv() {
    use clap::{CommandFactory, Parser};

    // Positional input + named output + format + segment-duration.
    let a = Args::try_parse_from([
        "transmux",
        "in.ts",
        "-o",
        "out.m3u8",
        "-f",
        "hls",
        "--segment-duration",
        "4",
    ])
    .expect("representative argv must parse");
    assert_eq!(a.output, PathBuf::from("out.m3u8"));
    assert_eq!(a.segment_duration, 4);

    // `-i/--input` form + track selection.
    let b = Args::try_parse_from([
        "transmux", "-i", "in.mp4", "-o", "out.cmaf", "--tracks", "1,2",
    ])
    .expect("-i form must parse");
    assert_eq!(b.tracks, vec![1, 2]);

    // Missing both input and output must be an error, not a panic.
    assert!(Args::try_parse_from(["transmux"]).is_err());

    // The command builds (so --help/--version are wired) without panic.
    Args::command().debug_assert();
}
