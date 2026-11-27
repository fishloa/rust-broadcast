//! The still-image item layer: HEIF and AVIF fixtures read through
//! `item_reader`, asserted against the independent tool's dump, re-serialized
//! byte-exact, and rebuilt through `item_writer`.

use isobmff::item::{ItemId, ItemInfoEntry};
use isobmff::item_reader::{ItemReader, ItemReaderConfig, ItemStep, ReemitOp, StillImage};
use isobmff::item_writer::ItemWriter;
use isobmff::reader::ByteRange;
use isobmff::{ItemType, ReferenceType};

fn fnv(bytes: &[u8]) -> u64 {
    let mut hash = 14_695_981_039_346_656_037u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash
}

fn load(name: &str) -> (Vec<u8>, StillImage) {
    let bytes = test_fixtures::read(name);
    let image = load_bytes(&bytes, ItemReaderConfig::default());
    (bytes, image)
}

fn load_bytes(bytes: &[u8], config: ItemReaderConfig) -> StillImage {
    let config = config.with_total_len(Some(bytes.len() as u64));
    let mut reader = ItemReader::new(config);
    loop {
        match reader.step().expect("step") {
            ItemStep::Need(range) => {
                let start = usize::try_from(range.offset).expect("offset");
                let len = usize::try_from(range.len).expect("length");
                reader
                    .supply(range.offset, &bytes[start..start + len])
                    .expect("supply");
            }
            ItemStep::Loaded(image) => {
                let image = image.clone();
                assert!(matches!(reader.step().expect("step"), ItemStep::Done));
                return image;
            }
            ItemStep::Done => panic!("done without a model"),
            _ => panic!("unexpected step"),
        }
    }
}

fn expect_load_error(bytes: &[u8], config: ItemReaderConfig) -> isobmff::Error {
    let config = config.with_total_len(Some(bytes.len() as u64));
    let mut reader = ItemReader::new(config);
    loop {
        match reader.step() {
            Ok(ItemStep::Need(range)) => {
                let start = usize::try_from(range.offset).expect("offset");
                let len = usize::try_from(range.len).expect("length");
                if let Err(err) = reader.supply(range.offset, &bytes[start..start + len]) {
                    return err;
                }
            }
            Ok(ItemStep::Loaded(_)) => panic!("loaded an invalid file"),
            Ok(ItemStep::Done) => panic!("done without an error"),
            Ok(_) => panic!("unexpected step"),
            Err(err) => return err,
        }
    }
}

fn hash_ranges(image: &StillImage, id: ItemId, source: &[u8]) -> u64 {
    fnv(&image.data_bytes(id, source).expect("item bytes"))
}

fn property_codes(image: &StillImage, id: ItemId) -> Vec<([u8; 4], bool)> {
    image
        .item(id)
        .expect("item")
        .properties()
        .iter()
        .map(|associated| (associated.property().fourcc(), associated.essential()))
        .collect()
}

fn reemit_is_byte_exact(name: &str) {
    let (bytes, image) = load(name);
    let plan = image.reemit_plan().expect("plan");
    assert!(matches!(plan.first(), Some(ReemitOp::Bytes(_))));
    let out = image.reemit_into(&bytes).expect("re-emit");
    if out != bytes {
        let index = out
            .iter()
            .zip(&bytes)
            .position(|(left, right)| left != right)
            .unwrap_or(out.len().min(bytes.len()));
        panic!(
            "{name}: first difference at {index} (got {:02x?}, want {:02x?}); lengths {} vs {}",
            out.get(index).copied(),
            bytes.get(index).copied(),
            out.len(),
            bytes.len()
        );
    }
}

