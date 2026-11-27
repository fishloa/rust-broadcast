//! The still-image item layer against ISO/IEC 23008-12:2025 and Amendment 1:
//! the real third-edition fixtures, clause-assembled structures no installed
//! tool writes, and the typed findings hostile input produces.

use broadcast_core::{Parse, Serialize};
use isobmff::ReferenceType;
use isobmff::image::{
    DerivedImage, GainMapChannel, GainMapField, GainMapMetadata, ItemProperty, SignedRational,
    UnsignedRational,
};
use isobmff::image_brands::{BrandFinding, ImageFinding};
use isobmff::image_derived::{ColourFormatEnhancement, ColourFormatInput, ToneMapImage};
use isobmff::image_groups::{
    ImageGroupBody, ImagePyramidEntityGroup, PYMD_TILE_INFO_CONSTANT, PYMD_TILE_INFO_PRESENT,
    RGPA_AREA_INFO_PRESENT, RegionPartitionArea, RegionPartitionGroup,
};
use isobmff::image_props::{
    AccessibilityTextProperty, AutoExposureProperty, CameraExtrinsicMatrixProperty,
    CameraIntrinsicMatrixProperty, CameraOrientation, ConstrainedExtentsGridProperty,
    CreationTimeProperty, DepthOfFieldProperty, DisparityAdjustmentProperty,
    FadeTransitionEffectProperty, FocusProperty, ImageScaling, MaskConfigurationProperty,
    ModificationTimeProperty, NominalDiffuseWhiteProperty, PanoramaDirection, PanoramaProperty,
    ProgressiveDerivedImageItemInformationProperty, ReferenceViewingEnvironmentProperty,
    RequiredReferenceTypesProperty, SingleStreamProperty, SplitTransitionEffectProperty,
    StereoAggressor, StereoAggressorType, StereoAggressorsProperty,
    SuggestedTimeDisplayDurationProperty, SuggestedTransitionPeriodProperty,
    TargetOutputLayerSetProperty, UserDescriptionProperty, WhiteBalanceProperty, WipeDirection,
    WipeTransitionEffectProperty, ZoomDirection, ZoomShape, ZoomTransitionEffectProperty,
};
use isobmff::image_sequence::{CodingConstraintsBox, DirectReferenceSamplesList};
use isobmff::init_segment::{SampleDescriptionBox, SampleEntryVariant};
use isobmff::integrity::{
    INPUT_TRAK, IntegrityBox, IntegrityItem, Md5IntegrityBox, SampleGroupDigest,
};
use isobmff::item::{EntityGroupType, GroupId, ItemId, ItemInfoEntry, ItemType};
use isobmff::item_reader::{ItemReader, ItemReaderConfig, ItemStep, StillImage};
use isobmff::item_writer::ItemWriter;
use isobmff::limits::Limits;
use isobmff::region::{
    AnnotationContainerType, RegionAnnotation, RegionAnnotationAssociation, RegionAnnotationMap,
    RegionGeometry, RegionItem, RegionSampleEntry, RegionTrackConfigBox,
    UserDescriptionSampleGroupEntry,
};
use isobmff::sample_groups::{
    GROUPING_TYPE_AEBR, GROUPING_TYPE_AFBR, GROUPING_TYPE_DOBR, GROUPING_TYPE_EQIV,
    GROUPING_TYPE_FOBR, GROUPING_TYPE_PANO, GROUPING_TYPE_REFS, GROUPING_TYPE_REGM,
    GROUPING_TYPE_UDES, GROUPING_TYPE_WBBR, SampleGroupDescriptionBox, SgpdEntry,
};

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

fn reemit_is_byte_exact(name: &str) {
    let (bytes, image) = load(name);
    let out = image.reemit_into(&bytes).expect("re-emit");
    assert_eq!(out, bytes, "{name} re-emit differs");
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
        assert_eq!(
            reread
                .data_bytes(item.id(), &rebuilt)
                .expect("rebuilt data"),
            image.data_bytes(item.id(), &bytes).expect("source data")
        );
    }
    assert_eq!(
        reread.groups().groups().len(),
        image.groups().groups().len()
    );
}

#[test]
fn props_fixture_reads_the_third_edition_properties() {
    let (_bytes, image) = load("heif/props.heic");
    let item = image.item(ItemId::new(1)).expect("primary item");
    let udes = item
        .associated_property(isobmff::image_props::UDES)
        .expect("udes")
        .property();
    match udes {
        ItemProperty::Udes(udes) => {
            assert_eq!(udes.description(), b"Golden hour sample");
            assert!(udes.lang().is_empty());
            assert!(udes.name_bytes().is_empty());
            assert!(udes.tags().is_empty());
        }
        other => panic!("expected udes, got {other:?}"),
    }
    assert!(item.associated_property(isobmff::image::PASP).is_none());
    assert!(
        item.associated_property(isobmff::hdr_boxes::CLLI_FOURCC)
            .is_some()
    );
    match item
        .associated_property(isobmff::image::IROT)
        .expect("irot")
        .property()
    {
        ItemProperty::Irot(rotation) => assert_eq!(rotation.degrees(), 270),
        other => panic!("expected irot, got {other:?}"),
    }
    match item
        .associated_property(isobmff::image::IMIR)
        .expect("imir")
        .property()
    {
        ItemProperty::Imir(mirror) => {
            assert_eq!(mirror.axis(), isobmff::image::ImageMirrorAxis::Horizontal)
        }
        other => panic!("expected imir, got {other:?}"),
    }
    assert!(image.brand_findings().is_empty());
    assert!(image.structure_findings().is_empty());
    reemit_is_byte_exact("heif/props.heic");
    rebuild_matches("heif/props.heic");
}

#[test]
fn tiled_fixture_reads_the_grid_body_and_tiles() {
    let (bytes, image) = load("heif/tiled.heic");
    let primary = image.primary().expect("primary");
    let item = image.item(primary).expect("grid item");
    match item.derived().expect("derived body") {
        DerivedImage::Grid(grid) => {
            assert_eq!(grid.rows(), 2);
            assert_eq!(grid.columns(), 2);
            assert_eq!(grid.tile_count(), 4);
            assert_eq!(grid.output_width(), 160);
            assert_eq!(grid.output_height(), 120);
        }
        other => panic!("expected grid, got {other:?}"),
    }
    assert_eq!(image.grid_tiles(primary).len(), 4);
    assert!(image.structure_findings().is_empty());
    reemit_is_byte_exact("heif/tiled.heic");
    rebuild_matches("heif/tiled.heic");
    let _ = bytes;
}

#[test]
fn premultiplied_fixture_carries_the_prem_reference() {
    let (_bytes, image) = load("heif/premult.heic");
    let item = image.item(ItemId::new(1)).expect("colour item");
    assert!(
        item.references_out()
            .iter()
            .any(|reference| reference.reference_type() == ReferenceType::Prem)
    );
    let aux = image.auxiliary_images_of(item.id());
    assert_eq!(aux.len(), 1);
    assert!(aux[0].associated_property(isobmff::image::AUXC).is_some());
    reemit_is_byte_exact("heif/premult.heic");
    rebuild_matches("heif/premult.heic");
}

