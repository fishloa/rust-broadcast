//! The bounded-prefix probe loop: a playout file must be identifiable from at
//! most [`PROBE_PREFIX_MAX`] leading bytes, never the whole file.

use container_probe::{DEFAULT_BUDGET, Probe, probe_with_budget};
use media_plane::file_source::{
    DemuxerKind, FileReaderError, PROBE_PREFIX_MAX, ProbeStep, probe_prefix, select_demuxer,
};

/// The demuxer the whole-file path selects for `bytes`.
fn whole_file_demuxer(bytes: &[u8]) -> DemuxerKind {
    match probe_with_budget(bytes, bytes.len()) {
        Probe::Identified { format, detail, .. } => {
            select_demuxer(format, detail).expect("a playout fixture selects a demuxer")
        }
        other => panic!("fixture must probe cleanly, got {other:?}"),
    }
}

/// Drive the documented caller loop: start at `DEFAULT_BUDGET` and follow
/// every `NeedPrefix` step until the file is identified or the probe fails.
fn identify_from_prefix(bytes: &[u8]) -> Result<DemuxerKind, FileReaderError> {
    let file_len = u64::try_from(bytes.len()).expect("fixture length fits u64");
    let mut len = DEFAULT_BUDGET.min(bytes.len());
    loop {
        match probe_prefix(&bytes[..len], file_len)? {
            ProbeStep::Identified(kind) => return Ok(kind),
            ProbeStep::NeedPrefix { prefix_len } => {
                assert!(prefix_len <= PROBE_PREFIX_MAX, "asked for {prefix_len}");
                assert!(prefix_len <= bytes.len(), "asked for more than the file");
                assert!(prefix_len > len, "the loop must make progress");
                len = prefix_len;
            }
            _ => panic!("unexpected probe step"),
        }
    }
}

#[test]
fn every_playout_fixture_is_identified_from_a_prefix_as_the_whole_file_is() {
    for name in [
        "ts/h264-aac.ts",
        "flv/av.flv",
        "mp4/h264-high.mp4",
        "mp4/cmaf/av-frag.mp4",
        "mkv/h264-aac.mkv",
        "ps/h264-ac3.ps",
    ] {
        let bytes = test_fixtures::read(name);
        assert_eq!(
            identify_from_prefix(&bytes).expect(name),
            whole_file_demuxer(&bytes),
            "{name}"
        );
    }
}

#[test]
fn the_loop_never_asks_for_more_than_the_cap_or_the_file() {
    let bytes = test_fixtures::read("ts/h264-aac-40s.ts");
    assert!(bytes.len() < PROBE_PREFIX_MAX);
    identify_from_prefix(&bytes).expect("a real capture identifies");
    let big = vec![0u8; PROBE_PREFIX_MAX * 2];
    assert!(identify_from_prefix(&big).is_err());
}

#[test]
fn an_empty_file_is_too_short_to_identify() {
    let err = identify_from_prefix(&[]).expect_err("an empty file has no container");
    assert!(
        matches!(err, FileReaderError::FileTooShortToIdentify { .. }),
        "got {err:?}"
    );
}

#[test]
fn a_file_of_zeros_is_unknown_not_a_hang() {
    let zeros = vec![0u8; PROBE_PREFIX_MAX * 2];
    let err = identify_from_prefix(&zeros).expect_err("zeros are not a container");
    assert!(
        matches!(
            err,
            FileReaderError::UnknownProbe | FileReaderError::FileTooShortToIdentify { .. }
        ),
        "got {err:?}"
    );
}

/// A progressive file whose `mdat` runs past the probe prefix leaves the
/// fragmented/progressive layout unsettled; the one streaming MP4 stage reads
/// both, so the selection routes it there rather than failing.
#[test]
fn a_large_progressive_mp4_with_an_unsettled_layout_selects_the_mp4_stage() {
    use container_probe::{Detail, IsobmffLayout};

    const TARGET: u64 = 8 << 20;
    const SAMPLES: u32 = 100;
    let dir = std::env::temp_dir().join(format!("media-plane-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join("big.mp4");
    test_harness::big_media::write_big_mp4(
        &path,
        &test_harness::big_media::BigMp4Options {
            target_bytes: TARGET,
            sample_count: SAMPLES,
            moov_first: true,
            ..test_harness::big_media::BigMp4Options::default()
        },
    );
    let bytes = std::fs::read(&path).expect("read generated file");
    let prefix_len = DEFAULT_BUDGET.min(bytes.len());
    let probe = probe_with_budget(&bytes[..prefix_len], prefix_len);
    let layout = match &probe {
        Probe::Identified {
            detail: Detail::Isobmff { layout, .. },
            ..
        } => *layout,
        other => panic!("expected an identified ISOBMFF file, got {other:?}"),
    };
    assert_eq!(
        layout,
        IsobmffLayout::Unknown,
        "a 64 KiB prefix must not settle a 8 MiB progressive file's layout"
    );
    let Probe::Identified { format, detail, .. } = probe else {
        unreachable!("checked above");
    };
    assert_eq!(
        select_demuxer(format, detail).expect("unsettled ISOBMFF selects a stage"),
        DemuxerKind::Mp4,
        "an unsettled layout must route to the one streaming MP4 stage"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