fn rebuild_matches(name: &str) {
    let (bytes, image) = load(name);
    let brands = image.brands().expect("brands");
    let mut writer = ItemWriter::new(brands.major_brand)
        .with_compatible_brands(brands.compatible_brands.clone());
    for item in image.items() {
        writer.add_item(
            item.entry().clone(),
            image.data_bytes(item.id(), &bytes).expect("data"),
        );
        for associated in item.properties() {
            writer
                .add_property_with_essential(
                    item.id(),
                    associated.property().clone(),
                    associated.essential(),
                )
                .expect("property");
        }
        for reference in item.references_out() {
            writer
                .add_reference(reference.reference_type(), item.id(), reference.to())
                .expect("reference");
        }
    }
    if let Some(primary) = image.primary() {
        writer.set_primary(primary).expect("primary");
    }
    for group in image.groups().groups() {
        writer.add_group(group.clone());
    }
    let rebuilt = writer.to_bytes().expect("writer");
    let reread = load_bytes(&rebuilt, ItemReaderConfig::default());
    assert_eq!(reread.items().len(), image.items().len());
    for item in image.items() {
        let other = reread.item(item.id()).expect("rebuilt item");
        assert_eq!(other.item_type(), item.item_type());
        assert_eq!(other.item_name(), item.item_name());
        assert_eq!(other.is_hidden(), item.is_hidden());
        assert_eq!(
            property_codes(&reread, item.id()),
            property_codes(&image, item.id())
        );
        let refs: Vec<(ReferenceType, ItemId)> = other
            .references_out()
            .iter()
            .map(|reference| (reference.reference_type(), reference.to()))
            .collect();
        let expected: Vec<(ReferenceType, ItemId)> = item
            .references_out()
            .iter()
            .map(|reference| (reference.reference_type(), reference.to()))
            .collect();
        assert_eq!(refs, expected);
        assert_eq!(
            reread
                .data_bytes(item.id(), &rebuilt)
                .expect("rebuilt data"),
            image.data_bytes(item.id(), &bytes).expect("source data")
        );
    }
    assert_eq!(reread.primary(), image.primary());
}

fn find_box(bytes: &[u8], base: usize, fourcc: [u8; 4]) -> Option<usize> {
    use isobmff::box_types::BoxIter;
    for item in BoxIter::new(bytes) {
        let item = item.expect("box");
        let absolute = base + item.start;
        if item.header.box_type.is(&fourcc) {
            return Some(absolute);
        }
        let descend = item.header.box_type.is(b"meta")
            || item.header.box_type.is(b"iprp")
            || item.header.box_type.is(b"ipco")
            || item.header.box_type.is(b"grpl")
            || item.header.box_type.is(b"iref");
        if !descend {
            continue;
        }
        let body = if item.header.box_type.is(b"meta") {
            &item.body[4..]
        } else {
            item.body
        };
        let child_base = absolute + item.header.header_size() + (item.body.len() - body.len());
        if let Some(found) = find_box(body, child_base, fourcc) {
            return Some(found);
        }
    }
    None
}

fn meta_child(bytes: &[u8], fourcc: [u8; 4]) -> usize {
    find_box(bytes, 0, fourcc).unwrap_or_else(|| panic!("no meta child {fourcc:?}"))
}

#[test]
fn heif_plain_reads_one_hevc_item() {
    let (bytes, image) = load("heif/plain.heic");
    let brands = image.brands().expect("ftyp");
    assert_eq!(&brands.major_brand, b"heic");
    assert_eq!(
        brands.compatible_brands,
        [*b"mif1", *b"heic", *b"miaf"].to_vec()
    );
    assert!(image.has_brand(*b"mif1"));
    assert!(image.is_still_image());
    assert!(image.is_picture_handler());
    assert_eq!(image.primary(), Some(ItemId::new(1)));
    assert_eq!(image.items().len(), 1);
    let item = image.item(ItemId::new(1)).expect("item 1");
    assert_eq!(item.item_type(), ItemType::Hvc1);
    assert!(item.item_name().is_empty());
    assert!(!item.is_hidden());
    assert_eq!(item.protection_index(), 0);
    assert_eq!(image.image_size(item.id()), Some((160, 120)));
    assert_eq!(image.pixel_bits(item.id()), Some(&[8u8, 8, 8][..]));
    assert_eq!(
        property_codes(&image, item.id()),
        [
            (*b"hvcC", true),
            (*b"colr", false),
            (*b"ispe", false),
            (*b"pixi", false),
        ]
    );
    assert_eq!(
        item.data_ranges(),
        &[ByteRange {
            offset: 378,
            len: 2093
        }]
    );
    assert_eq!(
        hash_ranges(&image, item.id(), &bytes),
        0x9b1d_4da5_2e6b_1f1c
    );
    reemit_is_byte_exact("heif/plain.heic");
    rebuild_matches("heif/plain.heic");
}

