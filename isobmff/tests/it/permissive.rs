//! Best effort is the reader's default: it tolerates the faults the design
//! lists, each published once as the workspace's one report, a
//! [`Damage`](broadcast_core::damage::Damage); strict mode keeps failing them.
//!
//! Every damaged input is made in the test from a real fixture; no damaged
//! file is committed.

use std::io::Cursor;

use broadcast_core::damage::{Damage, DamageKind};
use isobmff::driver::{DriverStep, FileReader, ForwardReader};
use isobmff::reader::{ByteRange, Mode, Reader, ReaderConfig, SampleOrder, Step};
use isobmff::sample_table::SampleRef;
use isobmff::{Error, Limits};

struct Drive {
    samples: Vec<SampleRef>,
    warnings: Vec<Damage>,
    error: Option<Error>,
}

fn drive(data: &[u8], config: ReaderConfig) -> Drive {
    let mut reader =
        Reader::new(config.with_total_len(Some(u64::try_from(data.len()).expect("length"))));
    let mut drive = Drive {
        samples: Vec::new(),
        warnings: Vec::new(),
        error: None,
    };
    loop {
        match reader.step() {
            Ok(Step::Need(ByteRange { offset, len })) => {
                let start = usize::try_from(offset).expect("offset");
                let end = usize::try_from(offset + len).expect("end");
                if let Err(error) = reader.supply(offset, &data[start..end]) {
                    drive.error = Some(error);
                    break;
                }
            }
            Ok(Step::Sample(sample)) => drive.samples.push(sample),
            Ok(Step::Damage(warning)) => drive.warnings.push(warning),
            Ok(Step::Done) => break,
            Ok(_) => {}
            Err(error) => {
                drive.error = Some(error);
                break;
            }
        }
    }
    drive
}

fn strict(data: &[u8]) -> Drive {
    drive(
        data,
        ReaderConfig::default()
            .with_order(SampleOrder::File)
            .with_mode(Mode::Strict),
    )
}

fn permissive(data: &[u8]) -> Drive {
    drive(data, ReaderConfig::default().with_order(SampleOrder::File))
}

fn fixture(rel: &str) -> Vec<u8> {
    let path = test_fixtures::require(&format!("mp4/{rel}"), "permissive").expect("fixture");
    std::fs::read(&path).expect("fixture bytes")
}

fn top_boxes(data: &[u8]) -> Vec<(usize, usize, [u8; 4])> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 8 <= data.len() {
        let size = usize::try_from(u32::from_be_bytes([
            data[at],
            data[at + 1],
            data[at + 2],
            data[at + 3],
        ]))
        .expect("size");
        if size < 8 || at + size > data.len() {
            break;
        }
        out.push((at, size, data[at + 4..at + 8].try_into().expect("fourcc")));
        at += size;
    }
    out
}

fn top(data: &[u8], fourcc: &[u8; 4]) -> (usize, usize) {
    top_boxes(data)
        .into_iter()
        .find(|(_, _, found)| found == fourcc)
        .map(|(at, size, _)| (at, size))
        .unwrap_or_else(|| panic!("{fourcc:?} box"))
}

fn last_moof(data: &[u8]) -> usize {
    top_boxes(data)
        .into_iter()
        .filter(|(_, _, fourcc)| fourcc == b"moof")
        .map(|(at, _, _)| at)
        .next_back()
        .expect("a moof")
}

/// The last `moof`'s first `trun` sample count, inflated so its runs no longer
/// fit the following `mdat`.
fn damage_middle_moof(data: &[u8]) -> Vec<u8> {
    let mut damaged = data.to_vec();
    let moofs: Vec<usize> = top_boxes(data)
        .into_iter()
        .filter(|(_, _, fourcc)| fourcc == b"moof")
        .map(|(at, _, _)| at)
        .collect();
    let at = moofs[1];
    let end = at
        + top_boxes(&data[at..])
            .first()
            .map(|(_, size, _)| *size)
            .expect("moof size");
    let needle = data[at..end]
        .windows(4)
        .position(|w| w == b"trun")
        .expect("trun")
        + at;
    let count_at = needle + 8;
    let count = u32::from_be_bytes(damaged[count_at..count_at + 4].try_into().expect("count"));
    let inflated = count.saturating_add(1024);
    damaged[count_at..count_at + 4].copy_from_slice(&inflated.to_be_bytes());
    damaged
}

