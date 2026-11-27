//! Print a still image's third-edition facts: its typed brands and findings,
//! every tone-map pair, its entity groups with typed bodies, and each item's
//! capture information.
//!
//! ```bash
//! cargo run -p isobmff --example heif_2025 -- fixtures/heif/groups.mp4
//! ```

use std::path::Path;

use broadcast_common::bounded::{Limit, read_file_bounded};
use isobmff::image_groups::ImageGroupBody;
use isobmff::item_reader::{ItemReader, ItemReaderConfig, ItemStep, StillImage};

const FILE_LIMIT: Limit = Limit::new("still image", 16 * 1024 * 1024);

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: heif_2025 <FILE>");
        std::process::exit(2);
    });
    let bytes = read_file_bounded(Path::new(&path), FILE_LIMIT).expect("read the input file");
    let image = load(&bytes);
    if let Some(brands) = image.image_brands() {
        println!(
            "major brand: {} ({})",
            brands.major_brand().name(),
            if brands.major_brand().is_sequence_brand() {
                "image sequence"
            } else {
                "items"
            }
        );
        for finding in brands.findings() {
            println!("brand finding: {}", finding.name());
        }
    }
    for finding in image.brand_findings() {
        println!("brand finding: {}", finding.name());
    }
    for item in image.items() {
        if let Some((base, gain_map)) = image.tone_map_inputs(item.id()) {
            println!(
                "tone map {}: base item {}, gain map item {}",
                item.id().get(),
                base.id().get(),
                gain_map.id().get()
            );
        }
        let capture = image.capture_info(item.id());
        if !capture.is_empty() {
            println!("item {} capture information:", item.id().get());
            for associated in capture {
                println!(
                    "  {}",
                    String::from_utf8_lossy(&associated.property().fourcc())
                );
            }
        }
        if let Some(region) = item.region() {
            println!(
                "region item {}: {} region(s) in {}x{}",
                item.id().get(),
                region.regions().len(),
                region.reference_width(),
                region.reference_height()
            );
        }
    }
    for group in image.groups().groups() {
        println!(
            "group {} {}: {:?}",
            group.group_id().get(),
            group.grouping_type().name(),
            group.entity_ids()
        );
        match group.image_body() {
            Ok(ImageGroupBody::ImagePyramid(pyramid)) => {
                println!("  tile sizes: {:?}", pyramid.tile_sizes());
            }
            Ok(ImageGroupBody::RegionPartition(partition)) => {
                println!("  area: {:?}", partition.area());
            }
            Ok(ImageGroupBody::EntitiesOnly) => println!("  body: entities only"),
            Ok(ImageGroupBody::Opaque(_)) => println!("  body: opaque"),
            Ok(_) => println!("  body: typed"),
            Err(err) => println!("  body: invalid ({err})"),
        }
    }
    for finding in image.structure_findings() {
        println!("finding: {}", finding.name());
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
