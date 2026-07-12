//! Identification — SMPTE ST 377-1:2019 Annex A.3 (`docs/st377-1.md`):
//! records the application/device that created or last modified the file.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};
use crate::local_set::{LocalSet, StructuralSetKind};
use crate::sets::{
    InterchangeObjectFields, LocalSetOwnedItem, collect_dark, finish_owned_set, get_optional_raw,
    get_required_fixed, get_required_raw, owned_set_serialized_len, serialize_owned_set,
};
use crate::types::{MxfTimestamp, ProductVersion, UlBytes, decode_utf16_be, encode_utf16_be};

/// Local tag: This Generation UID (A.3).
pub const TAG_THIS_GENERATION_UID: u16 = 0x3C09;
/// Local tag: Company Name (A.3).
pub const TAG_COMPANY_NAME: u16 = 0x3C01;
/// Local tag: Product Name (A.3).
pub const TAG_PRODUCT_NAME: u16 = 0x3C02;
/// Local tag: Product Version (A.3).
pub const TAG_PRODUCT_VERSION: u16 = 0x3C03;
/// Local tag: Version String (A.3).
pub const TAG_VERSION_STRING: u16 = 0x3C04;
/// Local tag: Product UID (A.3).
pub const TAG_PRODUCT_UID: u16 = 0x3C05;
/// Local tag: Modification Date (A.3).
pub const TAG_MODIFICATION_DATE: u16 = 0x3C06;
/// Local tag: Toolkit Version (A.3).
pub const TAG_TOOLKIT_VERSION: u16 = 0x3C07;
/// Local tag: Platform (A.3).
pub const TAG_PLATFORM: u16 = 0x3C08;

const KNOWN_TAGS: [u16; 12] = [
    crate::sets::TAG_INSTANCE_UID,
    crate::sets::TAG_GENERATION_UID,
    crate::sets::TAG_OBJECT_CLASS,
    TAG_THIS_GENERATION_UID,
    TAG_COMPANY_NAME,
    TAG_PRODUCT_NAME,
    TAG_PRODUCT_VERSION,
    TAG_VERSION_STRING,
    TAG_PRODUCT_UID,
    TAG_MODIFICATION_DATE,
    TAG_TOOLKIT_VERSION,
    TAG_PLATFORM,
];

/// The Identification Set — SMPTE ST 377-1:2019 Annex A.3: one instance per
/// application/device that has created or modified the file (§7.5.2), each
/// referenced from the Preface's `Identifications` array.
///
/// Per A.3's closing note, the Interchange Object's optional Generation UID
/// property "shall not be encoded in Identification Set instances" — this
/// crate does not enforce that at parse time (a non-conformant file is
/// still identified, not rejected); callers constructing a fresh
/// `Identification` should simply leave `interchange.generation_uid` as
/// `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identification {
    /// Interchange Object (A.1) base properties.
    pub interchange: InterchangeObjectFields,
    /// This Generation UID (`0x3C09`, Req) — referenced by other Sets'
    /// optional `Generation UID` property (§7.5.2).
    pub this_generation_uid: UlBytes,
    /// Company Name (`0x3C01`, Req).
    pub company_name: String,
    /// Product Name (`0x3C02`, Req).
    pub product_name: String,
    /// Product Version (`0x3C03`, Opt).
    pub product_version: Option<ProductVersion>,
    /// Version String (`0x3C04`, Req).
    pub version_string: String,
    /// Product UID (`0x3C05`, Req).
    pub product_uid: UlBytes,
    /// Modification Date (`0x3C06`, Req).
    pub modification_date: MxfTimestamp,
    /// Toolkit Version (`0x3C07`, Opt).
    pub toolkit_version: Option<ProductVersion>,
    /// Platform (`0x3C08`, Opt).
    pub platform: Option<String>,
    /// Every other property found on parse (private/dark extension).
    pub dark: Vec<(u16, Vec<u8>)>,
}

impl<'a> Parse<'a> for Identification {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let set = LocalSet::parse(bytes)?;
        if set.kind() != StructuralSetKind::Identification {
            return Err(Error::KeyPrefixMismatch {
                what: "Identification (Table 17)",
            });
        }
        let items = &set.items;
        let interchange = InterchangeObjectFields::decode(items, "Identification")?;
        let this_generation_uid = get_required_fixed::<16>(
            items,
            TAG_THIS_GENERATION_UID,
            "This Generation UID",
            "Identification",
        )?;
        let company_name = decode_utf16_be(get_required_raw(
            items,
            TAG_COMPANY_NAME,
            "Company Name",
            "Identification",
        )?)
        .map_err(|_| Error::InvalidUtf16 {
            tag: TAG_COMPANY_NAME,
            name: "Company Name",
        })?;
        let product_name = decode_utf16_be(get_required_raw(
            items,
            TAG_PRODUCT_NAME,
            "Product Name",
            "Identification",
        )?)
        .map_err(|_| Error::InvalidUtf16 {
            tag: TAG_PRODUCT_NAME,
            name: "Product Name",
        })?;
        let product_version = get_optional_raw(items, TAG_PRODUCT_VERSION)
            .map(ProductVersion::parse)
            .transpose()?;
        let version_string = decode_utf16_be(get_required_raw(
            items,
            TAG_VERSION_STRING,
            "Version String",
            "Identification",
        )?)
        .map_err(|_| Error::InvalidUtf16 {
            tag: TAG_VERSION_STRING,
            name: "Version String",
        })?;
        let product_uid =
            get_required_fixed::<16>(items, TAG_PRODUCT_UID, "Product UID", "Identification")?;
        let modification_date = MxfTimestamp::parse(get_required_raw(
            items,
            TAG_MODIFICATION_DATE,
            "Modification Date",
            "Identification",
        )?)?;
        let toolkit_version = get_optional_raw(items, TAG_TOOLKIT_VERSION)
            .map(ProductVersion::parse)
            .transpose()?;
        let platform = get_optional_raw(items, TAG_PLATFORM)
            .map(decode_utf16_be)
            .transpose()
            .map_err(|_| Error::InvalidUtf16 {
                tag: TAG_PLATFORM,
                name: "Platform",
            })?;
        let dark = collect_dark(items, &KNOWN_TAGS);