#[test]
fn groups_fixture_reads_typed_entity_groups() {
    let (_bytes, image) = load("heif/groups.mp4");
    let groups = image.groups().groups();
    assert_eq!(groups.len(), 4);
    let kinds: Vec<EntityGroupType> = groups.iter().map(|group| group.grouping_type()).collect();
    assert_eq!(
        kinds,
        [
            EntityGroupType::Ster,
            EntityGroupType::Pymd,
            EntityGroupType::Slid,
            EntityGroupType::Prgr
        ]
    );
    assert_eq!(groups[0].group_id(), GroupId::new(1));
    assert_eq!(groups[0].entity_ids(), [10u32, 11]);
    assert_eq!(groups[1].group_id(), GroupId::new(2));
    assert_eq!(groups[1].entity_ids(), [12u32, 13]);
    assert_eq!(groups[2].entity_ids(), [14u32, 15]);
    assert_eq!(groups[3].entity_ids(), [16u32, 17]);
    assert!(matches!(
        groups[1].image_body().expect("typed body"),
        ImageGroupBody::ImagePyramid(_)
    ));
    let (left, right) = image.stereo_pair(GroupId::new(1)).expect("stereo pair");
    assert_eq!(left.id(), ItemId::new(10));
    assert_eq!(right.id(), ItemId::new(11));
    assert_eq!(image.group_entities(GroupId::new(2)).len(), 2);
    assert!(image.structure_findings().is_empty());
    reemit_is_byte_exact("heif/groups.mp4");
    rebuild_matches("heif/groups.mp4");
}

fn tone_map_file(inputs: [u32; 2]) -> Vec<u8> {
    let mut writer = ItemWriter::new(*b"heic").with_compatible_brands(vec![*b"mif1", *b"tmap"]);
    writer.add_item(
        ItemInfoEntry::new(ItemId::new(1), ItemType::Hvc1, b"base"),
        vec![1, 2, 3],
    );
    let gain = ItemInfoEntry::new(ItemId::new(2), ItemType::Hvc1, b"gain").with_hidden(true);
    writer.add_item(gain, vec![4, 5]);
    let tmap = DerivedImage::ToneMap(ToneMapImage::new(vec![0xde, 0xad, 0xbe, 0xef]));
    writer
        .add_derived_item(
            ItemInfoEntry::new(ItemId::new(3), ItemType::Tmap, b"tmap"),
            &tmap,
        )
        .expect("tmap");
    for input in inputs {
        writer
            .add_reference(ReferenceType::Dimg, ItemId::new(3), ItemId::new(input))
            .expect("dimg");
    }
    writer.set_primary(ItemId::new(3)).expect("primary");
    writer.add_group(isobmff::item::EntityToGroupBox::new(
        EntityGroupType::Altr,
        GroupId::new(1),
        vec![3, 1],
    ));
    writer.to_bytes().expect("writer")
}

#[test]
fn tone_map_item_reads_base_then_gain_map_and_reemits() {
    let bytes = tone_map_file([1, 2]);
    let image = load_bytes(&bytes, ItemReaderConfig::default());
    let tmap = image.item(ItemId::new(3)).expect("tmap item");
    match tmap.derived().expect("tmap body") {
        DerivedImage::ToneMap(body) => {
            assert_eq!(body.version(), 0);
            assert_eq!(body.gain_map_metadata(), [0xde, 0xad, 0xbe, 0xef]);
        }
        other => panic!("expected tmap, got {other:?}"),
    }
    let inputs = image.tone_map_inputs(ItemId::new(3)).expect("pair");
    assert_eq!(inputs.base().id(), ItemId::new(1));
    assert_eq!(inputs.gain_map().id(), ItemId::new(2));
    assert!(inputs.gain_map().is_hidden());
    assert!(inputs.gain_map_metadata().is_none());
    assert_eq!(image.group_entities(GroupId::new(1)).len(), 2);
    assert!(image.brand_findings().is_empty());
    assert!(image.structure_findings().is_empty());
    let out = image.reemit_into(&bytes).expect("re-emit");
    assert_eq!(out, bytes);
    rebuild_matches_bytes(&bytes);
}