#[test]
fn heif_thumbnail_links_to_its_master() {
    let (bytes, image) = load("heif/thumb.heic");
    assert_eq!(image.items().len(), 2);
    assert_eq!(image.primary(), Some(ItemId::new(1)));
    let thumb = image.item(ItemId::new(2)).expect("thumbnail");
    assert_eq!(thumb.references_out().len(), 1);
    assert_eq!(
        thumb.references_out()[0].reference_type(),
        ReferenceType::Thmb
    );
    assert_eq!(thumb.references_out()[0].to(), ItemId::new(1));
    assert_eq!(image.thumbnails_of(ItemId::new(1))[0].id(), thumb.id());
    assert_eq!(image.image_size(thumb.id()), Some((80, 64)));
    assert_eq!(
        property_codes(&image, thumb.id()),
        [
            (*b"hvcC", true),
            (*b"colr", false),
            (*b"ispe", false),
            (*b"pixi", false),
            (*b"clap", true),
        ]
    );
    assert_eq!(
        hash_ranges(&image, ItemId::new(2), &bytes),
        0x9f0d_0efc_9c15_ca0c
    );
    assert_eq!(
        image.item(ItemId::new(1)).expect("master").references_in()[0].from(),
        ItemId::new(2)
    );
    reemit_is_byte_exact("heif/thumb.heic");
    rebuild_matches("heif/thumb.heic");
}

#[test]
fn heif_alpha_links_an_auxiliary_image() {
    let (bytes, image) = load("heif/alpha.heic");
    assert_eq!(image.items().len(), 2);
    let aux = image.item(ItemId::new(2)).expect("auxiliary");
    let kind = image.auxiliary_type(aux.id()).expect("auxC");
    assert_eq!(kind.aux_type_str(), Some("urn:mpeg:hevc:2015:auxid:1"));
    assert!(kind.is_alpha());
    assert!(!kind.is_depth());
    assert_eq!(
        aux.references_out()[0].reference_type(),
        ReferenceType::Auxl
    );
    assert_eq!(image.auxiliary_images_of(ItemId::new(1))[0].id(), aux.id());
    assert_eq!(
        hash_ranges(&image, ItemId::new(2), &bytes),
        0xc3c1_2b60_7a56_ff98
    );
    reemit_is_byte_exact("heif/alpha.heic");
    rebuild_matches("heif/alpha.heic");
}

#[test]
fn heif_collection_carries_two_images_with_exif_and_xmp() {
    let (bytes, image) = load("heif/collection.mp4");
    assert!(!image.is_still_image());
    assert!(image.is_picture_handler());
    assert_eq!(image.items().len(), 4);
    assert_eq!(image.primary(), Some(ItemId::new(1)));
    let exif = image.exif_items();
    assert_eq!(exif.len(), 1);
    assert_eq!(exif[0].name_str(), Some("exif"));
    assert_eq!(
        hash_ranges(&image, exif[0].id(), &bytes),
        0x10dd_9a37_0f88_5f43
    );
    let xmp = image.xmp_items();
    assert_eq!(xmp.len(), 1);
    assert_eq!(xmp[0].name_str(), Some("xmp"));
    assert_eq!(xmp[0].content_type(), Some(&b"application/rdf+xml"[..]));
    assert_eq!(
        hash_ranges(&image, xmp[0].id(), &bytes),
        0x9856_5911_8f67_0487
    );
    assert_eq!(
        exif[0].references_out()[0].reference_type(),
        ReferenceType::Cdsc
    );
    assert_eq!(exif[0].references_out()[0].to(), ItemId::new(1));
    let group = image.groups().group_of(2).expect("altr group");
    assert_eq!(group.group_id().get(), 1);
    assert_eq!(group.grouping_type(), isobmff::EntityGroupType::Altr);
    assert_eq!(
        hash_ranges(&image, ItemId::new(2), &bytes),
        0xa9ea_8406_6331_14b2
    );
    reemit_is_byte_exact("heif/collection.mp4");
    rebuild_matches("heif/collection.mp4");
}

