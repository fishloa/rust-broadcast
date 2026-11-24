//! Read a file's metadata, set its title, and write the new `udta` box bytes.
//!
//! ```bash
//! cargo run -p isobmff --example write_tags -- INPUT.mp4 OUTPUT.udta
//! ```

use std::env;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

use isobmff::metadata::{ItemKey, Metadata, MetadataItem, TextLocale};
use isobmff::{Reader, ReaderConfig, Step};

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
    let metadata = read_metadata(&input);
    let mut metadata = metadata.unwrap_or_default();
    metadata
        .items
        .retain(|item| item.key() != Some(&ItemKey::FourCc(*b"\xa9nam")));
    metadata.items.push(MetadataItem::Text {
        key: ItemKey::FourCc(*b"\xa9nam"),
        value: "Written by isobmff".into(),
        locale: TextLocale::Itunes {
            data_type: isobmff::metadata::ilst::DATA_TYPE_UTF8,
            country: 0,
            language: 0,
        },
    });
    let bytes = metadata.to_udta_bytes().expect("metadata writes");
    File::create(&output)
        .and_then(|mut file| file.write_all(&bytes))
        .expect("write the output");
    println!("wrote {} bytes to {output}", bytes.len());
}

fn read_metadata(path: &str) -> Option<Metadata> {
    let mut file = File::open(path).expect("open the input file");
    let len = file.metadata().expect("stat the input file").len();
    let mut reader = Reader::new(ReaderConfig::default().with_total_len(Some(len)));
    loop {
        match reader.step().expect("reader step") {
            Step::Need(range) => {
                let size = usize::try_from(range.len).expect("range length");
                let mut buf = vec![0u8; size];
                file.seek(SeekFrom::Start(range.offset)).expect("seek");
                let read = file.read(&mut buf).expect("read");
                reader.supply(range.offset, &buf[..read]).expect("supply");
            }
            Step::Metadata(found) => return Some(found.movie.clone()),
            Step::Done => return None,
            _ => {}
        }
    }
}