fn rebuild_matches_bytes(bytes: &[u8]) {
    let image = load_bytes(bytes, ItemReaderConfig::default());
    let brands = image.brands().expect("brands");
    let mut writer = ItemWriter::new(brands.major_brand)
        .with_compatible_brands(brands.compatible_brands.clone());
    for item in image.items() {
        writer.add_item(
            item.entry().clone(),
            image.data_bytes(item.id(), bytes).expect("data"),
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
    assert_eq!(reread.primary(), image.primary());
    for item in image.items() {
        let other = reread.item(item.id()).expect("rebuilt item");
        assert_eq!(other.item_type(), item.item_type());
        assert_eq!(
            reread.data_bytes(item.id(), &rebuilt).expect("data"),
            image.data_bytes(item.id(), bytes).expect("data")
        );
    }
}

#[test]
fn angle_cover_item_with_the_narrow_geometry_round_trips() {
    let item = RegionItem::new(160, 120)
        .with_region(RegionGeometry::Point { x: 10, y: 20 })
        .with_region(RegionGeometry::Rectangle {
            x: 1,
            y: 2,
            width: 30,
            height: 40,
        })
        .with_region(RegionGeometry::Ellipse {
            x: 5,
            y: 6,
            radius_x: 7,
            radius_y: 8,
        })
        .with_region(RegionGeometry::Polygon(vec![(0, 0), (10, 0), (10, 10)]))
        .with_region(RegionGeometry::Polyline(vec![(1, 2), (3, 4)]))
        .with_region(RegionGeometry::InlineMask {
            x: 0,
            y: 0,
            width: 8,
            height: 8,
            mask_coding_method: 0,
            mask_coding_parameters: None,
            data: vec![0xff; 8],
        });
    let bytes = item.to_bytes().expect("body");
    let parsed = RegionItem::parse(&bytes).expect("region item");
    assert_eq!(parsed, item);
}

#[test]
fn wide_region_geometry_round_trips() {
    let item = RegionItem::new(70000, 80000)
        .with_wide_fields()
        .with_region(RegionGeometry::Rectangle {
            x: -70000,
            y: 70000,
            width: 65536,
            height: 65537,
        });
    let bytes = item.to_bytes().expect("body");
    let parsed = RegionItem::parse(&bytes).expect("region item");
    assert_eq!(parsed, item);
    assert_eq!(parsed.reference_width(), 70000);
    assert_eq!(parsed.flags() & 1, 1);
}

#[test]
fn group_bodies_round_trip_with_their_flags_and_tile_data() {
    let flags = PYMD_TILE_INFO_PRESENT | PYMD_TILE_INFO_CONSTANT;
    let body = ImageGroupBody::ImagePyramid(
        ImagePyramidEntityGroup::new(flags).with_tile_sizes(vec![(64, 64)]),
    );
    let group =
        isobmff::item::EntityToGroupBox::new(EntityGroupType::Pymd, GroupId::new(7), vec![1, 2])
            .with_version_flags(0, flags)
            .with_body(&body)
            .expect("body");
    let bytes = group.try_to_bytes().expect("group");
    let parsed = isobmff::item::EntityToGroupBox::parse(&bytes).expect("parse group");
    assert_eq!(parsed.group_id(), GroupId::new(7));
    match parsed.image_body().expect("body") {
        ImageGroupBody::ImagePyramid(pyramid) => {
            assert_eq!(pyramid.flags(), flags);
            assert_eq!(pyramid.tile_sizes(), [(64, 64)]);
        }
        other => panic!("expected pymd, got {other:?}"),
    }
    assert_eq!(parsed.try_to_bytes().expect("re-emit"), bytes);

    let area = RegionPartitionArea::new(160, 120, 10, 20, 30, 40);
    let region = RegionPartitionGroup::with_area(area);
    let body = ImageGroupBody::RegionPartition(region);
    let group =
        isobmff::item::EntityToGroupBox::new(EntityGroupType::Rgpa, GroupId::new(8), vec![1])
            .with_version_flags(0, region.flags() | RGPA_AREA_INFO_PRESENT)
            .with_body(&body)
            .expect("body");
    let bytes = group.try_to_bytes().expect("group");
    let parsed = isobmff::item::EntityToGroupBox::parse(&bytes).expect("parse group");
    match parsed.image_body().expect("body") {
        ImageGroupBody::RegionPartition(group) => {
            let area = group.area().expect("area");
            assert_eq!(area.reference_width(), 160);
            assert_eq!(area.left(), 20);
            assert_eq!(area.width(), 30);
        }
        other => panic!("expected rgpa, got {other:?}"),
    }
    assert_eq!(parsed.try_to_bytes().expect("re-emit"), bytes);
}

#[test]
fn integrity_boxes_round_trip() {
    let mut md5 = [0u8; 16];
    md5[0] = 0xaa;
    let plain = Md5IntegrityBox::new(md5, INPUT_TRAK);
    let group = Md5IntegrityBox::new(md5, isobmff::integrity::INPUT_SGPD)
        .with_sample_group(SampleGroupDigest::new(*b"roll", Some(9)).with_indices(vec![1, 2]));
    let item = IntegrityItem::new(vec![
        IntegrityBox::Md5(plain.clone()),
        IntegrityBox::Md5(group.clone()),
    ]);
    let bytes = item.to_bytes().expect("body");
    let parsed = IntegrityItem::parse(&bytes).expect("parse");
    assert_eq!(parsed, item);
    match &parsed.boxes()[1] {
        IntegrityBox::Md5(value) => {
            assert_eq!(value.flags(), 1);
            let group = value.sample_group().expect("group");
            assert_eq!(group.grouping_type(), *b"roll");
            assert_eq!(group.grouping_type_parameter(), Some(9));
            assert_eq!(group.group_description_indices(), [1, 2]);
        }
        other => panic!("expected md5, got {other:?}"),
    }
    assert_eq!(parsed.to_bytes().expect("re-emit"), bytes);
}

#[test]
fn colour_format_enhancement_round_trips_packed_and_whole_inputs() {
    let body = ColourFormatEnhancement::new(vec![
        ColourFormatInput::whole(2),
        ColourFormatInput::packed(1, 1, 0, 0, vec![2, 3, 4, 5]),
    ]);
    let bytes = body.try_to_bytes().expect("body");
    let parsed = ColourFormatEnhancement::parse_with_reference_count(&bytes, 2).expect("parse");
    assert_eq!(parsed, body);
    assert_eq!(parsed.inputs()[0].channel_id(), 2);
    assert!(parsed.inputs()[1].is_packed());
    assert_eq!(parsed.inputs()[1].packed_channel_ids(), [2, 3, 4, 5]);
    assert_eq!(parsed.try_to_bytes().expect("re-emit"), bytes);
}

#[test]
fn image_sequence_boxes_round_trip() {
    let constraints = isobmff::image_sequence::CodingConstraintsBox::new(true, false, 3);
    let bytes = constraints.try_to_bytes().expect("ccst");
    let parsed = isobmff::image_sequence::CodingConstraintsBox::parse(&bytes).expect("parse");
    assert_eq!(parsed, constraints);
    assert!(parsed.all_ref_pics_intra());
    assert!(!parsed.intra_pred_used());
    assert_eq!(parsed.max_ref_per_pic(), 3);
    let info = isobmff::image_sequence::AuxiliaryTypeInfoBox::new(
        b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha",
    );
    let bytes = info.try_to_bytes().expect("auxi");
    assert_eq!(
        isobmff::image_sequence::AuxiliaryTypeInfoBox::parse(&bytes).expect("parse"),
        info
    );
    let refs = isobmff::image_sequence::DirectReferenceSamplesList::new(7, vec![1, 2, 3]);
    let mut bytes = vec![0u8; refs.serialized_len()];
    refs.serialize_into(&mut bytes).expect("refs");
    assert_eq!(
        isobmff::image_sequence::DirectReferenceSamplesList::parse(&bytes).expect("parse"),
        refs
    );
}

#[test]
fn third_edition_item_properties_round_trip() {
    macro_rules! round_trip {
        ($ty:ty, $value:expr) => {{
            let value = $value;
            let bytes = value.try_to_bytes().expect("serialize");
            let parsed = <$ty as broadcast_core::Parse>::parse(&bytes).expect("parse");
            assert_eq!(parsed, value);
            bytes
        }};
    }
    round_trip!(ImageScaling, ImageScaling::new(1, 2, 3, 4));
    round_trip!(
        RequiredReferenceTypesProperty,
        RequiredReferenceTypesProperty::new(vec![*b"pred", *b"prem"])
    );
    round_trip!(
        CreationTimeProperty,
        CreationTimeProperty::new(1_700_000_000_000_000)
    );
    round_trip!(
        ModificationTimeProperty,
        ModificationTimeProperty::new(1_700_000_000_000_001)
    );
    round_trip!(
        UserDescriptionProperty,
        UserDescriptionProperty::new(b"en-US", b"name", b"desc", b"a,b")
    );
    round_trip!(
        AccessibilityTextProperty,
        AccessibilityTextProperty::new(b"a photo", b"fr-FR")
    );
    round_trip!(AutoExposureProperty, AutoExposureProperty::new(3, -2));
    round_trip!(WhiteBalanceProperty, WhiteBalanceProperty::new(5600, -7));
    round_trip!(FocusProperty, FocusProperty::new(1, 3));
    round_trip!(DepthOfFieldProperty, DepthOfFieldProperty::new(-1, 2));
    round_trip!(
        PanoramaProperty,
        PanoramaProperty::new(PanoramaDirection::GridRaster).with_grid(1, 2)
    );
    round_trip!(
        TargetOutputLayerSetProperty,
        TargetOutputLayerSetProperty::new(5)
    );
    round_trip!(
        WipeTransitionEffectProperty,
        WipeTransitionEffectProperty::new(WipeDirection::FromRightBottom)
    );
    round_trip!(
        ZoomTransitionEffectProperty,
        ZoomTransitionEffectProperty::new(ZoomDirection::Out, ZoomShape::Circle)
    );
    round_trip!(
        FadeTransitionEffectProperty,
        FadeTransitionEffectProperty::new(isobmff::image_props::FadeDirection::Dissolve)
    );
    round_trip!(
        SplitTransitionEffectProperty,
        SplitTransitionEffectProperty::new(isobmff::image_props::SplitDirection::HorizontalOut)
    );
    round_trip!(
        SuggestedTransitionPeriodProperty,
        SuggestedTransitionPeriodProperty::new(16)
    );
    round_trip!(
        SuggestedTimeDisplayDurationProperty,
        SuggestedTimeDisplayDurationProperty::new(80)
    );
    round_trip!(
        ProgressiveDerivedImageItemInformationProperty,
        ProgressiveDerivedImageItemInformationProperty::new(Some(3), vec![2, 2, 1])
    );
    round_trip!(SingleStreamProperty, SingleStreamProperty::new());
    round_trip!(
        ConstrainedExtentsGridProperty,
        ConstrainedExtentsGridProperty::new(1, 1, 80, 80)
            .with_large_fields()
            .with_extent_config(vec![1, 2, 3])
    );
    round_trip!(
        DisparityAdjustmentProperty,
        DisparityAdjustmentProperty::new(-250)
    );
    round_trip!(
        StereoAggressorsProperty,
        StereoAggressorsProperty::new(vec![
            StereoAggressor::new(StereoAggressorType::LensOcclusion, 100),
            StereoAggressor::new(StereoAggressorType::ObjectTooClose, 20)
                .with_sub_type_uri(b"https://example.test/aggressor"),
        ])
    );
    round_trip!(
        ReferenceViewingEnvironmentProperty,
        ReferenceViewingEnvironmentProperty::new(1000, 3127, 3290, 5000, 3457, 3585)
    );
    round_trip!(
        NominalDiffuseWhiteProperty,
        NominalDiffuseWhiteProperty::new(1000)
    );
    round_trip!(MaskConfigurationProperty, MaskConfigurationProperty::new(8));
    round_trip!(
        CameraIntrinsicMatrixProperty,
        CameraIntrinsicMatrixProperty::full(1, 2, 3, 4, 5)
            .with_denominator_shift(2)
            .with_skew_denominator_shift(3)
    );
    round_trip!(
        CameraExtrinsicMatrixProperty,
        CameraExtrinsicMatrixProperty::new(
            Some(10),
            Some(-20),
            None,
            Some(CameraOrientation::Quaternion { x: 1, y: -2, z: 3 }),
            Some(9)
        )
    );
}

#[test]
fn camera_extrinsic_version_one_keeps_its_unknown_orientation_opaque() {
    let value = CameraExtrinsicMatrixProperty::new(Some(1), None, None, None, None);
    let mut bytes = value.try_to_bytes().expect("serialize");
    bytes[8] = 1;
    bytes.extend_from_slice(&[0x11, 0x22, 0x33, 0x44]);
    let total = u32::try_from(bytes.len()).expect("length");
    bytes[..4].copy_from_slice(&total.to_be_bytes());
    let parsed = CameraExtrinsicMatrixProperty::parse(&bytes).expect("parse");
    assert_eq!(parsed.version(), 1);
    assert_eq!(parsed.opaque_tail(), [0x11, 0x22, 0x33, 0x44]);
    let reemitted = parsed.try_to_bytes().expect("re-emit");
    assert_eq!(reemitted, bytes);
}

fn region_file(referenced_regions: usize, mask_references: usize) -> Vec<u8> {
    let mut writer = ItemWriter::new(*b"heic").with_compatible_brands(vec![*b"mif1", *b"miaf"]);
    writer.add_item(
        ItemInfoEntry::new(ItemId::new(1), ItemType::Hvc1, b"image"),
        vec![1, 2, 3, 4],
    );
    writer.add_item(
        ItemInfoEntry::new(ItemId::new(2), ItemType::Mski, b"mask"),
        vec![0xff; 8],
    );
    writer
        .add_property(
            ItemId::new(2),
            ItemProperty::MskC(MaskConfigurationProperty::new(1)),
        )
        .expect("mskC");
    let mut region = RegionItem::new(160, 120);
    for _ in 0..referenced_regions {
        region = region.with_region(RegionGeometry::ReferencedMask {
            x: 0,
            y: 0,
            width: 8,
            height: 8,
            mask_ref_idx: None,
        });
    }
    region = region.with_region(RegionGeometry::Rectangle {
        x: 4,
        y: 4,
        width: 10,
        height: 10,
    });
    writer
        .add_region_item(
            ItemInfoEntry::new(ItemId::new(3), ItemType::Rgan, b"region"),
            &region,
        )
        .expect("region");
    writer
        .add_reference(ReferenceType::Cdsc, ItemId::new(3), ItemId::new(1))
        .expect("cdsc");
    for _ in 0..mask_references {
        writer
            .add_reference(ReferenceType::Mask, ItemId::new(3), ItemId::new(2))
            .expect("mask");
    }
    writer.set_primary(ItemId::new(1)).expect("primary");
    writer.to_bytes().expect("writer")
}

#[test]
fn region_item_reads_back_and_its_references_are_checked() {
    let bytes = region_file(1, 1);
    let image = load_bytes(&bytes, ItemReaderConfig::default());
    let region = image.item(ItemId::new(3)).expect("region");
    let body = region.region().expect("typed region body");
    assert_eq!(body.reference_width(), 160);
    assert_eq!(body.regions().len(), 2);
    assert_eq!(image.regions_of(ItemId::new(1)).len(), 1);
    assert!(image.structure_findings().is_empty());
    reemit_and_rebuild(&bytes);

    let hostile = region_file(2, 1);
    let image = load_bytes(&hostile, ItemReaderConfig::default());
    assert_eq!(
        image.structure_findings(),
        [ImageFinding::RegionMaskReferenceCount {
            item: ItemId::new(3),
            regions: 2,
            references: 1,
        }]
    );
}

fn reemit_and_rebuild(bytes: &[u8]) {
    let image = load_bytes(bytes, ItemReaderConfig::default());
    assert_eq!(image.reemit_into(bytes).expect("re-emit"), bytes);
    rebuild_matches_bytes(bytes);
}

#[test]
fn hostile_tone_map_input_count_is_reported() {
    let mut writer = ItemWriter::new(*b"heic").with_compatible_brands(vec![*b"mif1", *b"tmap"]);
    writer.add_item(
        ItemInfoEntry::new(ItemId::new(1), ItemType::Hvc1, b"base"),
        vec![1],
    );
    let tmap = DerivedImage::ToneMap(ToneMapImage::new(vec![0]));
    writer
        .add_derived_item(
            ItemInfoEntry::new(ItemId::new(2), ItemType::Tmap, b"tmap"),
            &tmap,
        )
        .expect("tmap");
    writer
        .add_reference(ReferenceType::Dimg, ItemId::new(2), ItemId::new(1))
        .expect("dimg");
    writer.set_primary(ItemId::new(2)).expect("primary");
    writer.add_group(isobmff::item::EntityToGroupBox::new(
        EntityGroupType::Altr,
        GroupId::new(1),
        vec![2, 1],
    ));
    let bytes = writer.to_bytes().expect("writer");
    let image = load_bytes(&bytes, ItemReaderConfig::default());
    assert_eq!(
        image.structure_findings(),
        [ImageFinding::ToneMapWrongInputCount {
            item: ItemId::new(2),
            count: 1,
        }]
    );
    assert!(image.tone_map_inputs(ItemId::new(2)).is_none());
}

#[test]
fn a_group_naming_a_missing_entity_is_reported() {
    let mut writer = ItemWriter::new(*b"heic").with_compatible_brands(vec![*b"mif1"]);
    writer.add_item(
        ItemInfoEntry::new(ItemId::new(1), ItemType::Hvc1, b"image"),
        vec![1],
    );
    writer.set_primary(ItemId::new(1)).expect("primary");
    writer.add_group(isobmff::item::EntityToGroupBox::new(
        EntityGroupType::Slid,
        GroupId::new(4),
        vec![1, 99],
    ));
    let bytes = writer.to_bytes().expect("writer");
    let image = load_bytes(&bytes, ItemReaderConfig::default());
    assert_eq!(
        image.structure_findings(),
        [ImageFinding::GroupNamesMissingEntity {
            group: GroupId::new(4),
            entity: 99,
        }]
    );
}

#[test]
fn a_region_referencing_a_missing_image_is_reported() {
    let mut writer = ItemWriter::new(*b"heic").with_compatible_brands(vec![*b"mif1"]);
    writer.add_item(
        ItemInfoEntry::new(ItemId::new(1), ItemType::Hvc1, b"image"),
        vec![1],
    );
    let region = RegionItem::new(10, 10).with_region(RegionGeometry::Point { x: 1, y: 1 });
    writer
        .add_region_item(
            ItemInfoEntry::new(ItemId::new(2), ItemType::Rgan, b"region"),
            &region,
        )
        .expect("region");
    writer
        .add_reference(ReferenceType::Cdsc, ItemId::new(2), ItemId::new(77))
        .expect("cdsc");
    writer.set_primary(ItemId::new(1)).expect("primary");
    let bytes = writer.to_bytes().expect("writer");
    let image = load_bytes(&bytes, ItemReaderConfig::default());
    assert!(
        image
            .structure_findings()
            .contains(&ImageFinding::RegionNamesMissingImage {
                item: ItemId::new(2),
                target: ItemId::new(77),
            })
    );
}

#[test]
fn capture_information_properties_are_reachable() {
    let mut writer = ItemWriter::new(*b"heic").with_compatible_brands(vec![*b"mif1"]);
    writer.add_item(
        ItemInfoEntry::new(ItemId::new(1), ItemType::Hvc1, b"image"),
        vec![1],
    );
    writer
        .add_property(
            ItemId::new(1),
            ItemProperty::Aebr(AutoExposureProperty::new(3, -2)),
        )
        .expect("aebr");
    writer
        .add_property(
            ItemId::new(1),
            ItemProperty::Pano(PanoramaProperty::new(PanoramaDirection::LeftToRight)),
        )
        .expect("pano");
    writer.set_primary(ItemId::new(1)).expect("primary");
    let bytes = writer.to_bytes().expect("writer");
    let image = load_bytes(&bytes, ItemReaderConfig::default());
    let info = image.capture_info(ItemId::new(1));
    assert_eq!(info.len(), 2);
    assert!(matches!(info[0].property(), ItemProperty::Aebr(_)));
    assert!(!info[0].essential());
    assert!(!info[1].essential());
    reemit_and_rebuild(&bytes);
}

#[test]
fn brand_findings_are_typed_and_brand_reader_properties_are_unioned() {
    let brands = isobmff::image_brands::ImageBrands::new(*b"msf1", vec![*b"mif1", *b"msf1"]);
    assert_eq!(brands.findings(), [BrandFinding::Msf1WithoutIso8]);
    assert!(brands.reader_properties().contains(b"ispe"));
    let brands = isobmff::image_brands::ImageBrands::new(*b"mif2", vec![*b"mif2"]);
    assert!(brands.reader_properties().contains(b"iscl"));
    assert!(brands.reader_properties().contains(b"rref"));
}

#[test]
fn item_limits_still_bound_a_third_edition_file() {
    let bytes = test_fixtures::read("heif/groups.mp4");
    let mut reader = ItemReader::new(
        ItemReaderConfig::default()
            .with_total_len(Some(bytes.len() as u64))
            .with_limits(Limits::default().with_max_items(1)),
    );
    let outcome = loop {
        match reader.step() {
            Ok(ItemStep::Need(range)) => {
                let start = usize::try_from(range.offset).expect("offset");
                let len = usize::try_from(range.len).expect("length");
                if let Err(err) = reader.supply(range.offset, &bytes[start..start + len]) {
                    break Some(err);
                }
            }
            Ok(ItemStep::Loaded(_)) => panic!("loaded past the item limit"),
            Ok(_) => {}
            Err(err) => break Some(err),
        }
    };
    assert!(matches!(
        outcome,
        Some(isobmff::Error::LimitExceeded { .. })
    ));
}

#[test]
fn an_unknown_region_geometry_keeps_its_bytes_opaque() {
    let body = [0u8, 0, 0, 1, 0, 1, 1, 99, 1, 2, 3, 4];
    let parsed = RegionItem::parse(&body).expect("region item");
    assert_eq!(parsed.regions().len(), 1);
    match &parsed.regions()[0] {
        RegionGeometry::Opaque {
            geometry_type,
            data,
        } => {
            assert_eq!(*geometry_type, 99);
            assert_eq!(data, &[1, 2, 3, 4]);
        }
        other => panic!("expected opaque, got {other:?}"),
    }
    assert_eq!(parsed.to_bytes().expect("re-emit"), body);
}

#[test]
fn payload_hashes_match_the_committed_files() {
    let expected: &[(&str, &[(u32, u64)])] = &[
        ("heif/props.heic", &[(1, 11_177_175_222_047_153_948)]),
        (
            "heif/tiled.heic",
            &[
                (1, 11_047_874_315_465_239_831),
                (2, 5_366_927_597_268_913_940),
                (3, 14_117_563_016_802_496_594),
                (4, 4_681_250_520_803_594_901),
                (5, 15_565_752_291_941_995_649),
            ],
        ),
        (
            "heif/premult.heic",
            &[
                (1, 11_177_175_222_047_153_948),
                (2, 14_105_603_201_270_480_792),
            ],
        ),
        (
            "heif/groups.mp4",
            &[
                (10, 11_177_175_222_047_153_948),
                (11, 12_243_743_699_928_945_842),
                (12, 12_243_743_699_928_945_842),
                (13, 12_243_743_699_928_945_842),
                (14, 12_243_743_699_928_945_842),
                (15, 12_243_743_699_928_945_842),
                (16, 12_243_743_699_928_945_842),
                (17, 12_243_743_699_928_945_842),
            ],
        ),
        (
            "avif/gainmap-multichannel.avif",
            &[
                (1, 9_583_158_648_172_166_789),
                (2, 15_199_326_944_231_806_815),
                (3, 14_129_626_318_572_203_412),
            ],
        ),
        (
            "avif/gainmap-single.avif",
            &[
                (1, 9_583_158_648_172_166_789),
                (2, 1_763_295_244_312_343_592),
                (3, 13_805_453_431_400_229_313),
            ],
        ),
        (
            "avif/gainmap-swapped.avif",
            &[
                (1, 14_399_554_598_986_032_146),
                (2, 6_886_417_231_187_156_639),
                (3, 12_196_124_787_203_775_267),
            ],
        ),
    ];
    for (name, items) in expected {
        let (bytes, image) = load(name);
        assert_eq!(image.items().len(), items.len(), "{name}");
        for (id, hash) in *items {
            let data = image.data_bytes(ItemId::new(*id), &bytes).expect("data");
            let mut actual = 14_695_981_039_346_656_037u64;
            for byte in &data {
                actual ^= u64::from(*byte);
                actual = actual.wrapping_mul(1_099_511_628_211);
            }
            assert_eq!(actual, *hash, "{name} item {id}");
        }
    }
}

fn gain_map(name: &str) -> (Vec<u8>, StillImage, ItemId) {
    let (bytes, image) = load(name);
    let tmap = image
        .items()
        .iter()
        .find(|item| item.item_type() == ItemType::Tmap)
        .expect("tmap item")
        .id();
    (bytes, image, tmap)
}

fn item_hash(image: &StillImage, id: ItemId, source: &[u8]) -> u64 {
    let mut hash = 14_695_981_039_346_656_037u64;
    for byte in image.data_bytes(id, source).expect("item bytes") {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash
}

#[test]
fn gainmap_multichannel_fixture_reads_every_channel() {
    let (bytes, image, tmap) = gain_map("avif/gainmap-multichannel.avif");
    assert_eq!(image.items().len(), 3);
    assert!(image.has_brand(*b"tmap"));
    let inputs = image.tone_map_inputs(tmap).expect("tone map inputs");
    assert_eq!(inputs.base().item_type(), ItemType::Av01);
    assert_eq!(inputs.gain_map().item_type(), ItemType::Av01);
    let metadata = inputs.gain_map_metadata().expect("typed metadata");
    assert_eq!(metadata.minimum_version(), 0);
    assert_eq!(metadata.writer_version(), 0);
    assert!(metadata.is_multichannel());
    assert!(!metadata.uses_base_colour_space());
    assert_eq!(metadata.reserved_flags(), 0);
    assert_eq!(
        metadata.base_headroom(),
        UnsignedRational::new(0, 1),
        "Base headroom:      0 (as fraction: 0/1)"
    );
    assert_eq!(
        metadata.alternate_headroom(),
        UnsignedRational::new(4, 1),
        "Alternate headroom: 4 (as fraction: 4/1)"
    );
    let channels = metadata.channels();
    assert_eq!(channels.len(), 3);
    assert_eq!(
        channels[0].min(),
        SignedRational::new(-4_523_049, 1_048_576),
        "R -4.31352 (as fraction: -4523049/1048576)"
    );
    assert_eq!(
        channels[0].max(),
        SignedRational::new(3_686_517, 524_288),
        "R 7.03147 (as fraction: 3686517/524288)"
    );
    assert_eq!(
        channels[1].min(),
        SignedRational::new(-8_657_321, 4_194_304)
    );
    assert_eq!(
        channels[1].max(),
        SignedRational::new(13_606_797, 2_097_152)
    );
    assert_eq!(
        channels[2].min(),
        SignedRational::new(-4_761_391, 2_097_152)
    );
    assert_eq!(channels[2].max(), SignedRational::new(889_281, 131_072));
    for channel in channels {
        assert_eq!(channel.gamma(), UnsignedRational::new(1, 1));
        assert_eq!(channel.base_offset(), SignedRational::new(1, 64));
        assert_eq!(channel.alternate_offset(), SignedRational::new(1, 64));
    }
    assert!(metadata.zero_denominators().is_empty());
    assert_eq!(
        item_hash(&image, ItemId::new(2), &bytes),
        15_199_326_944_231_806_815
    );
    assert!(image.structure_findings().is_empty());
    reemit_is_byte_exact("avif/gainmap-multichannel.avif");
    rebuild_matches("avif/gainmap-multichannel.avif");
}

#[test]
fn gainmap_single_channel_fixture_reads_one_channel() {
    let (bytes, image, tmap) = gain_map("avif/gainmap-single.avif");
    let metadata = image
        .tone_map_inputs(tmap)
        .expect("tone map inputs")
        .gain_map_metadata()
        .expect("typed metadata");
    assert!(!metadata.is_multichannel());
    let channels = metadata.channels();
    assert_eq!(channels.len(), 1);
    assert_eq!(
        channels[0].min(),
        SignedRational::new(-1_161_593, 2_097_152),
        "-0.553891 (as fraction: -1161593/2097152)"
    );
    assert_eq!(
        channels[0].max(),
        SignedRational::new(14_152_539, 2_097_152),
        "6.74846 (as fraction: 14152539/2097152)"
    );
    assert_eq!(
        item_hash(&image, ItemId::new(2), &bytes),
        1_763_295_244_312_343_592
    );
    reemit_is_byte_exact("avif/gainmap-single.avif");
    rebuild_matches("avif/gainmap-single.avif");
}

#[test]
fn gainmap_swapped_fixture_swaps_headrooms_and_base_colour_space() {
    let (bytes, image, tmap) = gain_map("avif/gainmap-swapped.avif");
    let metadata = image
        .tone_map_inputs(tmap)
        .expect("tone map inputs")
        .gain_map_metadata()
        .expect("typed metadata");
    assert!(metadata.is_multichannel());
    assert!(metadata.uses_base_colour_space());
    assert_eq!(
        metadata.base_headroom(),
        UnsignedRational::new(4, 1),
        "Base headroom:      4 (as fraction: 4/1)"
    );
    assert_eq!(
        metadata.alternate_headroom(),
        UnsignedRational::new(0, 1),
        "Alternate headroom: 0 (as fraction: 0/1)"
    );
    assert_eq!(metadata.channels().len(), 3);
    assert_eq!(
        item_hash(&image, ItemId::new(2), &bytes),
        6_886_417_231_187_156_639
    );
    reemit_is_byte_exact("avif/gainmap-swapped.avif");
    rebuild_matches("avif/gainmap-swapped.avif");
}

#[test]
fn gainmap_metadata_round_trips_the_fixture_body() {
    let (bytes, image, tmap) = gain_map("avif/gainmap-multichannel.avif");
    let body = image.data_bytes(tmap, &bytes).expect("tmap body");
    assert_eq!(body.len(), 142);
    let parsed = GainMapMetadata::parse(&body[1..]).expect("metadata");
    assert_eq!(parsed.try_to_bytes().expect("serialize"), &body[1..]);
    let tone_map = ToneMapImage::with_metadata(parsed).expect("tone map");
    assert_eq!(tone_map.try_to_bytes().expect("body"), body);
}

fn assembled_gain_map_body(reserved: u8, minimum_version: u16, trailing: usize) -> Vec<u8> {
    let channel = GainMapChannel::new(
        SignedRational::new(1, 2),
        SignedRational::new(3, 4),
        UnsignedRational::new(1, 1),
        SignedRational::new(0, 1),
        SignedRational::new(0, 1),
    );
    let metadata = GainMapMetadata::new(
        false,
        UnsignedRational::new(0, 1),
        UnsignedRational::new(1, 1),
        vec![channel],
    )
    .expect("metadata");
    let mut bytes = metadata.try_to_bytes().expect("serialize");
    bytes[0..2].copy_from_slice(&minimum_version.to_be_bytes());
    bytes[4] |= reserved;
    bytes.resize(bytes.len() + trailing, 0);
    let mut body = vec![0u8];
    body.extend_from_slice(&bytes);
    body
}

#[test]
fn gainmap_reserved_bits_keep_the_body_opaque() {
    let body = assembled_gain_map_body(1, 0, 0);
    let tone_map = ToneMapImage::parse(&body).expect("tmap body");
    assert!(tone_map.gain_map_metadata_typed().is_none());
    assert_eq!(tone_map.gain_map_metadata(), &body[1..]);
    assert_eq!(tone_map.try_to_bytes().expect("re-emit"), body);
}

#[test]
fn gainmap_minimum_version_above_zero_keeps_the_body_opaque() {
    let body = assembled_gain_map_body(0, 1, 0);
    let tone_map = ToneMapImage::parse(&body).expect("tmap body");
    assert!(tone_map.gain_map_metadata_typed().is_none());
    assert_eq!(tone_map.try_to_bytes().expect("re-emit"), body);
}

#[test]
fn gainmap_trailing_bytes_keep_the_body_opaque() {
    let body = assembled_gain_map_body(0, 0, 1);
    let tone_map = ToneMapImage::parse(&body).expect("tmap body");
    assert!(tone_map.gain_map_metadata_typed().is_none());
    assert_eq!(tone_map.try_to_bytes().expect("re-emit"), body);
}

#[test]
fn gainmap_zero_denominator_is_a_typed_finding() {
    let channel = GainMapChannel::new(
        SignedRational::new(0, 1),
        SignedRational::new(1, 1),
        UnsignedRational::new(1, 0),
        SignedRational::new(0, 1),
        SignedRational::new(0, 1),
    );
    let metadata = GainMapMetadata::new(
        false,
        UnsignedRational::new(0, 1),
        UnsignedRational::new(1, 1),
        vec![channel],
    )
    .expect("metadata");
    assert_eq!(
        metadata.zero_denominators(),
        [(Some(0), GainMapField::Gamma)]
    );
    let mut writer = ItemWriter::new(*b"mif1").with_compatible_brands(vec![*b"mif1"]);
    writer.add_item(
        ItemInfoEntry::new(ItemId::new(1), ItemType::Av01, b"base"),
        vec![0u8; 4],
    );
    writer.add_item(
        ItemInfoEntry::new(ItemId::new(2), ItemType::Av01, b"gain"),
        vec![1u8; 4],
    );
    writer
        .add_derived_item(
            ItemInfoEntry::new(ItemId::new(3), ItemType::Tmap, b"gain-map"),
            &DerivedImage::ToneMap(ToneMapImage::with_metadata(metadata).expect("tone map")),
        )
        .expect("tmap item");
    for input in [1, 2] {
        writer
            .add_reference(ReferenceType::Dimg, ItemId::new(3), ItemId::new(input))
            .expect("dimg");
    }
    writer.set_primary(ItemId::new(1)).expect("primary");
    let bytes = writer.to_bytes().expect("writer");
    let image = load_bytes(&bytes, ItemReaderConfig::default());
    assert_eq!(
        image.structure_findings(),
        [ImageFinding::GainMapZeroDenominator {
            item: ItemId::new(3),
            channel: Some(0),
            field: GainMapField::Gamma,
        }]
    );
    let metadata = image
        .tone_map_inputs(ItemId::new(3))
        .expect("tone map inputs")
        .gain_map_metadata()
        .expect("typed metadata");
    assert_eq!(metadata.channels()[0].gamma(), UnsignedRational::new(1, 0));
    assert_eq!(metadata.zero_denominators().len(), 1);
}

#[test]
fn gainmap_gamma_numerator_is_unsigned() {
    let channel = GainMapChannel::new(
        SignedRational::new(0, 1),
        SignedRational::new(1, 1),
        UnsignedRational::new(u32::MAX, 1),
        SignedRational::new(0, 1),
        SignedRational::new(0, 1),
    );
    let metadata = GainMapMetadata::new(
        false,
        UnsignedRational::new(0, 1),
        UnsignedRational::new(1, 1),
        vec![channel],
    )
    .expect("metadata");
    let bytes = metadata.try_to_bytes().expect("serialize");
    let parsed = GainMapMetadata::parse(&bytes).expect("parse");
    assert_eq!(
        parsed.channels()[0].gamma(),
        UnsignedRational::new(u32::MAX, 1)
    );
    assert_eq!(parsed.try_to_bytes().expect("serialize"), bytes);
}

#[test]
fn sgpd_dispatches_every_image_sequence_sample_group_entry() {
    let eqiv = SgpdEntry::Eqiv {
        time_offset: -3,
        timescale_multiplier: 0x0100,
    };
    let aebr = SgpdEntry::Aebr {
        exposure_step: 3,
        exposure_numerator: -2,
    };
    let wbbr = SgpdEntry::Wbbr {
        blue_amber: 6500,
        green_magenta: -12,
    };
    let fobr = SgpdEntry::Fobr {
        focus_distance_numerator: 250,
        focus_distance_denominator: 1000,
    };
    let afbr = SgpdEntry::Afbr {
        flash_exposure_numerator: 1,
        flash_exposure_denominator: 2,
    };
    let dobr = SgpdEntry::Dobr {
        f_stop_numerator: 28,
        f_stop_denominator: 10,
    };
    let pano = SgpdEntry::Pano { frame_number: 7 };
    let refs = SgpdEntry::Refs(DirectReferenceSamplesList::new(9, vec![1, 2]));
    let regm = SgpdEntry::Regm(RegionAnnotationMap::new(vec![RegionAnnotation::new(
        4,
        vec![RegionAnnotationAssociation::new(
            AnnotationContainerType::SampleGroup,
            *b"udes",
            2,
        )],
    )]));
    let udes = SgpdEntry::Udes(UserDescriptionSampleGroupEntry::new(
        b"en-US",
        b"panorama frame",
        b"the third frame of the pano",
        b"pano,frame",
    ));
    let cases = [
        (GROUPING_TYPE_EQIV, eqiv),
        (GROUPING_TYPE_AEBR, aebr),
        (GROUPING_TYPE_WBBR, wbbr),
        (GROUPING_TYPE_FOBR, fobr),
        (GROUPING_TYPE_AFBR, afbr),
        (GROUPING_TYPE_DOBR, dobr),
        (GROUPING_TYPE_PANO, pano),
        (GROUPING_TYPE_REFS, refs),
        (GROUPING_TYPE_REGM, regm),
        (GROUPING_TYPE_UDES, udes),
    ];
    for (grouping_type, entry) in cases {
        let box_ = SampleGroupDescriptionBox {
            version: 1,
            flags: 0,
            grouping_type,
            default_length: 0,
            default_group_description_index: None,
            entries: vec![entry.clone()],
        };
        let bytes = box_.try_to_bytes().expect("serialize");
        let parsed = SampleGroupDescriptionBox::parse(&bytes).expect("parse");
        assert_eq!(parsed.entries, [entry], "grouping type {grouping_type:08x}");
        assert_eq!(parsed.try_to_bytes().expect("re-serialize"), bytes);
    }
}

#[test]
fn rgan_sample_entry_is_typed_in_the_stsd_dispatch() {
    let entry =
        RegionSampleEntry::new(RegionTrackConfigBox::new(1920, 1080).with_persistent_region_ids());
    let bytes = entry.try_to_bytes().expect("serialize");
    let parsed = isobmff::init_segment::sample_entry_variant(&bytes).expect("entry");
    match parsed {
        SampleEntryVariant::Rgan(value) => {
            assert_eq!(value.data_reference_index(), 1);
            assert_eq!(value.config().reference_width(), 1920);
            assert_eq!(value.config().reference_height(), 1080);
            assert!(value.config().persistent_region_ids());
        }
        other => panic!("expected rgan, got {other:?}"),
    }
    let box_ = SampleDescriptionBox::new(0, 0, vec![SampleEntryVariant::Rgan(Box::new(entry))]);
    let bytes = box_.try_to_bytes().expect("serialize stsd");
    let parsed = SampleDescriptionBox::parse(&bytes).expect("parse stsd");
    assert!(matches!(parsed.entries[0], SampleEntryVariant::Rgan(_)));
    assert_eq!(parsed.try_to_bytes().expect("re-serialize"), bytes);
}

#[test]
fn coding_constraints_keep_the_reserved_bits_after_max_ref_per_pic() {
    let wire = [
        0u8, 0, 0, 16, b'c', b'c', b's', b't', 0, 0, 0, 0, 0x80, 0, 0, 1,
    ];
    let parsed = CodingConstraintsBox::parse(&wire).expect("ccst parses");
    assert!(parsed.all_ref_pics_intra());
    assert_eq!(parsed.reserved(), 1);
    assert_eq!(parsed.try_to_bytes().expect("ccst serializes"), wire);
}

#[test]
fn region_annotations_keep_the_reserved_bits_of_each_association() {
    let wire = [
        0u8, 0, 0, 1, 0, 0, 0, 7, 0, 1, 0xF9, b'a', b'b', b'c', b'd', 0, 0, 0, 5,
    ];
    let parsed = RegionAnnotationMap::parse(&wire).expect("regm parses");
    let association = parsed.entries()[0].associations()[0];
    assert_eq!(association.reserved(), 0x1f);
    assert_eq!(
        association.container_type(),
        AnnotationContainerType::SampleGroup
    );
    assert_eq!(parsed.to_bytes().expect("regm serializes"), wire);
}

#[test]
fn colour_format_enhancement_keeps_the_reserved_bits_before_is_packed() {
    let wire = [0u8, 0x82, 5];
    let parsed =
        ColourFormatEnhancement::parse_with_reference_count(&wire, 1).expect("cfen parses");
    assert_eq!(parsed.inputs()[0].reserved(), 0x41);
    assert!(!parsed.inputs()[0].is_packed());
    assert_eq!(parsed.inputs()[0].channel_id(), 5);
    assert_eq!(parsed.try_to_bytes().expect("cfen serializes"), wire);
}

#[test]
fn colour_format_enhancement_refuses_a_body_it_cannot_split() {
    let wire = [0u8, 0x00, 5];
    assert!(ColourFormatEnhancement::parse(&wire).is_err());
}

#[test]
fn a_non_zero_md5i_version_is_a_typed_error() {
    let mut wire = Md5IntegrityBox::new([0u8; 16], INPUT_TRAK)
        .try_to_bytes()
        .expect("md5i serializes");
    wire[8] = 1;
    assert!(Md5IntegrityBox::parse(&wire).is_err());
}

#[test]
fn a_property_fullbox_word_the_clause_fixes_is_a_typed_error() {
    let wire = ImageScaling::new(2, 3, 4, 5).try_to_bytes().expect("iscl");
    for offset in [8usize, 9, 10, 11] {
        let mut mutated = wire.clone();
        mutated[offset] = 1;
        assert!(
            ImageScaling::parse(&mutated).is_err(),
            "iscl byte {offset} must be refused"
        );
    }
    let mut tail = wire.clone();
    tail.push(0);
    let total = u32::try_from(tail.len()).expect("length");
    tail[..4].copy_from_slice(&total.to_be_bytes());
    assert!(
        ImageScaling::parse(&tail).is_err(),
        "a body tail is refused"
    );

    let prdi = ProgressiveDerivedImageItemInformationProperty::new(Some(1), vec![1])
        .try_to_bytes()
        .expect("prdi");
    let mut versioned = prdi;
    versioned[8] = 1;
    assert!(ProgressiveDerivedImageItemInformationProperty::parse(&versioned).is_err());
}

#[test]
fn a_signalled_but_empty_stereo_sub_type_uri_round_trips() {
    let wire = [
        0u8, 0, 0, 16, b's', b't', b'a', b'g', 0, 0, 0, 0, 0, 1, 0x81, 0,
    ];
    let parsed = StereoAggressorsProperty::parse(&wire).expect("stag parses");
    let aggressor = &parsed.aggressors()[0];
    assert!(aggressor.sub_type_present());
    assert!(aggressor.sub_type_uri().is_empty());
    assert_eq!(parsed.try_to_bytes().expect("stag serializes"), wire);
}
