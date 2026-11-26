//! Print a file's audio tracks with their language, kind, channel layout and
//! loudness, as the `Reader` surfaces them.
//!
//! ```bash
//! cargo run -p isobmff --example print_audio_tracks -- fixtures/mp4/boxes/track-description.mp4
//! ```

use std::env;
use std::fs::File;

use isobmff::ReaderConfig;
use isobmff::driver::{DriverStep, FileReader};

const AUDIO_HANDLER: [u8; 4] = *b"soun";

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: print_audio_tracks <FILE>");
        std::process::exit(2);
    });
    let file = File::open(&path).expect("open the input file");
    let mut reader = FileReader::new(file, ReaderConfig::default()).expect("reader");
    loop {
        match reader.step().expect("read a step") {
            DriverStep::Tracks(tracks) => {
                for track in tracks {
                    if track.handler != AUDIO_HANDLER {
                        continue;
                    }
                    println!(
                        "track {} ({})",
                        track.track_id.value(),
                        track.timescale.get()
                    );
                    if let Some(language) = &track.extended_language {
                        println!("  language: {}", language.language_str().unwrap_or("?"));
                    }
                    for kind in &track.kinds {
                        println!(
                            "  kind: {} = {}",
                            kind.scheme_uri_str().unwrap_or("?"),
                            kind.value_str().unwrap_or("")
                        );
                    }
                    for entry in &track.entries {
                        if let Some(layout) = entry.channel_layout() {
                            println!(
                                "  channels: defined_layout={:?} positions={}",
                                layout.defined_layout(),
                                layout.speaker_positions().len()
                            );
                        }
                    }
                    if let Some(loudness) = &track.loudness {
                        for track_loudness in loudness.track() {
                            for entry in track_loudness.base().entries() {
                                println!(
                                    "  loudness: sample_peak={} true_peak={} measurements={}",
                                    entry.sample_peak_level(),
                                    entry.true_peak_level(),
                                    entry.measurements().len()
                                );
                            }
                        }
                    }
                }
            }
            DriverStep::Done => break,
            _ => {}
        }
    }
}
