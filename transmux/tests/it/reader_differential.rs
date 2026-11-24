//! Differential: the `isobmff` progressive reader against `ProgressiveDemux`.
//!
//! For every fixture the demuxer accepts, each track it keeps must agree with
//! the reader sample by sample on timing, sync and size; the coded bytes must
//! agree too for codecs the demuxer does not rewrite.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use broadcast_common::Unpackage;
use isobmff::sample_table::SampleRef;
use isobmff::{ReadStep, Reader, ReaderConfig, SampleOrder, Step};
use transmux::pipeline::CodecConfig;
use transmux::progressive_demux::ProgressiveDemux;

/// Codecs whose sample bytes `normalise_track_nal_lengths` may rewrite (an
/// AVC/HEVC `lengthSizeMinusOne` other than 3). Their per-sample timing still
/// has to match; their byte length and payload may differ.
fn rewrites_bytes(config: &CodecConfig) -> bool {
    matches!(config, CodecConfig::Avc { .. } | CodecConfig::Hevc { .. })
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e, "mp4" | "m4a" | "mov"))
        {
            out.push(path);
        }
    }
}

fn fixtures() -> Vec<(String, PathBuf)> {
    let mut roots = vec![("public", test_fixtures::root().join("mp4"))];
    if let Some(private) = test_fixtures::private("mp4") {
        roots.push(("private", private));
    }
    let mut out = Vec::new();
    for (tier, root) in roots {
        let mut files = Vec::new();
        collect_files(&root, &mut files);
        files.sort();
        for file in files {
            if let Ok(rel) = file.strip_prefix(&root) {
                out.push((format!("{tier}/{}", rel.display()), file));
            }
        }
    }
    out
}

fn reader_rows(bytes: &[u8]) -> isobmff::Result<Vec<SampleRef>> {
    let config = ReaderConfig::default()
        .with_total_len(Some(bytes.len() as u64))
        .with_order(SampleOrder::File);
    let mut reader = Reader::new(config);
    let mut samples = Vec::new();
    loop {
        match reader.next()? {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                let end = usize::try_from(range.end()).unwrap_or(usize::MAX);
                reader.supply(range.offset, bytes.get(start..end).unwrap_or(&[]))?;
            }
            Step::Sample(sample) => samples.push(sample),
            Step::Tracks(_) => {}
            Step::Done => break,
            _ => break,
        }
    }
    Ok(samples)
}

#[test]
fn the_reader_agrees_with_the_progressive_demuxer() {
    let mut compared_tracks = 0usize;
    let mut compared_samples = 0usize;
    for (key, path) in fixtures() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(mut demux) = ProgressiveDemux::new(bytes.len().max(1)) else {
            continue;
        };
        let Ok(media) = demux.unpackage(bytes.as_slice()) else {
            continue;
        };
        if media.tracks.is_empty() {
            continue;
        }
        let rows = reader_rows(&bytes).expect("reader");
        let mut by_track: BTreeMap<u32, Vec<SampleRef>> = BTreeMap::new();
        for sample in &rows {
            by_track
                .entry(sample.track.value())
                .or_default()
                .push(*sample);
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
                    usize::try_from(reader_sample.decode_time).expect("decode time"),
                    usize::try_from(timing.dts().expect("dts")).expect("dts"),
                    "{key}: track {id} decode time"
                );
                assert_eq!(
                    Some(reader_sample.duration),
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
        compared_tracks >= 10,
        "only {compared_tracks} tracks compared"
    );
    assert!(
        compared_samples >= 1000,
        "only {compared_samples} samples compared"
    );
}
