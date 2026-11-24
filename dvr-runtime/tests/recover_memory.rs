//! `FsArchive::recover_spans` reads box headers off the disk and seeks over
//! every payload, so its footprint does not grow with the period file.

#![cfg(feature = "fs")]

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use dvr_runtime::fs::FsArchive;
use dvr_runtime::layout;
use dvr_runtime::recover::RecoveredBox;
use dvr_runtime::{ArchiveContainer, ArchiveName, PeriodNum};

#[global_allocator]
static A: test_alloc::ProcessPeak = test_alloc::ProcessPeak::new();

/// The generated period file's size: large enough that reading it whole is
/// unmistakable against the budget below.
const PERIOD_BYTES: u64 = 512 << 20;

/// Most live bytes recovery may hold above its entry footprint.
const BUDGET: usize = 4 << 20;

/// Bytes of one generated `moof`+`mdat` fragment, header included.
const FRAGMENT_BYTES: u64 = 4 << 20;

/// Bytes of the `moof` header that opens each fragment.
const MOOF_HEADER_BYTES: usize = 16;

/// Bytes of the `mdat` header that closes each fragment.
const MDAT_HEADER_BYTES: usize = 8;

/// Fragments the generated period holds.
const FRAGMENTS: u64 = PERIOD_BYTES / FRAGMENT_BYTES;

/// Spans the scan reports for that period: its init plus every fragment.
const RECOVERED_BOXES: usize = FRAGMENTS as usize + 1;

#[test]
fn recovering_a_large_period_reads_only_headers() {
    let root = temp_dir("recover-memory");
    let name = ArchiveName::new("large").expect("name");
    let dir = layout::route_dir(&root, &name);
    std::fs::create_dir_all(&dir).expect("route dir");
    let init_len = write_period(&dir.join(layout::media_file_name(
        PeriodNum::new(0),
        ArchiveContainer::Fmp4,
    )));

    let archive = FsArchive::open(&root, &name, ArchiveContainer::Fmp4);
    let (peak, recovered) =
        A.peak_during(|| archive.recover_spans(PeriodNum::new(0)).expect("recovers"));

    assert!(
        peak < BUDGET,
        "{peak} bytes live, budget {BUDGET}, period {} bytes",
        PERIOD_BYTES
    );
    assert_eq!(recovered.len(), RECOVERED_BOXES);
    let mut expected = Vec::with_capacity(RECOVERED_BOXES);
    expected.push((0, init_len));
    for index in 0..FRAGMENTS {
        expected.push((init_len + index * FRAGMENT_BYTES, FRAGMENT_BYTES));
    }
    assert_eq!(spans(&recovered), expected);

    let _ = std::fs::remove_dir_all(&root);
}

fn spans(boxes: &[RecoveredBox]) -> Vec<(u64, u64)> {
    boxes
        .iter()
        .map(|recovered| match recovered {
            RecoveredBox::Init(span) | RecoveredBox::Fragment(span) => {
                (span.offset, span.len.get())
            }
            _ => panic!("unknown recovered box"),
        })
        .collect()
}

/// Write a sparse period of `init` plus fragments whose `mdat` bodies are
/// never written, returning the init's length.
fn write_period(path: &Path) -> u64 {
    let init = init();
    let moof = boxed(b"moof", &[0x22; 8]);
    let body = FRAGMENT_BYTES - (MOOF_HEADER_BYTES + MDAT_HEADER_BYTES) as u64;
    let mdat_size = u32::try_from(MDAT_HEADER_BYTES as u64 + body).expect("mdat size");
    let mut file = File::create(path).expect("create period");
    file.write_all(&init).expect("init");
    for _ in 0..FRAGMENTS {
        file.write_all(&moof).expect("moof");
        file.write_all(&mdat_size.to_be_bytes()).expect("mdat size");
        file.write_all(b"mdat").expect("mdat type");
        file.seek(SeekFrom::Current(body as i64))
            .expect("seek body");
    }
    file.set_len(init.len() as u64 + FRAGMENTS * FRAGMENT_BYTES)
        .expect("extend");
    init.len() as u64
}

fn init() -> Vec<u8> {
    let mut bytes = boxed(b"ftyp", b"isom0000");
    bytes.extend_from_slice(&boxed(b"moov", &[0x11; 24]));
    bytes
}

fn boxed(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let size = u32::try_from(8 + payload.len()).expect("size");
    let mut bytes = size.to_be_bytes().to_vec();
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(payload);
    bytes
}

fn temp_dir(tag: &str) -> PathBuf {
    let base = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) if Path::new(&dir).is_absolute() => PathBuf::from(dir),
        Some(dir) => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(dir),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target"),
    };
    let dir = base.join("dvr-runtime-tests").join(format!(
        "{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}
