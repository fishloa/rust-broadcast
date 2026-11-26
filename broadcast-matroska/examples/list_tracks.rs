//! List the tracks of a Matroska or WebM file.
//!
//! ```text
//! cargo run -p broadcast-matroska --example list_tracks -- fixtures/mkv/vp9-opus.webm
//! ```
//!
//! The file is walked with the sans-IO reader's blocking `FileReader`, so only
//! the `Info` and `Tracks` elements are ever read.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::process::ExitCode;

use broadcast_common::bounded::{Limit, read_bounded};
use broadcast_core::Parse;

use broadcast_matroska::Limits;
use broadcast_matroska::driver::{DriverStep, FileReader};
use broadcast_matroska::matroska::Tracks;
use broadcast_matroska::reader::Mode;

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: list_tracks <file.mkv|file.webm>");
        return ExitCode::FAILURE;
    };
    match run(&path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("list_tracks: {error}");
            ExitCode::FAILURE
        }
    }
}

/// One `Tracks` element: the reader hands back its byte range, and it is read
/// back whole because it is a single small wire structure (never a frame).
fn read_element(
    path: &str,
    element: broadcast_matroska::reader::ByteRange,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(element.offset))?;
    let limit = Limit::new("Matroska Tracks element", element.len);
    Ok(read_bounded(file.by_ref().take(element.len), limit)?)
}

/// What one driver step asks the caller to do.
enum Action {
    /// The Segment's `Info` reported its timestamp scale.
    TimestampScale(u64),
    /// The `Tracks` element occupies this byte range.
    Tracks(broadcast_matroska::reader::ByteRange),
    /// No more steps.
    Done,
    /// Nothing this example reports.
    Skip,
}

fn run(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let mut driver = FileReader::new(file, Mode::Permissive, Limits::new())?;
    let mut bytes;
    let mut listed = false;
    loop {
        let action = match driver.step()? {
            DriverStep::Info { info, .. } => Action::TimestampScale(info.timestamp_scale_ns()),
            DriverStep::Tracks { element, .. } => Action::Tracks(element),
            DriverStep::Done => Action::Done,
            _ => Action::Skip,
        };
        match action {
            Action::TimestampScale(scale) => {
                println!("segment timestamp_scale={scale}ns");
            }
            Action::Tracks(element) => {
                bytes = read_element(path, element)?;
                let tracks = Tracks::parse(&bytes)?;
                for entry in tracks.entries() {
                    let number = entry.track_number().unwrap_or(0);
                    let kind = entry
                        .track_type()
                        .map_or("unknown".to_string(), |kind| kind.to_string());
                    let codec = entry.codec_id().unwrap_or("unknown");
                    println!("track {number}: {kind} codec={codec}");
                    if let Some(video) = entry.video() {
                        println!(
                            "  video {}x{} display {}x{}",
                            video.pixel_width().unwrap_or(0),
                            video.pixel_height().unwrap_or(0),
                            video.display_width().unwrap_or(0),
                            video.display_height().unwrap_or(0)
                        );
                    }
                    if let Some(audio) = entry.audio() {
                        println!(
                            "  audio {} Hz {} channels",
                            audio.sampling_frequency().unwrap_or(0.0),
                            audio.channels().unwrap_or(0)
                        );
                    }
                }
                listed = true;
            }
            Action::Done => {
                if !listed {
                    println!("no Tracks element");
                }
                return Ok(());
            }
            Action::Skip => {}
        }
    }
}