fn damage_mvhd_timescale(data: &[u8]) -> Vec<u8> {
    let mut damaged = data.to_vec();
    let (moov, _) = top(data, b"moov");
    let mut at = moov + 8;
    while at + 8 <= data.len() {
        let size = usize::try_from(u32::from_be_bytes([
            data[at],
            data[at + 1],
            data[at + 2],
            data[at + 3],
        ]))
        .expect("size");
        if &data[at + 4..at + 8] == b"mvhd" {
            let version = data[at + 8];
            let timescale = if version == 1 { at + 28 } else { at + 20 };
            damaged[timescale..timescale + 4].fill(0);
            return damaged;
        }
        at += size;
    }
    panic!("mvhd");
}

fn damage_moov_child_overrun(data: &[u8]) -> Vec<u8> {
    let mut damaged = data.to_vec();
    let (moov, size) = top(data, b"moov");
    let children = top_boxes(&data[moov + 8..moov + size]);
    let (at, _, _) = *children.last().expect("a moov child");
    let absolute = moov + 8 + at;
    let old = u32::from_be_bytes(damaged[absolute..absolute + 4].try_into().expect("size"));
    damaged[absolute..absolute + 4].copy_from_slice(&old.saturating_add(4).to_be_bytes());
    damaged
}

fn stsd_entry(data: &[u8]) -> usize {
    let (moov, size) = top(data, b"moov");
    find_box(data, moov + 8, moov + size, b"stsd").expect("stsd") + 16
}

fn find_box(data: &[u8], start: usize, end: usize, fourcc: &[u8; 4]) -> Option<usize> {
    let mut at = start;
    while at + 8 <= end {
        let size = usize::try_from(u32::from_be_bytes([
            data[at],
            data[at + 1],
            data[at + 2],
            data[at + 3],
        ]))
        .ok()?;
        if size < 8 || at + size > end {
            return None;
        }
        if &data[at + 4..at + 8] == fourcc {
            return Some(at);
        }
        if matches!(
            &data[at + 4..at + 8],
            b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"dinf" | b"edts" | b"udta" | b"mvex"
        ) && let Some(found) = find_box(data, at + 8, at + size, fourcc)
        {
            return Some(found);
        }
        at += size;
    }
    None
}

fn damage_avc_config(data: &[u8]) -> Vec<u8> {
    let mut damaged = data.to_vec();
    let start = stsd_entry(data);
    let end = start
        + usize::try_from(u32::from_be_bytes(
            data[start..start + 4].try_into().expect("size"),
        ))
        .expect("size");
    let needle = data[start..end]
        .windows(4)
        .position(|w| w == b"avcC")
        .expect("avcC")
        + start;
    damaged[needle..needle + 4].copy_from_slice(b"xxxx");
    damaged
}

fn damage_wave_esds(data: &[u8]) -> Vec<u8> {
    let mut damaged = data.to_vec();
    let (moov, _) = top(data, b"moov");
    let needle = data[moov..]
        .windows(4)
        .position(|w| w == b"esds")
        .expect("esds")
        + moov;
    damaged[needle..needle + 4].copy_from_slice(b"xxxx");
    damaged
}

fn sample_key(sample: &SampleRef) -> (u32, u32, u64, u64, u32) {
    (
        sample.track.value(),
        sample.index,
        sample.decode_time,
        sample.offset,
        sample.size,
    )
}

fn assert_prefix(permissive: &[SampleRef], undamaged: &[SampleRef], before: u64) {
    let expected: Vec<_> = undamaged
        .iter()
        .filter(|sample| sample.offset < before)
        .map(sample_key)
        .collect();
    let got: Vec<_> = permissive
        .iter()
        .filter(|sample| sample.offset < before)
        .map(sample_key)
        .collect();
    assert_eq!(got, expected);
}

