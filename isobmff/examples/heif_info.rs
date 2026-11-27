//! Print the items, image sizes, properties and references of a HEIF/AVIF
//! still image.
//!
//! ```bash
//! cargo run -p isobmff --example heif_info -- fixtures/heif/thumb.heic
//! ```

use std::path::Path;

use broadcast_common::bounded::{Limit, read_file_bounded};
use isobmff::item::ItemId;
use isobmff::item_reader::{ItemReader, ItemReaderConfig, ItemStep, StillImage};

const FILE_LIMIT: Limit = Limit::new("still image", 16 * 1024 * 1024);

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: heif_info <FILE>");
        std::process::exit(2);
    });
    let bytes = read_file_bounded(Path::new(&path), FILE_LIMIT).expect("read the input file");
    let image = load(&bytes);
    let brands = image.brands().expect("brands");
    println!(
        "brands: major {} compatible {}",
        fourcc(brands.major_brand),
        brands
            .compatible_brands
            .iter()
            .map(|brand| fourcc(*brand))
            .collect::<Vec<_>>()
            .join(" ")
    );
    if let Some(handler) = image.handler_type() {
        println!("handler: {}", fourcc(handler));
    }
    match image.primary() {
        Some(primary) => println!("primary item: {}", primary.get()),
        None => println!("primary item: none"),
    }
    for item in image.items() {
        println!(
            "item {} type {} name {:?}{}",
            item.id().get(),
            item.item_type().name(),
            item.name_str().unwrap_or(""),
            if item.is_hidden() { " hidden" } else { "" }
        );
        if let Some((width, height)) = image.image_size(item.id()) {
            println!("  image: {width}x{height}");
        }
        if let Some(bits) = image.pixel_bits(item.id()) {
            println!("  bits per channel: {bits:?}");
        }
        if let Some(rotation) = image.rotation(item.id()) {
            println!("  rotation: {} degrees", rotation.degrees());
        }
        if let Some(mirror) = image.mirror(item.id()) {
            println!("  mirror: {}", mirror.axis().name());
        }
        for associated in item.properties() {
            println!(
                "  property {} at index {} (essential {})",
                fourcc(associated.property().fourcc()),
                associated.index().get(),
                associated.essential()
            );
        }
        for reference in item.references_out() {
            println!(
                "  reference {} -> item {}",
                reference.reference_type().name(),
                reference.to().get()
            );
        }
        for range in item.data_ranges() {
            println!("  data: offset {} length {}", range.offset, range.len);
        }
        if let Some(isobmff::image::DerivedImage::Grid(grid)) = item.derived() {
            println!(
                "  grid: {} rows x {} columns, canvas {}x{}",
                grid.rows(),
                grid.columns(),
                grid.output_width(),
                grid.output_height()
            );
        }
        if let Some(isobmff::image::DerivedImage::Overlay(overlay)) = item.derived() {
            println!(
                "  overlay: {} layers, canvas {}x{}",
                overlay.offsets().len(),
                overlay.output_width(),
                overlay.output_height()
            );
        }
    }
    for thumbnail in image.thumbnails_of(ItemId::new(1)) {
        println!("thumbnail of item 1: item {}", thumbnail.id().get());
    }
    for auxiliary in image.auxiliary_images_of(ItemId::new(1)) {
        let kind = image
            .auxiliary_type(auxiliary.id())
            .and_then(|aux| aux.aux_type_str())
            .unwrap_or("");
        println!(
            "auxiliary of item 1: item {} ({kind})",
            auxiliary.id().get()
        );
    }
    println!("exif items: {}", image.exif_items().len());
    println!("xmp items: {}", image.xmp_items().len());
    for group in image.groups().groups() {
        println!(
            "group {} of type {}: {:?}",
            group.group_id().get(),
            group.grouping_type().name(),
            group.entity_ids()
        );
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

fn fourcc(bytes: [u8; 4]) -> String {
    String::from_utf8_lossy(&bytes).into_owned()
}
