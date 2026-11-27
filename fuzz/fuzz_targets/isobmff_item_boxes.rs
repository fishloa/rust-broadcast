#![no_main]

use broadcast_core::{Parse, Serialize};
use isobmff::image::{DerivedImage, ItemProperty};
use isobmff::item::{
    BrandProperty, EntityToGroupBox, GroupsListBox, HandlerProperty, ItemDataBox, ItemInfoBox,
    ItemInfoEntry, ItemLocationBox, ItemPropertiesBox, ItemProtectionBox, ItemReferenceBox,
    PrimaryItemBox,
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
    ] {
        let _ = DerivedImage::parse(item_type, data, data.len() % 8);
    }
});
