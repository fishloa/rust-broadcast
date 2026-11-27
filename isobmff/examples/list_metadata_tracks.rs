//! List a file's metadata and text tracks: each one's handler, sample-entry
//! type, metadata URI and the local keys of a boxed metadata entry.
//!
//! ```sh
//! cargo run -p isobmff --example list_metadata_tracks -- some.mp4
//! ```

use std::fs::File;

use isobmff::ReaderConfig;
use isobmff::driver::{DriverStep, FileReader};

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
        eprintln!("usage: list_metadata_tracks <FILE>");
        std::process::exit(2);
    };
    let mut reader = FileReader::new(File::open(&path)?, ReaderConfig::default())?;
    loop {
        match reader.step()? {
            DriverStep::Tracks(tracks) => {
                for track in tracks {
                    let handler = fourcc(track.handler);
                    let metadata_handlers = ["meta", "text", "subt"];
                    if !metadata_handlers.contains(&handler.as_str()) {
                        continue;
                    }
                    println!(
                        "track {} handler={handler} timescale={}",
                        track.track_id.value(),
                        track.timescale.get()
                    );
                    for entry in &track.entries {
                        match entry.timed_metadata() {
                            Some(metadata) => {
                                println!("  entry {}", fourcc(metadata.fourcc()));
                                if let Some(uri) = metadata.uri() {
                                    println!("    uri {}", uri.uri_str().unwrap_or("<invalid>"));
                                }
                                if let Some(init) = metadata.uri_init() {
                                    println!("    init data {} bytes", init.data().len());
                                }
                                if let Some(table) = metadata.key_table() {
                                    for key in table.keys() {
                                        let name = key
                                            .declaration()
                                            .and_then(|d| d.key_value_str())
                                            .unwrap_or("<no keyd>");
                                        println!("    key {} ({name})", key.local_key_id());
                                    }
                                }
                            }
                            None => println!("  entry {}", fourcc(entry.fourcc)),
                        }
                    }
                }
            }
            DriverStep::Done => break,
            _ => {}
        }
    }
    Ok(())
}
