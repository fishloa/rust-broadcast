//! Read a file's metadata, set its title, and write the new `udta` box bytes.
//!
//! ```bash
//! cargo run -p isobmff --example write_tags -- INPUT.mp4 OUTPUT.udta
//! ```

use std::env;
use std::fs::File;
use std::io::Write;

use isobmff::ReaderConfig;
use isobmff::driver::{DriverStep, FileReader};
use isobmff::metadata::{ItemKey, Metadata};

fn main() {
    let mut args = env::args().skip(1);
    let input = args.next().unwrap_or_else(|| {
        eprintln!("usage: write_tags <INPUT> <OUTPUT>");
        std::process::exit(2);
    });
    let output = args.next().unwrap_or_else(|| {
        eprintln!("usage: write_tags <INPUT> <OUTPUT>");
        std::process::exit(2);
    });
    let mut metadata = read_metadata(&input).unwrap_or_default();
    metadata.set_text(&ItemKey::TITLE, "Written by isobmff");
    let bytes = metadata.to_udta_bytes().expect("metadata writes");
    File::create(&output)
        .and_then(|mut file| file.write_all(&bytes))
        .expect("write the output");
    println!("wrote {} bytes to {output}", bytes.len());
}

fn read_metadata(path: &str) -> Option<Metadata> {
    let file = File::open(path).expect("open the input file");
    let mut reader = FileReader::new(file, ReaderConfig::default()).expect("wrap the input file");
    loop {
        match reader.step().expect("driver step") {
            DriverStep::Metadata(found) => return Some(found.movie.clone()),
            DriverStep::Done => return None,
            _ => {}
        }
    }
}
