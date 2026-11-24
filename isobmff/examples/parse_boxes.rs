//! Walk the top-level boxes of a file and print each one's four-CC and size.
//!
//! ```bash
//! cargo run -p isobmff --example parse_boxes -- fixtures/mp4/h264-aac-prog.mp4
//! ```

use std::env;
use std::fs;

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: parse_boxes <FILE>");
        std::process::exit(2);
    });
    let bytes = fs::read(&path).expect("read the input file");
    for step in isobmff::BoxIter::new(&bytes) {
        let boxref = match step {
            Ok(b) => b,
            Err(e) => {
                eprintln!("stop: {e}");
                break;
            }
        };
        println!(
            "{} size {} body {}",
            boxref.header.box_type,
            boxref.header.size,
            boxref.body.len()
        );
    }
}