#[test]
fn a_moov_child_overrun_is_clamped_with_a_warning() {
    let data = fixture("quicktime/lavf-aac-tmcd.mov");
    let damaged = damage_moov_child_overrun(&data);
    assert!(strict(&damaged).error.is_some(), "strict refuses");
    let good = strict(&data);
    let lenient = permissive(&damaged);
    assert!(matches!(
        lenient.warnings.as_slice(),
        [damage, ..] if damage.what() == "clamped-box" && damage.kind() == DamageKind::MetadataSkipped
    ));
    assert_eq!(
        lenient.samples.iter().map(sample_key).collect::<Vec<_>>(),
        good.samples.iter().map(sample_key).collect::<Vec<_>>()
    );
}

#[test]
fn an_unparseable_sample_entry_is_kept_raw_with_a_warning() {
    let data = fixture("camera/gopro-karma.mp4");
    let damaged = damage_avc_config(&data);
    assert!(strict(&damaged).error.is_some(), "strict refuses");
    let good = strict(&data);
    let lenient = permissive(&damaged);
    assert!(lenient.warnings.iter().any(|damage| {
        damage.what() == "unparseable-sample-entry" && damage.fourcc() == Some(*b"avc1")
    }));
    assert_eq!(lenient.samples.len(), good.samples.len());
}

#[test]
fn a_wave_atom_without_esds_is_kept_with_a_warning() {
    let data = fixture("quicktime/lavf-aac-tmcd.mov");
    let damaged = damage_wave_esds(&data);
    assert!(strict(&damaged).error.is_some(), "strict refuses");
    let good = strict(&data);
    let lenient = permissive(&damaged);
    assert!(matches!(
        lenient.warnings.as_slice(),
        [damage, ..] if damage.what() == "wave-without-esds"
    ));
    assert_eq!(
        lenient.samples.iter().map(sample_key).collect::<Vec<_>>(),
        good.samples.iter().map(sample_key).collect::<Vec<_>>()
    );
}

#[test]
fn a_zero_movie_timescale_is_reported_with_a_warning() {
    let data = fixture("progressive/av-faststart.mp4");
    let damaged = damage_mvhd_timescale(&data);
    assert!(strict(&damaged).error.is_some(), "strict refuses");
    let good = strict(&data);
    let lenient = permissive(&damaged);
    assert!(matches!(
        lenient.warnings.as_slice(),
        [damage, ..] if damage.what() == "zero-movie-timescale"
            && damage.kind() == DamageKind::TimelineRepaired
    ));
    assert_eq!(
        lenient.samples.iter().map(sample_key).collect::<Vec<_>>(),
        good.samples.iter().map(sample_key).collect::<Vec<_>>()
    );
}

