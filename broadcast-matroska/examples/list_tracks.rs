//! List the tracks of a Matroska or WebM file.
//!
//! ```text
//! cargo run -p broadcast-matroska --example list_tracks -- fixtures/mkv/vp9-opus.webm
//! ```

use std::process::ExitCode;

use broadcast_common::Parse;

use broadcast_matroska::matroska::Document;

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

fn run(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    let document = Document::parse(&bytes)?;
    let segment = document.segment();
    println!("segment timestamp_scale={}ns", segment.timestamp_scale());
    let Some(tracks) = segment.tracks() else {
        println!("no Tracks element");
        return Ok(());
    };
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
    Ok(())
}
