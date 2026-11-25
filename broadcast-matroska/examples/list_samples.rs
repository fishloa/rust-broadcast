//! List the tracks, Info and every Block frame of a Matroska or WebM file
//! through the sans-IO reader's blocking driver.
//!
//! ```text
//! cargo run -p broadcast-matroska --example list_samples -- fixtures/mkv/vp9-opus.webm
//! ```

use std::fs::File;
use std::process::ExitCode;

use broadcast_matroska::Limits;
use broadcast_matroska::driver::{DriverStep, FileReader};
use broadcast_matroska::reader::Mode;

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: list_samples <file.mkv|file.webm>");
        return ExitCode::FAILURE;
    };
    match run(&path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("list_samples: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let mut driver = FileReader::new(file, Mode::Strict, Limits::new())?;
    let mut frame = Vec::new();
    let mut samples = 0u64;
    loop {
        match driver.step()? {
            DriverStep::Info { info, .. } => {
                println!(
                    "info timestamp_scale={}ns duration={:?}",
                    info.timestamp_scale_ns, info.duration
                );
            }
            DriverStep::Tracks { tracks, .. } => {
                for track in tracks {
                    println!(
                        "track {} codec={} type={}",
                        track.track_number(),
                        track.codec_id(),
                        track.track_type_value()
                    );
                }
            }
            DriverStep::Block(block) => {
                driver.read_frame(&block, &mut frame)?;
                samples += 1;
                println!(
                    "block track={} tick={} ns={} lace={}/{} keyframe={} bytes={}",
                    block.track,
                    block.timestamp,
                    block.timestamp_ns,
                    block.lace_index,
                    block.lace_count,
                    block.keyframe,
                    frame.len()
                );
            }
            DriverStep::Cues(cues) => println!("cues entries={}", cues.len()),
            DriverStep::Chapters(element) => println!("chapters at {}", element.range.offset),
            DriverStep::Tags(element) => println!("tags at {}", element.range.offset),
            DriverStep::Attachments(element) => {
                println!("attachments at {}", element.range.offset);
            }
            DriverStep::Warning(warning) => println!("warning {warning}"),
            DriverStep::Done => break,
            _ => return Err("unexpected driver step".into()),
        }
    }
    println!("samples={samples}");
    Ok(())
}
