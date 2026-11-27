//! Wrap an AV1 still image's coded payload from one AVIF file into a new AVIF
//! file, then re-read it.
//!
//! ```bash
//! cargo run -p isobmff --example build_avif -- fixtures/avif/plain.avif
//! ```

use std::path::Path;

use broadcast_common::bounded::{Limit, read_file_bounded};
use isobmff::ItemId;
use isobmff::image::ItemProperty;
use isobmff::item::ItemInfoEntry;
use isobmff::item_reader::{ItemReader, ItemReaderConfig, ItemStep, StillImage};
use isobmff::item_writer::ItemWriter;

const FILE_LIMIT: Limit = Limit::new("still image", 16 * 1024 * 1024);
const AVIF_BRANDS: [[u8; 4]; 3] = [*b"mif1", *b"miaf", *b"MA1A"];

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: build_avif <AVIF FILE> [OUTPUT]");
        std::process::exit(2);
    });
    let bytes = read_file_bounded(Path::new(&path), FILE_LIMIT).expect("read the input file");
    let source = load(&bytes);
    let primary = source.primary().unwrap_or_else(|| {
        eprintln!("{path}: the file names no primary item");
        std::process::exit(1);
    });
    let item = source.item(primary).expect("primary item");
    let payload = source.data_bytes(primary, &bytes).expect("item data");
    let mut writer = ItemWriter::new(*b"avif").with_compatible_brands(AVIF_BRANDS.to_vec());
    let new_id = writer.add_item(
        ItemInfoEntry::new(ItemId::new(1), item.item_type(), item.item_name()),
        payload,
    );
    for associated in item.properties() {
        if matches!(associated.property(), ItemProperty::Opaque(_)) {
            continue;
        }
        writer
            .add_property_with_essential(
                new_id,
                associated.property().clone(),
                associated.essential(),
            )
            .expect("property");
    }
    writer.set_primary(new_id).expect("primary item");
    let built = writer.to_bytes().expect("build the file");
    let reread = load(&built);
    let rebuilt_item = reread.item(new_id).expect("rebuilt item");
    let (width, height) = reread.image_size(new_id).unwrap_or((0, 0));
    println!(
        "built {} bytes: {} item(s), primary {}, image {}x{}, properties {}",
        built.len(),
        reread.items().len(),
        new_id.get(),
        width,
        height,
        rebuilt_item.properties().len()
    );
    if let Some(output) = std::env::args().nth(2) {
        std::fs::write(&output, &built).expect("write the output file");
        println!("wrote {output}");
    }
}

fn load(bytes: &[u8]) -> StillImage {
    let config = ItemReaderConfig::default().with_total_len(Some(bytes.len() as u64));
    let mut reader = ItemReader::new(config);
    loop {
        match reader.step().expect("read the file") {
            ItemStep::Need(range) => {
                let start = usize::try_from(range.offset).expect("offset");
                let len = usize::try_from(range.len).expect("length");
                reader
                    .supply(range.offset, &bytes[start..start + len])
                    .expect("supply the demanded range");
            }
            ItemStep::Loaded(image) => return image.clone(),
            ItemStep::Done => panic!("the reader finished without a model"),
            _ => panic!("unexpected reader step"),
        }
    }
}