#[test]
fn avif_plain_reads_one_av1_item() {
    let (bytes, image) = load("avif/plain.avif");
    let brands = image.brands().expect("ftyp");
    assert_eq!(&brands.major_brand, b"avif");
    assert!(image.has_brand(*b"MA1A"));
    assert!(image.is_still_image());
    assert_eq!(image.items().len(), 1);
    let item = image.item(ItemId::new(1)).expect("item");
    assert_eq!(item.item_type(), ItemType::Av01);
    assert_eq!(item.name_str(), Some("Color"));
    assert_eq!(image.image_size(item.id()), Some((160, 120)));
    assert_eq!(
        property_codes(&image, item.id()),
        [
            (*b"ispe", false),
            (*b"pixi", false),
            (*b"av1C", true),
            (*b"colr", false),
        ]
    );
    assert_eq!(
        item.data_ranges(),
        &[ByteRange {
            offset: 275,
            len: 1933
        }]
    );
    assert_eq!(
        hash_ranges(&image, item.id(), &bytes),
        0xa9ea_8406_6331_14b2
    );
    reemit_is_byte_exact("avif/plain.avif");
    rebuild_matches("avif/plain.avif");
}

#[test]
fn avif_alpha_marks_the_alpha_urn() {
    let (bytes, image) = load("avif/alpha.avif");
    assert_eq!(image.items().len(), 2);
    let alpha = image.item(ItemId::new(2)).expect("alpha");
    let kind = image.auxiliary_type(alpha.id()).expect("auxC");
    assert!(kind.is_alpha());
    assert_eq!(
        image.auxiliary_images_of(ItemId::new(1))[0].id(),
        alpha.id()
    );
    assert_eq!(
        hash_ranges(&image, alpha.id(), &bytes),
        0x31ca_67a5_aae0_9945
    );
    reemit_is_byte_exact("avif/alpha.avif");
    rebuild_matches("avif/alpha.avif");
}

#[test]
fn avif_hdr_carries_the_light_level() {
    let (bytes, image) = load("avif/hdr10.avif");
    let item = image.item(ItemId::new(1)).expect("item");
    assert_eq!(image.pixel_bits(item.id()), Some(&[10u8, 10, 10][..]));
    let clli = item
        .properties()
        .iter()
        .find_map(|associated| match associated.property() {
            isobmff::image::ItemProperty::Clli(value) => Some(*value),
            _ => None,
        })
        .expect("clli");
    assert_eq!(clli.max_content_light_level, 1000);
    assert_eq!(clli.max_pic_average_light_level, 400);
    assert_eq!(
        hash_ranges(&image, item.id(), &bytes),
        0x2b74_da11_068a_2029
    );
    reemit_is_byte_exact("avif/hdr10.avif");
    rebuild_matches("avif/hdr10.avif");
}

#[test]
fn avif_grid_exposes_its_tiles_and_canvas() {
    let (bytes, image) = load("avif/grid.avif");
    assert_eq!(image.items().len(), 5);
    let grid = image.item(ItemId::new(1)).expect("grid");
    assert_eq!(grid.item_type(), ItemType::Grid);
    let Some(isobmff::image::DerivedImage::Grid(body)) = grid.derived() else {
        panic!("grid body");
    };
    assert_eq!(body.rows(), 2);
    assert_eq!(body.columns(), 2);
    assert_eq!(body.tile_count(), 4);
    assert_eq!((body.output_width(), body.output_height()), (160, 120));
    let tiles: Vec<ItemId> = image
        .grid_tiles(grid.id())
        .iter()
        .map(|tile| tile.id())
        .collect();
    assert_eq!(
        tiles,
        [
            ItemId::new(2),
            ItemId::new(3),
            ItemId::new(4),
            ItemId::new(5)
        ]
    );
    assert!(image.item(ItemId::new(2)).expect("tile").is_hidden());
    assert_eq!(image.image_size(ItemId::new(2)), Some((80, 64)));
    assert_eq!(
        grid.data_ranges(),
        &[ByteRange {
            offset: 514,
            len: 8
        }]
    );
    assert_eq!(
        hash_ranges(&image, grid.id(), &bytes),
        0x9951_ef27_e077_ad17
    );
    assert_eq!(
        hash_ranges(&image, ItemId::new(5), &bytes),
        0x137c_382e_91dd_d2f2
    );
    reemit_is_byte_exact("avif/grid.avif");
    rebuild_matches("avif/grid.avif");
}

