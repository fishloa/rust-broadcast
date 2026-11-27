//! The delivery-count oracle: on a damaged file, what this crate hands out is
//! compared with what `ffprobe -show_packets` finds in the same bytes.
//!
//! The comparison is one-sided on purpose. We may deliver *more* than ffprobe
//! (it drops a whole fragment whose header it cannot read; we deliver the
//! samples before the damage and say where it was), never fewer without a
//! stated reason.
//!
//! Damaged files are made in the test from committed fixtures; nothing damaged
//! is committed.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use isobmff::reader::{Reader, ReaderConfig, SampleOrder, Source, Step};

/// How long `ffprobe` may run on one file.
const FFPROBE_DEADLINE: Duration = Duration::from_secs(30);

/// One damaged input and what it exercises.
struct Case {
    fixture: &'static str,
    label: &'static str,
    damage: Damage,
    /// Why we deliver fewer packets than `ffprobe`, when we do.
    fewer_because: Option<&'static str>,
}

/// The damage applied to a fixture.
#[derive(Clone, Copy)]
enum Damage {
    /// Cut the file at a fraction of its length.
    Truncate(usize, usize),
    /// Add `delta` to the 32-bit count at `offset`.
    Count(usize, i64),
}

const CASES: &[Case] = &[
    Case {
        fixture: "mp4/progressive/av-faststart.mp4",
        label: "truncate@25%",
        damage: Damage::Truncate(1, 4),
        fewer_because: Some(
            "the cut splits a sample: ffprobe counts the packet whose bytes are only partly \
             present, and we never hand out short sample data",
        ),
    },
    Case {
        fixture: "mp4/progressive/av-faststart.mp4",
        label: "truncate@75%",
        damage: Damage::Truncate(3, 4),
        fewer_because: Some(
            "the cut splits a sample: ffprobe counts the packet whose bytes are only partly \
             present, and we never hand out short sample data",
        ),
    },
    Case {
        fixture: "mp4/progressive/av-faststart.mp4",
        label: "count-1",
        damage: Damage::Count(651, -1),
        fewer_because: None,
    },
    Case {
        fixture: "mp4/progressive/av-faststart.mp4",
        label: "count+1",
        damage: Damage::Count(651, 1),
        fewer_because: None,
    },
    Case {
        fixture: "mp4/h264-aac-frag.mp4",
        label: "truncate@60%",
        damage: Damage::Truncate(3, 5),
        fewer_because: Some(
            "the cut splits a sample: ffprobe counts the packet whose bytes are only partly \
             present, and we never hand out short sample data",
        ),
    },
    Case {
        fixture: "mp4/quicktime/lavf-aac-tmcd.mov",
        label: "count-1",
        damage: Damage::Count(78149, -1),
        fewer_because: Some(
            "the sample-count tables disagree: we cut the count to the shortest extent every \
             table agrees on and report the rest, where ffprobe trusts the size table alone",
        ),
    },
];

fn fixture(rel: &str) -> Vec<u8> {
    let path = test_fixtures::require(rel, "ffprobe_agreement").expect("fixture");
    std::fs::read(&path).expect("fixture bytes")
}

fn damaged(bytes: &[u8], damage: Damage) -> Vec<u8> {
    let mut out = bytes.to_vec();
    match damage {
        Damage::Truncate(numerator, denominator) => {
            out.truncate(bytes.len() * numerator / denominator);
        }
        Damage::Count(offset, delta) => {
            let value = u32::from_be_bytes(
                out[offset..offset + 4]
                    .try_into()
                    .expect("four count bytes"),
            );
            let changed = i64::from(value).saturating_add(delta);
            let changed = u32::try_from(changed.max(0)).unwrap_or(u32::MAX);
            out[offset..offset + 4].copy_from_slice(&changed.to_be_bytes());
        }
    }
    out
}

/// Samples this crate delivers from `bytes`.
fn our_samples(bytes: &[u8]) -> Result<usize, String> {
    let config = ReaderConfig::default()
        .with_total_len(Some(bytes.len() as u64))
        .with_order(SampleOrder::File)
        .with_source(Source::Seekable);
    let mut reader = Reader::new(config);
    let mut samples = 0usize;
    loop {
        match reader.step() {
            Ok(Step::Need(range)) => {
                let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                let end = usize::try_from(range.end()).unwrap_or(usize::MAX);
                if let Err(error) =
                    reader.supply(range.offset, bytes.get(start..end).unwrap_or(&[]))
                {
                    return Err(format!("{error:?}"));
                }
            }
            Ok(Step::Sample(_)) => samples += 1,
            Ok(Step::Done) => return Ok(samples),
            Ok(_) => {}
            Err(error) => return Err(format!("{error:?}")),
        }
    }
}

/// Packets `ffprobe -show_packets` finds in the file at `path`.
fn ffprobe_packets(path: &Path) -> usize {
    let mut command = Command::new("ffprobe");
    command.args([
        "-v",
        "error",
        "-show_entries",
        "packet=stream_index",
        "-of",
        "csv=p=0",
    ]);
    command.arg(path);
    match test_fixtures::process::output_bounded(&mut command, FFPROBE_DEADLINE) {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count(),
        Ok(output) => panic!(
            "ffprobe failed on {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(error) => panic!("ffprobe could not run: {error}"),
    }
}

/// A directory beside the test binary for the damaged files, inside the build
/// tree so no scratch lands outside the worktree.
fn scratch_dir() -> PathBuf {
    let mut dir = std::env::current_exe().expect("test binary path");
    dir.pop();
    dir.push("isobmff-ffprobe-agreement");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn a_damaged_file_delivers_at_least_the_packets_ffprobe_finds() {
    if !test_fixtures::process::require_tool("ffprobe", "-version", "REQUIRE_FFPROBE") {
        return;
    }
    let dir = scratch_dir();
    let mut compared = 0usize;
    for case in CASES {
        let bytes = damaged(&fixture(case.fixture), case.damage);
        let path = dir.join(format!(
            "{}-{}.mp4",
            case.fixture.replace('/', "-"),
            case.label.replace(['@', '%', '+'], "_")
        ));
        std::fs::write(&path, &bytes).expect("write damaged file");
        let ours = our_samples(&bytes);
        let theirs = ffprobe_packets(&path);
        let _ = std::fs::remove_file(&path);
        println!(
            "ORACLE {} {}: ours={ours:?} ffprobe={theirs}",
            case.fixture, case.label
        );
        match ours {
            Ok(count) => {
                if count < theirs {
                    assert!(
                        case.fewer_because.is_some(),
                        "{} {}: delivered {count} packets, ffprobe found {theirs}",
                        case.fixture,
                        case.label
                    );
                }
                compared += 1;
            }
            Err(error) => {
                assert!(
                    case.fewer_because.is_some(),
                    "{} {}: refused ({error}), ffprobe found {theirs}",
                    case.fixture,
                    case.label
                );
            }
        }
    }
    assert_eq!(compared, CASES.len(), "every case was comparable");
}
