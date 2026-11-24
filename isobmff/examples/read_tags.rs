//! Print every metadata item of an MP4/MOV file, reading only box structure.
//!
//! ```bash
//! cargo run -p isobmff --example read_tags -- fixtures/mp4/progressive/av-prog.mp4
//! ```

use std::env;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use isobmff::metadata::{MetadataItem, MovieMetadata};
use isobmff::{Reader, ReaderConfig, Step};

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: read_tags <FILE>");
        std::process::exit(2);
    });
    let mut file = File::open(&path).expect("open the input file");
    let len = file.metadata().expect("stat the input file").len();
    let mut reader = Reader::new(ReaderConfig::default().with_total_len(Some(len)));
    let mut metadata = None;
    loop {
        match reader.step().expect("reader step") {
            Step::Need(range) => {
                let size = usize::try_from(range.len).expect("range length");
                let mut buf = vec![0u8; size];
                file.seek(SeekFrom::Start(range.offset)).expect("seek");
                let read = file.read(&mut buf).expect("read");
                reader.supply(range.offset, &buf[..read]).expect("supply");
            }
            Step::Metadata(found) => {
                metadata = Some(found.clone());
                break;
            }
            Step::Done => break,
            _ => {}
        }
    }
    match metadata {
        Some(metadata) => print(&metadata),
        None => println!("no metadata"),
    }
}

fn print(metadata: &MovieMetadata) {
    for item in &metadata.movie.items {
        println!("movie: {}", describe(item));
    }
    for (track, metadata) in &metadata.tracks {
        for item in &metadata.items {
            println!("track {track}: {}", describe(item));
        }
    }
}

fn describe(item: &MetadataItem) -> String {
    match item {
        MetadataItem::Text { key, value, .. } => format!("{key:?} = {value:?}"),
        MetadataItem::Number { key, value, .. } => format!("{key:?} = {value}"),
        MetadataItem::Pair { key, index, total } => {
            format!("{key:?} = {index}/{total}")
        }
        MetadataItem::Binary {
            key,
            data_type,
            bytes,
            ..
        } => {
            format!("{key:?} = {} bytes of type {data_type}", bytes.len())
        }
        MetadataItem::Freeform { mean, name, values } => {
            format!("{mean}/{name} = {} value(s)", values.len())
        }
        MetadataItem::Location(location) => format!("location = {}", location.as_str()),
        MetadataItem::Loci(loci) => format!(
            "location = {:.4},{:.4}",
            loci.latitude_degrees(),
            loci.longitude_degrees()
        ),
        MetadataItem::Chapters(chapters) => format!("{} chapter(s)", chapters.len()),
        MetadataItem::Opaque(boxed) => {
            format!("{:?} = {} opaque bytes", boxed.box_type, boxed.data.len())
        }
        _ => "unknown item".into(),
    }
}