        Ok(Identification {
            interchange,
            this_generation_uid,
            company_name,
            product_name,
            product_version,
            version_string,
            product_uid,
            modification_date,
            toolkit_version,
            platform,
            dark,
        })
    }
}

impl Identification {
    fn owned_items(&self) -> Vec<LocalSetOwnedItem> {
        let mut out = Vec::new();
        self.interchange.encode_into(&mut out);
        out.push(LocalSetOwnedItem::fixed(
            TAG_THIS_GENERATION_UID,
            self.this_generation_uid,
        ));
        out.push(LocalSetOwnedItem::owned(
            TAG_COMPANY_NAME,
            encode_utf16_be(&self.company_name),
        ));
        out.push(LocalSetOwnedItem::owned(
            TAG_PRODUCT_NAME,
            encode_utf16_be(&self.product_name),
        ));
        if let Some(pv) = self.product_version {
            let mut buf = [0u8; crate::types::PRODUCT_VERSION_LEN];
            pv.serialize_into(&mut buf).expect("fixed-size buffer");
            out.push(LocalSetOwnedItem::owned(TAG_PRODUCT_VERSION, buf.to_vec()));
        }
        out.push(LocalSetOwnedItem::owned(
            TAG_VERSION_STRING,
            encode_utf16_be(&self.version_string),
        ));
        out.push(LocalSetOwnedItem::fixed(TAG_PRODUCT_UID, self.product_uid));
        {
            let mut buf = [0u8; crate::types::TIMESTAMP_LEN];
            self.modification_date
                .serialize_into(&mut buf)
                .expect("fixed-size buffer");
            out.push(LocalSetOwnedItem::owned(
                TAG_MODIFICATION_DATE,
                buf.to_vec(),
            ));
        }
        if let Some(tv) = self.toolkit_version {
            let mut buf = [0u8; crate::types::PRODUCT_VERSION_LEN];
            tv.serialize_into(&mut buf).expect("fixed-size buffer");
            out.push(LocalSetOwnedItem::owned(TAG_TOOLKIT_VERSION, buf.to_vec()));
        }
        if let Some(platform) = &self.platform {
            out.push(LocalSetOwnedItem::owned(
                TAG_PLATFORM,
                encode_utf16_be(platform),
            ));
        }
        out
    }
}

impl Serialize for Identification {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let (key, items) = finish_owned_set(
            StructuralSetKind::Identification,
            self.owned_items(),
            &self.dark,
        );
        owned_set_serialized_len(key, &items)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let (key, items) = finish_owned_set(
            StructuralSetKind::Identification,
            self.owned_items(),
            &self.dark,
        );
        serialize_owned_set(key, &items, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ReleaseType;

    fn sample() -> Identification {
        Identification {
            interchange: InterchangeObjectFields {
                instance_uid: [0x11; 16],
                generation_uid: None,
                object_class: None,
            },
            this_generation_uid: [0x22; 16],
            company_name: String::from("Acme Broadcast"),
            product_name: String::from("st377-1"),
            product_version: Some(ProductVersion {
                major: 0,
                minor: 1,
                tertiary: 0,
                patch: 0,
                release: ReleaseType::Development,
            }),
            version_string: String::from("0.1.0-dev"),
            product_uid: [0x33; 16],
            modification_date: MxfTimestamp {
                year: 2019,
                month: 11,
                day: 28,
                hour: 9,
                minute: 30,
                second: 0,
                msec_div4: 0,
            },
            toolkit_version: None,
            platform: Some(String::from("rustc (linux)")),
            dark: Vec::new(),
        }
    }

    #[test]
    fn construct_serialize_parse_round_trip() {
        let id = sample();
        let bytes = id.to_bytes();
        let parsed = Identification::parse(&bytes).unwrap();
        assert_eq!(parsed, id);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn mutation_changes_serialized_bytes() {
        let mut id = sample();
        let before = id.to_bytes();
        id.company_name = String::from("Changed Inc.");
        let after = id.to_bytes();
        assert_ne!(before, after);
        assert_eq!(
            Identification::parse(&after).unwrap().company_name,
            "Changed Inc."
        );
    }
}
