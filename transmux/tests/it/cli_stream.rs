//! The CLI's streaming core over every input × output combination.
//!
//! Each input container the CLI accepts (TS, fragmented MP4, progressive MP4,
//! FLV, MPEG-PS, WebM/Matroska) is driven through `run_bytes` — the same
//! streaming path the binary uses — into every output format. Where a
//! combination is impossible (a codec a container cannot carry) the test
//! asserts the typed error rather than skipping.
#![cfg(feature = "cli")]

use transmux::cli::{CliError, Opts, Output, OutputFormat, run_bytes};
use transmux::error::Error;

/// Every output format the CLI writes.
const FORMATS: [OutputFormat; 6] = [
    OutputFormat::Cmaf,
    OutputFormat::Hls,
    OutputFormat::TsHls,
    OutputFormat::Dash,
    OutputFormat::Ts,
    OutputFormat::Progressive,
];

fn read(fixture: &str) -> Vec<u8> {
    std::fs::read(test_fixtures::path(fixture))
        .unwrap_or_else(|error| panic!("read fixture {fixture}: {error}"))
}

fn opts(format: OutputFormat) -> Opts {
    Opts {
        format,
        segment_duration: 1,
        ..Opts::default()
    }
}

/// Every format accepts this input and produces an artifact of the expected
/// shape.
fn expect_all(input: &[u8], fixture: &str) {
    for format in FORMATS {
        let output = run_bytes(input, &opts(format))
            .unwrap_or_else(|error| panic!("{fixture} -> {format}: {error}"));
        match output {
            Output::Bytes(bytes) => assert!(!bytes.is_empty(), "{fixture} -> {format}: no bytes"),
            Output::Manifest { text, segments } => {
                assert!(!text.is_empty(), "{fixture} -> {format}: no manifest");
                assert!(!segments.is_empty(), "{fixture} -> {format}: no segments");
            }
            _ => panic!("{fixture} -> {format}: unexpected output"),
        }
    }
}

#[test]
fn ts_feeds_every_output() {
    expect_all(&read("ts/h264-aac.ts"), "ts/h264-aac.ts");
}

#[test]
fn fragmented_mp4_feeds_every_output() {
    expect_all(
        &read("mp4/frag/h264-high.frag.mp4"),
        "mp4/frag/h264-high.frag.mp4",
    );
}

#[test]
fn progressive_mp4_feeds_every_output() {
    expect_all(&read("mp4/h264-aac-prog.mp4"), "mp4/h264-aac-prog.mp4");
}

#[test]
fn flv_feeds_every_output() {
    expect_all(&read("flv/av.flv"), "flv/av.flv");
}

#[test]
fn mpeg_ps_feeds_every_output() {
    expect_all(&read("ps/h264-ac3.ps"), "ps/h264-ac3.ps");
}

#[test]
fn matroska_feeds_every_output() {
    expect_all(&read("mkv/h264-aac.mkv"), "mkv/h264-aac.mkv");
}

/// WebM/Matroska carries VP8/VP9/Opus, which the TS layer cannot represent:
/// every TS output must fail with the typed error, never a silent skip. The
/// ISOBMFF outputs support those codecs and must succeed.
#[test]
fn webm_refuses_ts_outputs_with_a_typed_error() {
    let input = read("mkv/vp9-opus.webm");
    for format in [
        OutputFormat::Cmaf,
        OutputFormat::Hls,
        OutputFormat::Dash,
        OutputFormat::Progressive,
    ] {
        assert!(
            run_bytes(&input, &opts(format)).is_ok(),
            "WebM -> {format} must package"
        );
    }
    for format in [OutputFormat::Ts, OutputFormat::TsHls] {
        let error = run_bytes(&input, &opts(format)).expect_err("WebM VP9/Opus has no TS carriage");
        assert!(
            matches!(error, CliError::Transmux(Error::InvalidInput(_))),
            "WebM -> {format} must be a typed InvalidInput, got {error:?}"
        );
    }
}

/// A scratch directory under the workspace `target/` for one binary run.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target/cli-stream-tmp")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_transmux")
}

/// Standard input is a forward-only source: a fragmented output streams to a
/// file.
#[test]
fn stdin_to_fragmented_output_works() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let dir = scratch("stdin-cmaf");
    let out = dir.join("out.cmaf");
    let mut child = Command::new(binary())
        .arg("-")
        .arg("-o")
        .arg(&out)
        .args(["-f", "cmaf"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the transmux binary");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(&read("ts/h264-aac.ts"))
        .expect("write the input");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait for the binary");
    assert!(
        output.status.success(),
        "stdin -> cmaf failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = std::fs::read(&out).expect("the cmaf output");
    assert_eq!(&bytes[4..8], b"ftyp", "stdin -> cmaf opens with ftyp");
}

/// Progressive MP4 is two-pass and needs a replayable input, so standard input
/// is refused with the typed error naming fragmented MP4 as the alternative.
#[test]
fn stdin_to_progressive_is_refused() {
    use std::process::{Command, Stdio};

    let dir = scratch("stdin-progressive");
    let mut child = Command::new(binary())
        .arg("-")
        .arg("-o")
        .arg(dir.join("out.mp4"))
        .args(["-f", "progressive"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the transmux binary");
    // The refusal comes before any read, so the child may exit first; closing
    // stdin is all this test needs to do.
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait for the binary");
    assert!(!output.status.success(), "stdin -> progressive must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("progressive MP4 needs a replayable input"),
        "stdin -> progressive must name the typed error: {stderr}"
    );
}

/// The file path streams too, and emits the same bytes `run_bytes` does for a
/// seekable input.
#[test]
fn file_input_matches_the_bytes_core() {
    let dir = scratch("file-cmaf");
    let input = dir.join("in.ts");
    let out = dir.join("out.cmaf");
    std::fs::write(&input, read("ts/h264-aac.ts")).expect("write the input");
    let status = std::process::Command::new(binary())
        .arg(&input)
        .arg("-o")
        .arg(&out)
        .args(["-f", "cmaf"])
        .status()
        .expect("spawn the transmux binary");
    assert!(status.success(), "file -> cmaf failed");
    let from_disk = std::fs::read(&out).expect("the cmaf output");
    let Output::Bytes(from_core) =
        run_bytes(&read("ts/h264-aac.ts"), &opts(OutputFormat::Cmaf)).expect("run_bytes")
    else {
        panic!("CMAF must be Bytes");
    };
    assert_eq!(
        from_disk, from_core,
        "the file path and the bytes core emit the same CMAF"
    );
}
