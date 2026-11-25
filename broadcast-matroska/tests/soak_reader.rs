#![cfg(feature = "std")]
//! Soak: reading a generated 4 GiB Matroska file holds only one Block header
//! plus small constants, never the file (streaming-first Phase 5, slice 3).
//!
//! Gated by `RUST_BROADCAST_SOAK=1` (the nightly soak env); without it the test
//! prints why it skipped. `RUST_BROADCAST_SOAK_BYTES` shrinks the generated
//! file for a quick local run; the default is the 4 GiB the gate names.

use std::fs::File;
use std::io::{Seek, SeekFrom, Write as _};
use std::time::Duration;

use broadcast_matroska::Limits;
use broadcast_matroska::driver::{DriverStep, FileReader};
use broadcast_matroska::reader::Mode;
use broadcast_matroska::writer::{
    BlockFlags, InfoSpec, Layout, TrackSpec, Write, Writer, info_element, tracks_element,
};
use test_alloc::ThreadCapped;

/// Live-byte budget: enough for generating one 1 MiB frame and one cluster,
/// then measuring the reader alone. The reader itself must stay far below it.
const BUDGET: usize = 16 * 1024 * 1024;

#[global_allocator]
static ALLOC: ThreadCapped<BUDGET> = ThreadCapped::new();

const DEFAULT_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const FRAME_BYTES: usize = 1024 * 1024;
/// Bytes the reader may hold live after the generation buffers are dropped:
/// one 64 KiB Block-header prefetch, one Cluster index entry, the track table
/// and the harness's own small allocations.
const READ_BOUND: usize = 1024 * 1024;

#[test]
fn reading_four_gibibytes_holds_one_block_header() {
    if std::env::var_os("RUST_BROADCAST_SOAK").as_deref() != Some(std::ffi::OsStr::new("1")) {
        eprintln!("SKIPPED soak_reader: set RUST_BROADCAST_SOAK=1 to run the 4 GiB soak");
        return;
    }
    let target = std::env::var_os("RUST_BROADCAST_SOAK_BYTES")
        .and_then(|value| value.to_str().and_then(|text| text.parse::<u64>().ok()))
        .unwrap_or(DEFAULT_BYTES);
    let dir = env!("CARGO_TARGET_TMPDIR");
    let path = format!("{dir}/soak-reader.mkv");
    generate(&path, target);
    ThreadCapped::<BUDGET>::reset_peak();
    let file = File::open(&path).expect("generated file opens");
    let mut reader = FileReader::new(file, Mode::Strict, Limits::new()).expect("driver");
    let mut blocks = 0u64;
    loop {
        match reader.step().expect("step") {
            DriverStep::Block(_) => blocks += 1,
            DriverStep::Done => break,
            DriverStep::Warning(warning) => panic!("warning {warning}"),
            _ => {}
        }
    }
    let peak = ThreadCapped::<BUDGET>::peak();
    println!("soak_reader: {target} bytes, {blocks} blocks, peak {peak} bytes, bound {READ_BOUND}");
    assert!(peak <= READ_BOUND, "peak {peak} exceeds {READ_BOUND}");
    let _ = std::fs::remove_file(&path);
}

fn generate(path: &str, target: u64) {
    let info = info_element(&InfoSpec::new(1_000_000, 0.0)).expect("info");
    let specs = [TrackSpec {
        track_number: 1,
        track_uid: 1,
        track_type: 1,
        codec_id: "V_VP9",
        codec_private: None,
        default_duration_ns: None,
        video: Some((16, 16)),
        audio: None,
    }];
    let tracks = tracks_element(&specs).expect("tracks");
    let mut writer =
        Writer::new(Layout::new(Duration::from_millis(1)), &info, &tracks).expect("writer");
    let frame = vec![0u8; FRAME_BYTES];
    let mut file = File::create(path).expect("create");
    let mut end = 0u64;
    let blocks = target / (FRAME_BYTES as u64);
    for index in 0..blocks {
        let writes = writer
            .push_block(
                1,
                index as i64 * 10_000,
                None,
                BlockFlags::keyframe(index == 0),
                &frame,
            )
            .expect("push");
        end = apply(writes, &mut file, end);
    }
    end = apply(writer.finish().expect("finish"), &mut file, end);
    file.set_len(end).expect("truncate");
    file.flush().expect("flush");
}

fn apply(writes: Vec<Write>, file: &mut File, mut end: u64) -> u64 {
    for write in writes {
        match write {
            Write::Append(bytes) => {
                file.seek(SeekFrom::Start(end)).expect("seek");
                file.write_all(&bytes).expect("write");
                end += bytes.len() as u64;
            }
            Write::At { offset, bytes } => {
                file.seek(SeekFrom::Start(offset)).expect("seek");
                file.write_all(&bytes).expect("write");
                file.seek(SeekFrom::End(0)).expect("seek end");
            }
            _ => {}
        }
    }
    end
}
