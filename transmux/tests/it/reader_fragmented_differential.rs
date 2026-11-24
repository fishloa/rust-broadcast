//! Differential: the `isobmff` fragmented reader against `Fmp4Demux`.
//!
//! For every fragmented fixture in the workspace, each track the demuxer keeps
//! must agree with the reader sample by sample on decode time, duration, sync,
//! composition offset, size and (for codecs the demuxer does not rewrite) the
//! coded bytes. CENC fixtures must also agree on every sample's IV and
//! subsample map.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use broadcast_common::Unpackage;
use isobmff::movie_fragment::MovieFragmentBox;
use isobmff::sample_table::SampleRef;
use isobmff::{Reader, ReaderConfig, SampleOrder, Step};
use transmux::cenc_decrypt::CencDecryptor;
use transmux::media::Fmp4Demux;
use transmux::pipeline::CodecConfig;

/// Fragmented fixtures the walk must compare: a checkout with no usable files
/// fails rather than passing vacuously.
const MIN_FRAGMENTED_FIXTURES: usize = 40;
/// Tracks the walk must compare.
const MIN_DIFFERENTIAL_TRACKS: usize = 50;
/// Samples the walk must compare.
const MIN_DIFFERENTIAL_SAMPLES: usize = 8_000;
/// CENC fixtures whose per-sample IV/subsample map must be compared.
const MIN_CENC_FIXTURES: usize = 8;

/// Codecs whose sample bytes `normalise_track_nal_lengths` may rewrite (an
/// AVC/HEVC `lengthSizeMinusOne` other than 3). Their timing still has to
/// match; their byte length and payload may differ.
fn rewrites_bytes(config: &CodecConfig) -> bool {
    matches!(config, CodecConfig::Avc { .. } | CodecConfig::Hevc { .. })
}

fn media_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e, "mp4" | "m4s" | "mov" | "m4a"))
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else if media_extension(&path) {
            out.push(path);
        }
    }
}

/// Every directory holding segment files, paired with the concatenated bytes
/// of one self-contained candidate: an init segment followed by the media
/// segments that belong to it, or a lone self-contained file.
fn candidates() -> Vec<(String, Vec<u8>)> {
    let mut roots = vec![
        test_fixtures::root().join("mp4"),
        test_fixtures::root().join("dash"),
        test_fixtures::root().join("hls/cmaf-fmp4"),
        test_fixtures::root().join("scte35/dash"),
        test_fixtures::root().join("subtitles/mp4box/dash"),
    ];
    if let Some(private) = test_fixtures::private("mp4") {
        roots.push(private);
    }
    let mut dirs = Vec::new();
    for root in roots {
        collect_files(&root, &mut dirs);
    }
    let mut dir_names: Vec<PathBuf> = Vec::new();
    for file in &dirs {
        if let Some(parent) = file.parent()
            && !dir_names.contains(&parent.to_path_buf())
        {
            dir_names.push(parent.to_path_buf());
        }
    }
    dir_names.sort();
    let mut out = Vec::new();
    for dir in dir_names {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| media_extension(path))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        let inits: Vec<&PathBuf> = files
            .iter()
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains("init"))
            })
            .collect();
        if inits.is_empty() {
            for file in files {
                let Ok(bytes) = std::fs::read(&file) else {
                    continue;
                };
                out.push((file.display().to_string(), bytes));
            }
            continue;
        }
        for init in inits {
            let Some(init_name) = init.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let tag = init_name
                .split("init")
                .nth(1)
                .unwrap_or("")
                .trim_start_matches('-')
                .trim_end_matches(".m4s")
                .trim_end_matches(".mp4")
                .to_string();
            let mut group = std::fs::read(init).unwrap_or_default();
            let mut pieces = Vec::new();
            for file in &files {
                if file == init {
                    continue;
                }
                let Some(name) = file.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if name.contains("init") {
                    continue;
                }
                let belongs = if tag.is_empty() {
                    !name.contains("init")
                } else {
                    name.contains(&tag)
                };
                if belongs {
                    pieces.push(file.clone());
                }
            }
            pieces.sort();
            for piece in pieces {
                if let Ok(bytes) = std::fs::read(piece) {
                    group.extend_from_slice(&bytes);
                }
            }
            out.push((format!("{}", init.display()), group));
        }
    }
    out
}