#[test]
fn a_truncated_final_mdat_emits_the_samples_inside_it() {
    let data = fixture("progressive/av-faststart.mp4");
    let truncated = &data[..data.len() - 256];
    let strict_drive = strict(truncated);
    assert!(strict_drive.error.is_some());
    let lenient = permissive(truncated);
    assert!(
        matches!(
            lenient.warnings.as_slice(),
            [damage] if damage.kind() == DamageKind::InputTruncated && damage.what() == "truncated-mdat"
        ),
        "{:?}",
        lenient.warnings
    );
    assert_eq!(
        lenient.samples.iter().map(sample_key).collect::<Vec<_>>(),
        strict_drive
            .samples
            .iter()
            .map(sample_key)
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_truncated_final_moof_is_reported_after_the_complete_fragments() {
    let data = fixture("h264-aac-frag.mp4");
    let cut = last_moof(&data) + 12;
    let truncated = &data[..cut];
    assert!(strict(truncated).error.is_some());
    let good = strict(&data);
    let lenient = permissive(truncated);
    assert!(matches!(
        lenient.warnings.as_slice(),
        [damage] if damage.kind() == DamageKind::InputTruncated
            && damage.what() == "truncated-fragment"
    ));
    assert_prefix(
        &lenient.samples,
        &good.samples,
        u64::try_from(cut).expect("cut"),
    );
}

#[test]
fn a_malformed_middle_moof_is_skipped_with_a_warning() {
    let data = fixture("h264-aac-frag.mp4");
    let damaged = damage_middle_moof(&data);
    assert!(strict(&damaged).error.is_some(), "strict refuses");
    let good = strict(&data);
    let lenient = permissive(&damaged);
    let moofs: Vec<usize> = top_boxes(&data)
        .into_iter()
        .filter(|(_, _, fourcc)| fourcc == b"moof")
        .map(|(at, _, _)| at)
        .collect();
    let fault = u64::try_from(moofs[1]).expect("offset");
    assert!(matches!(
        lenient.warnings.as_slice(),
        [damage] if damage.kind() == DamageKind::Resynchronised
            && damage.location().byte_range().map(|range| range.start()) == Some(fault)
    ));
    assert_prefix(&lenient.samples, &good.samples, fault);
    assert!(lenient.samples.len() > lenient.samples.iter().filter(|s| s.offset < fault).count());
}

#[test]
fn a_limits_violation_fails_identically_in_both_modes() {
    let data = fixture("quicktime/lavf-aac-tmcd.mov");
    let limits = Limits::default().with_max_moov_bytes(64);
    for mode in [Mode::Strict, Mode::Permissive] {
        let drive = drive(
            &data,
            ReaderConfig::default().with_mode(mode).with_limits(limits),
        );
        assert!(matches!(
            drive.error,
            Some(Error::LimitExceeded {
                what: "moov size",
                ..
            })
        ));
        assert!(drive.warnings.is_empty());
    }
}

#[test]
fn both_drivers_pass_a_warning_through() {
    let data = fixture("progressive/av-faststart.mp4");
    let damaged = damage_mvhd_timescale(&data);
    let config = ReaderConfig::default();
    let mut file = FileReader::new(Cursor::new(damaged.clone()), config.clone()).expect("file");
    assert_warning(&mut file);
    let mut forward = ForwardReader::new(Cursor::new(damaged), config);
    assert_warning_forward(&mut forward);
}

fn assert_warning<R: std::io::Read + std::io::Seek>(reader: &mut FileReader<R>) {
    loop {
        match reader.step().expect("driver step") {
            DriverStep::Damage(damage) if damage.what() == "zero-movie-timescale" => return,
            DriverStep::Done => panic!("no warning"),
            _ => {}
        }
    }
}

fn assert_warning_forward<R: std::io::Read>(reader: &mut ForwardReader<R>) {
    loop {
        match reader.step().expect("driver step") {
            DriverStep::Damage(damage) if damage.what() == "zero-movie-timescale" => return,
            DriverStep::Done => panic!("no warning"),
            _ => {}
        }
    }
}

#[test]
fn a_cut_final_mdat_emits_its_complete_prefix() {
    let data = fixture("h264-aac-frag.mp4");
    let (mdat, size) = top_boxes(&data)
        .into_iter()
        .filter(|(_, _, fourcc)| fourcc == b"mdat")
        .map(|(at, size, _)| (at, size))
        .next_back()
        .expect("truncated-mdat");
    let cut = mdat + 8 + (size - 8) / 2;
    let truncated = &data[..cut];
    assert!(strict(truncated).error.is_some());
    let good = strict(&data);
    let lenient = permissive(truncated);
    assert!(
        matches!(
            lenient.warnings.as_slice(),
            [damage] if damage.kind() == DamageKind::InputTruncated
        ),
        "{:?}",
        lenient.warnings
    );
    let cut64 = u64::try_from(cut).expect("cut");
    let expected: Vec<_> = good
        .samples
        .iter()
        .filter(|sample| u64::from(sample.size).saturating_add(sample.offset) <= cut64)
        .map(sample_key)
        .collect();
    let got: Vec<_> = lenient.samples.iter().map(sample_key).collect();
    assert_eq!(got, expected);
}
