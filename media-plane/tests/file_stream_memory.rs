//! The file-playout memory bound (issue #4057): playing a 256 MiB TS/FLV file
//! must not hold memory proportional to the file. A generated capture is used
//! because a quarter-gigabyte fixture cannot be committed; the allocator
//! reports the live-byte peak during the whole drive.

#![cfg(all(feature = "std", target_pointer_width = "64"))]

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

use broadcast_common::{Stage, Timestamp};
use media_plane::file_stream::{
    FileDemand, FileInput, FileStreamLimits, StreamContainer, StreamingFileSession,
};
use media_plane::ingress::{ProgramId, SessionEvent};

#[global_allocator]
static A: test_alloc::ProcessPeak = test_alloc::ProcessPeak::new();

/// The generated capture's size for a normal run.
const FILE_BYTES: u64 = 256 << 20;
/// The generated capture's size under `RUST_BROADCAST_SOAK=1`.
const SOAK_FILE_BYTES: u64 = 4 << 30;
/// Live bytes the session may hold while playing either capture.
const BUDGET: usize = 32 << 20;
/// Bytes read from disk per `FileDemand::Bytes`.
const CHUNK: usize = 64 << 10;
/// Replay passes the drive performs over the generated capture.
const PASSES: u32 = 2;

/// The two memory tests share one process-wide allocator, so they must not
/// overlap: a sibling's allocations would be counted in this test's peak.
static PEAK_LOCK: Mutex<()> = Mutex::new(());

fn target_bytes() -> u64 {
    if std::env::var("RUST_BROADCAST_SOAK").is_ok() {
        SOAK_FILE_BYTES
    } else {
        FILE_BYTES
    }
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "media-plane-file-stream-{}-{tag}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Play `path` for `passes` passes, reading `CHUNK` bytes at a time and
/// dropping every emitted sample immediately. Returns the sample count.
fn drive_from_disk(path: &Path, container: StreamContainer, chunk: usize, passes: u32) -> u64 {
    let mut file = File::open(path).expect("open generated capture");
    let len = file.metadata().expect("stat generated capture").len();
    let mut session = StreamingFileSession::new(
        container,
        ProgramId(0),
        true,
        Some(passes),
        false,
        FileStreamLimits::default(),
    );
    let mut buffer = vec![0u8; chunk];
    let mut offset = 0u64;
    let mut emitted = 0u64;
    let mut eof_sent = false;
    let mut done = false;
    let feeds = usize::try_from(len).unwrap_or(usize::MAX) / chunk.max(1) + 8;
    let budget = feeds * (usize::try_from(passes).unwrap_or(usize::MAX) + 2) * 2;
    for _iteration in 0..budget {
        let demand = session.file_demand();
        let before = (offset, eof_sent, emitted, demand);
        if demand == FileDemand::Wait && offset >= len && !eof_sent {
            session
                .feed(FileInput::Eof, Timestamp::ZERO)
                .expect("end of input");
            eof_sent = true;
        }
        match demand {
            FileDemand::Bytes => {
                if offset >= len {
                    session
                        .feed(FileInput::Eof, Timestamp::ZERO)
                        .expect("end of input");
                    eof_sent = true;
                } else {
                    let want = usize::try_from(len - offset).unwrap_or(chunk).min(chunk);
                    let read = file.read(&mut buffer[..want]).expect("read capture");
                    offset += read as u64;
                    session
                        .feed(FileInput::Bytes(&buffer[..read]), Timestamp::ZERO)
                        .expect("session feed");
                }
            }
            FileDemand::Rewind => {
                file.seek(SeekFrom::Start(0)).expect("rewind capture");
                let want = usize::try_from(len).unwrap_or(chunk).min(chunk);
                let read = file.read(&mut buffer[..want]).expect("read capture");
                offset = read as u64;
                session
                    .feed(FileInput::Bytes(&buffer[..read]), Timestamp::ZERO)
                    .expect("session feed");
            }
            FileDemand::Wait => {}
            FileDemand::Done => {
                done = true;
                break;
            }
            _ => {}
        }
        while let Some(event) = session.poll() {
            if matches!(event, SessionEvent::Sample { .. }) {
                emitted += 1;
            }
        }
        assert!(
            offset != before.0
                || eof_sent != before.1
                || emitted != before.2
                || session.file_demand() != before.3,
            "drive stalled at offset {offset} of {len}"
        );
    }
    assert!(done, "drive exhausted its budget");
    emitted
}

fn run(name: &str, extension: &str, container: StreamContainer, source: &str) {
    let _serial = PEAK_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = temp_dir(name);
    let path = dir.join(format!("big.{extension}"));
    let target = target_bytes();
    let made = match container {
        StreamContainer::MpegTs => {
            test_harness::big_media::write_big_ts(&path, &test_fixtures::path(source), target)
        }
        _ => test_harness::big_media::write_big_flv(&path, &test_fixtures::path(source), target),
    };
    let expected = u64::from(PASSES)
        * made
            .samples_per_track
            .iter()
            .map(|track| track.1)
            .sum::<u64>();
    let (peak, emitted) = A.peak_during(|| drive_from_disk(&path, container, CHUNK, PASSES));
    eprintln!("{name}: peak {peak} bytes for a {target}-byte file");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(emitted, expected, "{name}: every sample across every pass");
    assert!(
        peak < BUDGET,
        "{name}: held {peak} bytes for a {target}-byte file"
    );
}

#[test]
fn ts_playout_memory_does_not_grow_with_the_file() {
    run("ts", "ts", StreamContainer::MpegTs, "ts/h264-aac.ts");
}

#[test]
fn flv_playout_memory_does_not_grow_with_the_file() {
    run("flv", "flv", StreamContainer::Flv, "flv/av.flv");
}