fn reader_rows(bytes: &[u8]) -> isobmff::Result<(Vec<isobmff::TrackId>, Vec<SampleRef>)> {
    let config = ReaderConfig::default()
        .with_total_len(Some(bytes.len() as u64))
        .with_order(SampleOrder::DecodeTime);
    let mut reader = Reader::new(config);
    let mut samples = Vec::new();
    let mut tracks = Vec::new();
    loop {
        match reader.step()? {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                let end = usize::try_from(range.end()).unwrap_or(usize::MAX);
                reader.supply(range.offset, bytes.get(start..end).unwrap_or(&[]))?;
            }
            Step::Fragment(_) => {}
            Step::Sample(sample) => samples.push(sample),
            Step::Tracks(infos) => tracks = infos.iter().map(|info| info.track_id).collect(),
            Step::Done => break,
            _ => {}
        }
    }
    Ok((tracks, samples))
}

/// Whether a candidate carries a top-level `moof`, which is what makes it a
/// fragmented file rather than a progressive one the demuxer only inits.
fn is_fragmented(bytes: &[u8]) -> bool {
    isobmff::BoxIter::clamped(bytes)
        .filter_map(|item| item.ok())
        .any(|item| item.header.box_type.is(b"moof"))
}

#[test]
fn the_reader_agrees_with_the_fragmented_demuxer() {
    let mut fixtures = 0usize;
    let mut compared_tracks = 0usize;
    let mut compared_samples = 0usize;
    for (key, bytes) in candidates() {
        if !is_fragmented(&bytes) {
            continue;
        }
        let Ok(media) = Fmp4Demux::new().unpackage(bytes.as_slice()) else {
            continue;
        };
        if media.tracks.is_empty() {
            continue;
        }
        let rows = match reader_rows(&bytes) {
            Ok((_, rows)) => rows,
            Err(err) => {
                // The reader is strict where the demuxer degrades a malformed
                // fragment; a fixture the demuxer reports as degraded is
                // compared up to that fragment by not being compared at all.
                assert!(
                    !media.degraded.is_empty(),
                    "{key}: reader refused a file the demuxer accepted cleanly: {err}"
                );
                continue;
            }
        };
        fixtures += 1;
        let mut by_track: BTreeMap<u32, Vec<SampleRef>> = BTreeMap::new();
        for sample in &rows {
            by_track
                .entry(sample.track.value())
                .or_default()
                .push(sample.clone());
        }
        for track in &media.tracks {
            let id = track.spec.track_id.value();
            let reader_samples = by_track.get(&id).cloned().unwrap_or_default();
            assert_eq!(
                reader_samples.len(),
                track.samples.len(),
                "{key}: track {id} sample count"
            );
            if reader_samples.is_empty() {
                continue;
            }
            let rewrite = rewrites_bytes(&track.spec.config);
            for (reader_sample, media_sample) in reader_samples.iter().zip(&track.samples) {
                let timing = media_sample.timing.expect("timed sample");
                assert_eq!(
                    reader_sample.decode_time,
                    u64::try_from(timing.dts().expect("dts")).expect("dts"),
                    "{key}: track {id} decode time"
                );
                // The IR carries a duration only when it is positive; a
                // fragment that declares none is 0 on both sides.
                assert_eq!(
                    (reader_sample.duration != 0).then_some(reader_sample.duration),
                    timing.duration(),
                    "{key}: track {id} duration"
                );
                assert_eq!(
                    reader_sample.sync, media_sample.flags.is_sync,
                    "{key}: track {id} sync"
                );
                assert_eq!(
                    i64::from(reader_sample.composition_offset),
                    i64::from(
                        media_sample
                            .composition_offset()
                            .expect("offset")
                            .unwrap_or(0)
                    ),
                    "{key}: track {id} composition offset"
                );
                if !rewrite {
                    assert_eq!(
                        reader_sample.size as usize,
                        media_sample.data.len(),
                        "{key}: track {id} sample size"
                    );
                    let start = usize::try_from(reader_sample.offset).expect("offset");
                    let end = start.saturating_add(reader_sample.size as usize);
                    assert_eq!(
                        &bytes[start..end],
                        media_sample.data.as_ref(),
                        "{key}: track {id} sample bytes"
                    );
                }
                compared_samples += 1;
            }
            compared_tracks += 1;
        }
    }
    assert!(
        fixtures >= MIN_FRAGMENTED_FIXTURES,
        "only {fixtures} fragmented fixtures compared"
    );
    assert!(
        compared_tracks >= MIN_DIFFERENTIAL_TRACKS,
        "only {compared_tracks} tracks compared"
    );
    assert!(
        compared_samples >= MIN_DIFFERENTIAL_SAMPLES,
        "only {compared_samples} samples compared"
    );
}

