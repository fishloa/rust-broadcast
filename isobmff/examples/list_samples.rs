//! Print every track of an MP4 and its sample count.
//!
//! ```bash
//! cargo run -p isobmff --example list_samples -- fixtures/mp4/progressive/av-prog.mp4
//! ```

use std::collections::BTreeMap;
use std::env;
use std::fs::File;

use isobmff::driver::{DriverStep, FileReader};
use isobmff::{ReaderConfig, SampleOrder, TrackId};

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: list_samples <FILE>");
        std::process::exit(2);
    });
    let file = File::open(&path).expect("open the input file");
    let config = ReaderConfig::default().with_order(SampleOrder::File);
    let mut reader = FileReader::new(file, config).expect("wrap the input file");
    let mut tracks: Vec<(TrackId, String, String)> = Vec::new();
    let mut counts: BTreeMap<TrackId, u32> = BTreeMap::new();
    loop {
        match reader.step() {
            Ok(DriverStep::Tracks(info)) => {
                for track in info {
                    let fourcc = track
                        .entries
                        .first()
                        .map(|entry| String::from_utf8_lossy(&entry.fourcc).into_owned())
                        .unwrap_or_default();
                    tracks.push((
                        track.track_id,
                        String::from_utf8_lossy(&track.handler).into_owned(),
                        fourcc,
                    ));
                }
            }
            Ok(DriverStep::Sample(sample)) => {
                *counts.entry(sample.track).or_insert(0) += 1;
            }
            Ok(DriverStep::Fragment(_)) => {}
            Ok(DriverStep::Done) => break,
            Ok(_) => {}
            Err(error) => {
                eprintln!("stop: {error}");
                std::process::exit(1);
            }
        }
    }
    for (track_id, handler, fourcc) in tracks {
        let count = counts.get(&track_id).copied().unwrap_or(0);
        println!(
            "track {} {handler} {fourcc} samples {count}",
            track_id.value()
        );
    }
}