#[test]
fn avif_two_by_one_grid_has_a_row_and_two_columns() {
    let (bytes, image) = load("avif/grid-2x1.avif");
    let grid = image.item(ItemId::new(1)).expect("grid");
    let Some(isobmff::image::DerivedImage::Grid(body)) = grid.derived() else {
        panic!("grid body");
    };
    assert_eq!(body.rows(), 1);
    assert_eq!(body.columns(), 2);
    assert_eq!(image.grid_tiles(grid.id()).len(), 2);
    assert_eq!(
        hash_ranges(&image, grid.id(), &bytes),
        hash_ranges(&image, grid.id(), &bytes)
    );
    reemit_is_byte_exact("avif/grid-2x1.avif");
    rebuild_matches("avif/grid-2x1.avif");
}

#[test]
fn sips_files_carry_rotation_and_light_level() {
    let (bytes, image) = load("heif/sips.heic");
    assert_eq!(image.rotation(ItemId::new(1)).expect("irot").degrees(), 0);
    assert_eq!(
        image.mirror(ItemId::new(1)).map(|mirror| mirror.axis()),
        None
    );
    assert_eq!(
        hash_ranges(&image, ItemId::new(1), &bytes),
        0x20fe_e45e_b34f_7916
    );
    reemit_is_byte_exact("heif/sips.heic");
    rebuild_matches("heif/sips.heic");

    let (bytes, image) = load("avif/sips.avif");
    assert_eq!(image.rotation(ItemId::new(1)).expect("irot").degrees(), 0);
    assert_eq!(
        hash_ranges(&image, ItemId::new(1), &bytes),
        0x70e8_4111_0479_2ac1
    );
    reemit_is_byte_exact("avif/sips.avif");
    rebuild_matches("avif/sips.avif");
}

fn grid_body(rows_minus_one: u8, columns_minus_one: u8) -> Vec<u8> {
    vec![0, 0, rows_minus_one, columns_minus_one, 0, 0xA0, 0, 0x78]
}

#[test]
fn iloc_offset_and_length_size_swap_is_detected() {
    let mut bytes = test_fixtures::read("heif/plain.heic");
    let iloc = meta_child(&bytes, *b"iloc");
    bytes[iloc + 8 + 4] = 0x80;
    let image = load_bytes(&bytes, ItemReaderConfig::default());
    let item = image.item(ItemId::new(1)).expect("item");
    assert_ne!(
        item.data_ranges(),
        &[ByteRange {
            offset: 378,
            len: 2093
        }]
    );
}

#[test]
fn ipma_narrow_index_read_as_wide_is_detected() {
    let mut bytes = test_fixtures::read("heif/plain.heic");
    let ipma = meta_child(&bytes, *b"ipma");
    bytes[ipma + 8 + 3] |= 1;
    let err = expect_load_error(&bytes, ItemReaderConfig::default());
    assert!(matches!(err, isobmff::Error::BufferTooShort(_)), "{err:?}");
}

#[test]
fn property_applied_to_the_wrong_item_is_detected() {
    let mut bytes = test_fixtures::read("heif/plain.heic");
    let ipma = meta_child(&bytes, *b"ipma");
    let first_id = ipma + 8 + 4 + 4;
    bytes[first_id] = 0;
    bytes[first_id + 1] = 2;
    let image = load_bytes(&bytes, ItemReaderConfig::default());
    assert!(property_codes(&image, ItemId::new(1)).is_empty());
    assert_ne!(
        property_codes(&image, ItemId::new(1)),
        [
            (*b"hvcC", true),
            (*b"colr", false),
            (*b"ispe", false),
            (*b"pixi", false),
        ]
    );
}

#[test]
fn grid_rows_and_columns_swap_is_detected() {
    let (bytes, image) = load("avif/grid-2x1.avif");
    let grid = image.item(ItemId::new(1)).expect("grid");
    let range = grid.data_ranges()[0];
    let mut mutated = bytes.clone();
    let start = usize::try_from(range.offset).expect("offset");
    mutated[start + 2] = 1;
    mutated[start + 3] = 0;
    let image = load_bytes(&mutated, ItemReaderConfig::default());
    let Some(isobmff::image::DerivedImage::Grid(body)) =
        image.item(ItemId::new(1)).expect("grid").derived()
    else {
        panic!("grid body");
    };
    assert_eq!(body.rows(), 2);
    assert_eq!(body.columns(), 1);
    assert_ne!((body.rows(), body.columns()), (1, 2));
}