#[test]
fn the_reader_agrees_with_the_cenc_harvest() {
    let mut cenc_fixtures = 0usize;
    let mut compared_samples = 0usize;
    for (key, bytes) in candidates() {
        if !is_fragmented(&bytes) {
            continue;
        }
        let Ok(decryptor) = CencDecryptor::from_fmp4(&bytes) else {
            continue;
        };
        let entries = decryptor.sample_entries();
        if entries.is_empty() {
            continue;
        }
        let Ok((_, rows)) = reader_rows(&bytes) else {
            continue;
        };
        // The harvest reports the *first* protected track in `moov` order; find
        // it by the one reader track whose per-sample auxiliary map has the
        // same length and IV widths.
        let mut by_track: BTreeMap<u32, Vec<SampleRef>> = BTreeMap::new();
        for sample in &rows {
            by_track
                .entry(sample.track.value())
                .or_default()
                .push(sample.clone());
        }
        let mut matched = None;
        for (track, samples) in &by_track {
            let aux: Vec<_> = samples
                .iter()
                .filter_map(|sample| sample.aux.as_ref())
                .collect();
            if aux.len() != entries.len() {
                continue;
            }
            if aux.iter().zip(entries).all(|(aux, entry)| {
                aux.initialization_vector.len() == entry.initialization_vector.len()
            }) {
                matched = Some((*track, samples.clone()));
                break;
            }
        }
        let Some((track, samples)) = matched else {
            continue;
        };
        for (sample, entry) in samples.iter().zip(entries) {
            let Some(aux) = sample.aux.as_ref() else {
                panic!("{key}: track {track} sample {} has no aux", sample.index);
            };
            assert_eq!(
                aux.initialization_vector, entry.initialization_vector,
                "{key}: track {track} sample {} IV",
                sample.index
            );
            assert_eq!(
                aux.subsamples, entry.subsamples,
                "{key}: track {track} sample {} subsamples",
                sample.index
            );
            compared_samples += 1;
        }
        cenc_fixtures += 1;
    }
    assert!(
        cenc_fixtures >= MIN_CENC_FIXTURES,
        "only {cenc_fixtures} CENC fixtures compared"
    );
    assert!(compared_samples > 0, "no CENC sample compared");
}

// ---------------------------------------------------------------------------
// Synthetic layouts: the header-only walk against the whole-file resolver
// ---------------------------------------------------------------------------

