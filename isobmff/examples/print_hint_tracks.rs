//! Print a file's hint tracks: entry format, RTP timescale, maximum packet
//! size, the signalled payload type and the hint statistics, when the file
//! carries them.
//!
//! ```sh
//! cargo run -p isobmff --example print_hint_tracks -- some.mp4
//! ```

use std::fs::File;

use isobmff::ReaderConfig;
use isobmff::driver::{DriverStep, FileReader};
use isobmff::hint::{HintSampleEntry, HintStatisticsChild};

fn fourcc(bytes: [u8; 4]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '.'
            }
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: print_hint_tracks <FILE>");
        std::process::exit(2);
    };
    let mut reader = FileReader::new(File::open(&path)?, ReaderConfig::default())?;
    let mut found = 0usize;
    loop {
        match reader.step()? {
            DriverStep::Tracks(tracks) => {
                for track in tracks {
                    let mut hint_entries = track.entries.iter().filter_map(|entry| entry.hint());
                    let mut printed_header = false;
                    for hint in hint_entries.by_ref() {
                        if !printed_header {
                            println!(
                                "track {} handler={} timescale={}",
                                track.track_id.value(),
                                fourcc(track.handler),
                                track.timescale.get()
                            );
                            printed_header = true;
                        }
                        hint_line(hint, track.hint_statistics.as_ref());
                        found += 1;
                    }
                    if let Some(statistics) = &track.hint_statistics {
                        for child in statistics.children() {
                            if let HintStatisticsChild::Totl(box_) = child {
                                println!("  totl bytes_sent={}", box_.bytes_sent());
                            }
                            if let HintStatisticsChild::Npck(box_) = child {
                                println!("  npck packets_sent={}", box_.packets_sent());
                            }
                        }
                    }
                }
            }
            DriverStep::Done => break,
            _ => {}
        }
    }
    if found == 0 {
        println!("no hint tracks");
    }
    Ok(())
}

fn hint_line(hint: &HintSampleEntry, statistics: Option<&isobmff::hint::HintStatisticsBox>) {
    let payload_type = statistics
        .and_then(|stats| {
            stats.children().iter().find_map(|child| match child {
                HintStatisticsChild::Payt(box_) => Some(box_.payload_id()),
                _ => None,
            })
        })
        .map_or_else(|| "none".to_string(), |id| id.to_string());
    match hint {
        HintSampleEntry::Rtp(entry) => println!(
            "  {} max_packet_size={} timescale={} payload_type={payload_type}",
            fourcc(entry.format()),
            entry.max_packet_size(),
            entry
                .timescale()
                .map_or_else(|| "none".to_string(), |t| t.to_string())
        ),
        HintSampleEntry::Mpeg2Ts(entry) => println!(
            "  {} preceding_bytes={} trailing_bytes={} precomputed_only={}",
            fourcc(entry.format()),
            entry.preceding_bytes_len(),
            entry.trailing_bytes_len(),
            entry.precomputed_only_flag()
        ),
        HintSampleEntry::Fd(entry) => println!(
            "  {} partition_entry_id={} fec_overhead=0x{:04X} timescale={}",
            fourcc(entry.format()),
            entry.partition_entry_id(),
            entry.fec_overhead(),
            entry
                .timescale()
                .map_or_else(|| "none".to_string(), |t| t.to_string())
        ),
        _ => println!("  unknown hint entry"),
    }
}
