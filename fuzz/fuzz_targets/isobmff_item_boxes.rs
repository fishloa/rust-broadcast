#![no_main]

use broadcast_core::{Parse, Serialize};
use isobmff::image::{DerivedImage, ItemProperty};
use isobmff::item::{
    BrandProperty, EntityToGroupBox, GroupsListBox, HandlerProperty, ItemDataBox, ItemInfoBox,
    ItemInfoEntry, ItemLocationBox, ItemPropertiesBox, ItemProtectionBox, ItemReferenceBox,
    PrimaryItemBox,
};
use isobmff::image_derived::{ColourFormatEnhancement, ToneMapImage};
use isobmff::image_props::{
    AccessibilityTextProperty, AutoExposureProperty, CameraExtrinsicMatrixProperty,
    CameraIntrinsicMatrixProperty, ConstrainedExtentsGridProperty, CreationTimeProperty,
    DepthOfFieldProperty, DisparityAdjustmentProperty, FadeTransitionEffectProperty,
    FlashExposureProperty, FocusProperty, ImageScaling, MaskConfigurationProperty,
    ModificationTimeProperty, NominalDiffuseWhiteProperty, PanoramaProperty,
    ProgressiveDerivedImageItemInformationProperty, ReferenceViewingEnvironmentProperty,
    RequiredReferenceTypesProperty, SingleStreamProperty, SplitTransitionEffectProperty,
    StereoAggressorsProperty, SuggestedTimeDisplayDurationProperty,
    SuggestedTransitionPeriodProperty, TargetOutputLayerSetProperty, UserDescriptionProperty,
    WhiteBalanceProperty, WipeTransitionEffectProperty, ZoomTransitionEffectProperty,
};
use isobmff::ItemType;
use libfuzzer_sys::fuzz_target;

macro_rules! round_trip {
    ($ty:ty, $data:expr) => {{
        if let Ok(value) = <$ty as Parse>::parse($data) {
            let mut bytes = vec![0u8; value.serialized_len()];
            let written = value.serialize_into(&mut bytes).expect("serialize");
            assert_eq!(written, bytes.len());
            let again = match <$ty as Parse>::parse(&bytes) {
                Ok(again) => again,
                Err(err) => panic!("{}: re-parse {err}: value {value:?} bytes {bytes:?}", stringify!($ty)),
            };
            assert_eq!(again, value, "{}", stringify!($ty));
        }
    }};
}

fuzz_target!(|data: &[u8]| {
    round_trip!(ItemLocationBox, data);
    round_trip!(ItemInfoBox, data);
    round_trip!(ItemInfoEntry, data);
    round_trip!(PrimaryItemBox, data);
    round_trip!(ItemReferenceBox, data);
    round_trip!(ItemPropertiesBox, data);
    round_trip!(ItemDataBox, data);
    round_trip!(ItemProtectionBox, data);
    round_trip!(GroupsListBox, data);
    round_trip!(BrandProperty, data);
    round_trip!(HandlerProperty, data);
    let _ = EntityToGroupBox::parse(data);
    if let Ok(property) = ItemProperty::parse_box(data) {
        let mut bytes = vec![0u8; property.serialized_len()];
        let _ = property.serialize_into(&mut bytes);
    }
    for item_type in [
        ItemType::Iden,
        ItemType::Iovl,
        ItemType::Grid,
        ItemType::Av01,
        ItemType::Tmap,
        ItemType::Cfen,
        ItemType::Rgan,
    ] {
        let _ = DerivedImage::parse(item_type, data, data.len() % 8);
    }
    round_trip!(ImageScaling, data);
    round_trip!(RequiredReferenceTypesProperty, data);
    round_trip!(CreationTimeProperty, data);
    round_trip!(ModificationTimeProperty, data);
    round_trip!(UserDescriptionProperty, data);
    round_trip!(AccessibilityTextProperty, data);
    round_trip!(AutoExposureProperty, data);
    round_trip!(WhiteBalanceProperty, data);
    round_trip!(FocusProperty, data);
    round_trip!(FlashExposureProperty, data);
    round_trip!(DepthOfFieldProperty, data);
    round_trip!(PanoramaProperty, data);
    round_trip!(TargetOutputLayerSetProperty, data);
    round_trip!(WipeTransitionEffectProperty, data);
    round_trip!(ZoomTransitionEffectProperty, data);
    round_trip!(FadeTransitionEffectProperty, data);
    round_trip!(SplitTransitionEffectProperty, data);
    round_trip!(SuggestedTransitionPeriodProperty, data);
    round_trip!(SuggestedTimeDisplayDurationProperty, data);
    round_trip!(ProgressiveDerivedImageItemInformationProperty, data);
    round_trip!(SingleStreamProperty, data);
    round_trip!(CameraExtrinsicMatrixProperty, data);
    round_trip!(CameraIntrinsicMatrixProperty, data);
    round_trip!(ConstrainedExtentsGridProperty, data);
    round_trip!(DisparityAdjustmentProperty, data);
    round_trip!(StereoAggressorsProperty, data);
    round_trip!(ReferenceViewingEnvironmentProperty, data);
    round_trip!(NominalDiffuseWhiteProperty, data);
    round_trip!(MaskConfigurationProperty, data);
    round_trip!(ToneMapImage, data);
    let _ = ColourFormatEnhancement::parse_with_reference_count(data, data.len() % 8);
    let _ = isobmff::region::RegionItem::parse(data);
    let _ = isobmff::integrity::IntegrityItem::parse(data);
    round_trip!(isobmff::image_sequence::CodingConstraintsBox, data);
    round_trip!(isobmff::image_sequence::AuxiliaryTypeInfoBox, data);
    if let Ok(group) = EntityToGroupBox::parse(data) {
        let _ = group.image_body();
    }
});
