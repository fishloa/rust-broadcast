//! Remux a Matroska or WebM file through the sans-IO reader and writer, then
//! ask `ffprobe` to read the result when it is installed.
//!
//! ```text
//! cargo run -p broadcast-matroska --example remux -- fixtures/mkv/vp9-opus.webm out.mkv
//! ```

use std::fs::File;
use std::io::{Seek, SeekFrom, Write as _};
use std::process::{Command, ExitCode};
use std::time::Duration;

use broadcast_matroska::Limits;
use broadcast_matroska::driver::{DriverStep, FileReader};
use broadcast_matroska::reader::Mode;
use broadcast_matroska::writer::{
    BlockFlags, InfoSpec, Layout, TrackSpec, Write, Writer, info_element, tracks_element,
};

/// The `Cluster` span this example chooses: Matroska players handle a five
/// second span well. A library never picks this.
const CLUSTER_SPAN_MS: u64 = 5_000;

fn main() -> ExitCode {
    let (Some(input), Some(output)) = (std::env::args().nth(1), std::env::args().nth(2)) else {
        eprintln!("usage: remux <in.mkv|in.webm> <out.mkv>");
        return ExitCode::FAILURE;
    };
    match run(&input, &output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("remux: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(input: &str, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let source = File::open(input)?;
    let mut reader = FileReader::new(source, Mode::Strict, Limits::new())?;
    let mut info = None;
    let mut tracks: Vec<OwnedTrack> = Vec::new();
    let mut frames = Vec::new();
    let mut bytes = Vec::new();
    loop {
        match reader.step()? {
            DriverStep::Info { info: segment, .. } => info = Some(segment),
            DriverStep::Tracks { tracks: table, .. } => {
                tracks = table
                    .iter()
                    .map(|track| OwnedTrack {
                        track_number: track.track_number(),
                        track_uid: track.track_uid(),
                        track_type: track.track_type_value(),
                        codec_id: track.codec_id().to_string(),
                        codec_private: track.codec_private().to_vec(),
                        default_duration_ns: track.default_duration_ns(),
                        video: track.pixel_width().zip(track.pixel_height()),
                        audio: track.channels().zip(track.sampling_frequency()),
                    })
                    .collect();
            }
            DriverStep::Block(block) => {
                reader.read_frame(&block, &mut bytes)?;
                frames.push((block, bytes.clone()));
            }
            DriverStep::Done => break,
            DriverStep::Warning(warning) => eprintln!("remux: warning {warning}"),
            _ => {}
        }
    }
    let Some(info) = info else {
        return Err("input has no Info element".into());
    };
    let specs: Vec<TrackSpec<'_>> = tracks
        .iter()
        .map(|track| TrackSpec {
            track_number: track.track_number,
            track_uid: track.track_uid,
            track_type: track.track_type,
            codec_id: &track.codec_id,
            codec_private: (!track.codec_private.is_empty())
                .then_some(track.codec_private.as_slice()),
            default_duration_ns: track.default_duration_ns,
            video: track.video,
            audio: track.audio,
        })
        .collect();
    let tracks = tracks_element(&specs)?;
    let duration_ticks = info.duration;
    let info = info_element(&InfoSpec {
        timestamp_scale_ns: info.timestamp_scale_ns,
        duration_ticks: duration_ticks.unwrap_or(0.0),
        muxing_app: "transmux",
        writing_app: "transmux",
    })?;
    let mut writer = Writer::new(
        Layout::new(Duration::from_millis(CLUSTER_SPAN_MS)).with_duration_ticks(duration_ticks),
        &info,
        &tracks,
    )?;
    let mut file = File::create(output)?;
    let mut end = 0u64;
    for (block, data) in frames {
        let writes = writer.push_block(
            block.track,
            block.timestamp,
            block.duration,
            BlockFlags::new(block.keyframe, block.discardable, block.invisible),
            &data,
        )?;
        end = apply(writes, &mut file, end)?;
    }
    end = apply(writer.finish()?, &mut file, end)?;
    file.flush()?;
    let _ = end;
    probe(output);
    Ok(())
}

struct OwnedTrack {
    track_number: u64,
    track_uid: u64,
    track_type: u64,
    codec_id: String,
    codec_private: Vec<u8>,
    default_duration_ns: Option<u64>,
    video: Option<(u64, u64)>,
    audio: Option<(u64, f64)>,
}

fn apply(
    writes: Vec<Write>,
    file: &mut File,
    mut end: u64,
) -> Result<u64, Box<dyn std::error::Error>> {
    for write in writes {
        match write {
            Write::Append(bytes) => {
                file.seek(SeekFrom::Start(end))?;
                file.write_all(&bytes)?;
                end += bytes.len() as u64;
            }
            Write::At { offset, bytes } => {
                file.seek(SeekFrom::Start(offset))?;
                file.write_all(&bytes)?;
                file.seek(SeekFrom::End(0))?;
            }
            _ => {}
        }
    }
    Ok(end)
}

fn probe(path: &str) {
    let status = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_name,width,height",
            "-of",
            "compact",
            path,
        ])
        .status();
    match status {
        Ok(status) if status.success() => println!("ffprobe read {path}"),
        Ok(status) => println!("SKIPPED ffprobe check: exited {status}"),
        Err(error) => println!("SKIPPED ffprobe check: {error}"),
    }
}