/// A 4-byte `size` + 4-byte `type` header around `payload`.
fn box_bytes(ty: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(
        &u32::try_from(8 + payload.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    out.extend_from_slice(ty);
    out.extend_from_slice(payload);
    out
}

/// A FullBox: version, 24-bit flags, then the payload.
fn full_box(ty: &[u8; 4], version: u8, flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut body = vec![
        version,
        ((flags >> 16) & 0xFF) as u8,
        ((flags >> 8) & 0xFF) as u8,
        (flags & 0xFF) as u8,
    ];
    body.extend_from_slice(payload);
    box_bytes(ty, &body)
}

const SYNTH_TRACK: u32 = 1;
const SYNTH_SAMPLE_SIZE: u32 = 8;
const SYNTH_SAMPLE_DURATION: u32 = 1_000;

/// A `moof` with one `traf` of one sample, addressed `data_offset` bytes from
/// the `moof` start (`default-base-is-moof`).
fn synthetic_moof(data_offset: i32) -> Vec<u8> {
    let mut mfhd = Vec::new();
    mfhd.extend_from_slice(&1u32.to_be_bytes());
    const TFHD_DEFAULT_BASE_IS_MOOF: u32 = 0x02_0000;
    let mut tfhd = Vec::new();
    tfhd.extend_from_slice(&SYNTH_TRACK.to_be_bytes());
    let mut tfdt = Vec::new();
    tfdt.extend_from_slice(&0u64.to_be_bytes());
    const TRUN_DATA_OFFSET_AND_DURATION_AND_SIZE: u32 = 0x000301;
    let mut trun = Vec::new();
    trun.extend_from_slice(&1u32.to_be_bytes());
    trun.extend_from_slice(&data_offset.to_be_bytes());
    trun.extend_from_slice(&SYNTH_SAMPLE_DURATION.to_be_bytes());
    trun.extend_from_slice(&SYNTH_SAMPLE_SIZE.to_be_bytes());
    let traf = box_bytes(
        b"traf",
        &[
            full_box(b"tfhd", 0, TFHD_DEFAULT_BASE_IS_MOOF, &tfhd),
            full_box(b"tfdt", 1, 0, &tfdt),
            full_box(b"trun", 1, TRUN_DATA_OFFSET_AND_DURATION_AND_SIZE, &trun),
        ]
        .concat(),
    );
    box_bytes(b"moof", &[full_box(b"mfhd", 0, 0, &mfhd), traf].concat())
}

/// The `mdat` forms a fragment's offsets may address.
#[derive(Debug, Clone, Copy)]
enum MdatForm {
    /// The run points past an empty `mdat` into a later one.
    LaterMdat,
    /// The `mdat` uses the 64-bit `largesize` form.
    Largesize,
    /// The `mdat` uses `size == 0`, running to the end of the file.
    ToEnd,
}

/// A fragmented init segment with one closed-GOP video track and no samples.
fn synthetic_init() -> Vec<u8> {
    use broadcast_common::timescale::Timescale;
    use isobmff::writer::{SampleEntry, TrackSetup, Writer};
    // A raw entry this crate has no typed form for: the test addresses
    // fragment samples, not sample-entry parsing.
    let entry_body = box_bytes(b"raw1", &[0u8; 78]);
    let setup = TrackSetup::new(
        isobmff::TrackId::new(SYNTH_TRACK),
        *b"vide",
        vec![],
        Timescale::try_new(1_000).expect("timescale"),
        SampleEntry::Raw {
            fourcc: *b"raw1",
            body: entry_body,
        },
    );
    let writer = Writer::fragmented(
        vec![setup],
        Timescale::try_new(1_000).expect("timescale"),
        isobmff::Limits::default(),
    )
    .expect("fragmented writer");
    writer.init().expect("init segment")
}

fn synthetic_file(init: &[u8], form: MdatForm) -> Vec<u8> {
    let placeholder = synthetic_moof(0);
    let payload = match form {
        MdatForm::LaterMdat => placeholder.len() + 8 + 8,
        MdatForm::Largesize => placeholder.len() + 16,
        MdatForm::ToEnd => placeholder.len() + 8,
    };
    let moof = synthetic_moof(i32::try_from(payload).expect("payload offset"));
    let mut file = init.to_vec();
    file.extend_from_slice(&moof);
    match form {
        MdatForm::LaterMdat => {
            file.extend_from_slice(&box_bytes(b"mdat", &[]));
            file.extend_from_slice(&box_bytes(b"mdat", &[0xAA; SYNTH_SAMPLE_SIZE as usize]));
        }
        MdatForm::Largesize => {
            let mut header = Vec::new();
            header.extend_from_slice(&1u32.to_be_bytes());
            header.extend_from_slice(b"mdat");
            header.extend_from_slice(&(16 + u64::from(SYNTH_SAMPLE_SIZE)).to_be_bytes());
            file.extend_from_slice(&header);
            file.extend_from_slice(&[0xBB; SYNTH_SAMPLE_SIZE as usize]);
        }
        MdatForm::ToEnd => {
            let mut header = Vec::new();
            header.extend_from_slice(&0u32.to_be_bytes());
            header.extend_from_slice(b"mdat");
            file.extend_from_slice(&header);
            file.extend_from_slice(&[0xCC; SYNTH_SAMPLE_SIZE as usize]);
        }
    }
    file
}

#[test]
fn a_later_largesize_or_to_end_mdat_addresses_the_same_bytes_as_the_resolver() {
    let init = synthetic_init();
    for form in [MdatForm::LaterMdat, MdatForm::Largesize, MdatForm::ToEnd] {
        let file = synthetic_file(&init, form);
        let moof = isobmff::BoxIter::clamped(&file)
            .filter_map(|item| item.ok())
            .find(|item| item.header.box_type.is(b"moof"))
            .expect("the synthetic file carries a moof");
        let parsed = MovieFragmentBox::parse_body(moof.body).expect("moof parses");
        let mdats = isobmff::frag_offsets::mdat_ranges(&file);
        let expected = isobmff::frag_offsets::sample_ranges_in(
            moof.start as u64,
            &parsed,
            isobmff::TrackId::new(SYNTH_TRACK),
            &mdats,
            isobmff::frag_offsets::sample_bytes_budget(file.len() as u64),
        )
        .unwrap_or_else(|err| panic!("{form:?}: whole-file resolver: {err}"));
        let (_, rows) = reader_rows(&file).unwrap_or_else(|err| panic!("{form:?}: reader: {err}"));
        assert_eq!(rows.len(), expected.len(), "{form:?}: sample count");
        for (row, want) in rows.iter().zip(&expected) {
            assert_eq!(row.offset, want.start, "{form:?}: offset");
            assert_eq!(u64::from(row.size), want.end - want.start, "{form:?}: size");
            assert_eq!(
                &file[usize::try_from(row.offset).expect("offset")
                    ..usize::try_from(row.offset + u64::from(row.size)).expect("end")],
                &file[usize::try_from(want.start).expect("start")
                    ..usize::try_from(want.end).expect("end")],
                "{form:?}: the same bytes"
            );
        }
    }
}
