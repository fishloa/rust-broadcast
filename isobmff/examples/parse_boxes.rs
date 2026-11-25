//! Walk the top-level boxes of a file and print each one's four-CC and size.
//!
//! ```bash
//! cargo run -p isobmff --example parse_boxes -- fixtures/mp4/h264-aac-prog.mp4
//! ```

use std::env;
use std::path::Path;

use broadcast_common::bounded::{Limit, read_file_bounded};

/// A whole box-only file this example walks top level by top level. It holds
/// samples too (they are skipped over, not decoded), so 64 MiB allows a real
/// short clip while still bounding the buffer.
const BOX_FILE_LIMIT: Limit = Limit::new("box file", 64 * 1024 * 1024);

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: parse_boxes <FILE>");
        std::process::exit(2);
    });
    let bytes = read_file_bounded(Path::new(&path), BOX_FILE_LIMIT).expect("read the input file");
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