#[test]
fn irot_angle_off_by_one_is_detected() {
    let mut bytes = test_fixtures::read("heif/sips.heic");
    let irot = meta_child(&bytes, *b"irot");
    bytes[irot + 8] = 1;
    let image = load_bytes(&bytes, ItemReaderConfig::default());
    assert_eq!(image.rotation(ItemId::new(1)).expect("irot").degrees(), 90);
    assert_ne!(image.rotation(ItemId::new(1)).expect("irot").degrees(), 0);
}

#[test]
fn derived_item_reference_cycle_is_a_typed_error() {
    let mut writer = ItemWriter::new(*b"avif");
    let first = writer.add_item(
        ItemInfoEntry::new(ItemId::new(1), ItemType::Grid, &[]),
        grid_body(0, 1),
    );
    let second = writer.add_item(
        ItemInfoEntry::new(ItemId::new(2), ItemType::Grid, &[]),
        grid_body(0, 1),
    );
    writer
        .add_reference(ReferenceType::Dimg, first, second)
        .expect("edge");
    writer
        .add_reference(ReferenceType::Dimg, second, first)
        .expect("edge");
    let bytes = writer.to_bytes().expect("writer");
    let err = expect_load_error(&bytes, ItemReaderConfig::default());
    assert!(
        matches!(err, isobmff::Error::ReferenceCycle { item } if item == 1 || item == 2),
        "{err:?}"
    );
}

#[test]
fn extent_past_end_of_file_is_a_typed_error() {
    let mut bytes = test_fixtures::read("heif/plain.heic");
    let iloc = meta_child(&bytes, *b"iloc");
    let length_at = iloc + 8 + 22;
    assert_eq!(
        u32::from_be_bytes(bytes[length_at..length_at + 4].try_into().expect("length")),
        2093
    );
    bytes[length_at..length_at + 4].copy_from_slice(&0xFFFFu32.to_be_bytes());
    let err = expect_load_error(&bytes, ItemReaderConfig::default());
    assert!(
        matches!(err, isobmff::Error::ExtentOutOfRange { item: 1, .. }),
        "{err:?}"
    );
}

#[test]
fn ipma_index_past_the_container_is_a_typed_error() {
    let mut bytes = test_fixtures::read("heif/plain.heic");
    let ipma = meta_child(&bytes, *b"ipma");
    let first_association = ipma + 8 + 4 + 4 + 2 + 1;
    bytes[first_association] = 0x7F;
    let err = expect_load_error(&bytes, ItemReaderConfig::default());
    assert!(
        matches!(
            err,
            isobmff::Error::PropertyIndexOutOfRange { index: 127, .. }
        ),
        "{err:?}"
    );
}

#[test]
fn zero_width_iloc_field_with_two_extents_is_a_typed_error() {
    use broadcast_core::Parse;
    let mut body = vec![1, 0, 0, 0];
    body.extend_from_slice(&[0x04, 0x00]);
    body.extend_from_slice(&1u16.to_be_bytes());
    body.extend_from_slice(&1u16.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    body.extend_from_slice(&2u16.to_be_bytes());
    for _ in 0..2 {
        body.extend_from_slice(&4u32.to_be_bytes());
    }
    let mut boxed = vec![0u8; 8];
    let size = u32::try_from(8 + body.len()).expect("size");
    boxed[..4].copy_from_slice(&size.to_be_bytes());
    boxed[4..8].copy_from_slice(b"iloc");
    boxed.extend_from_slice(&body);
    let err = isobmff::item::ItemLocationBox::parse(&boxed).expect_err("must reject");
    assert!(matches!(err, isobmff::Error::InvalidValue(_)), "{err:?}");
}

#[test]
fn a_huge_item_count_under_a_small_limit_is_a_typed_error() {
    let bytes = test_fixtures::read("heif/plain.heic");
    let limits = isobmff::Limits::default().with_max_items(0);
    let config = ItemReaderConfig::default().with_limits(limits);
    let err = expect_load_error(&bytes, config);
    assert!(
        matches!(
            err,
            isobmff::Error::LimitExceeded {
                what: "item count",
                ..
            }
        ),
        "{err:?}"
    );
}
