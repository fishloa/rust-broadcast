//! The still-image writer: build a HEIF/AVIF file from typed parts.
//!
//! The caller supplies the brands, each item's [`ItemInfoEntry`] (type, name,
//! hidden flag, protection index, MIME type) and its data bytes (or a range of
//! a source buffer the writer copies), the item's properties, its references,
//! the primary item and its entity groups. The writer chooses the `iloc` field
//! widths and every offset, lays the metadata out in the conventional order
//! (`hdlr`, `pitm`, `iloc`, `iinf`, `iref`, `iprp`, `grpl`) and appends one
//! `mdat` holding every item's bytes.
//!
//! Re-reading the output through [`crate::item_reader::ItemReader`] yields the
//! same items, types, names, primary item, properties (in the same order),
//! references and data bytes. The writer does not try to reproduce another
//! tool's layout.
//!
//! Sources: `private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md`
//! §8.11 and `private/specs-verified/mpeg/heif/iso-iec-23008-12-2025.md` 6.2.

use alloc::vec::Vec;

use broadcast_core::Serialize;

use crate::box_types::{BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, box_len, write_box_header};
use crate::error::{Error, Result};
use crate::image::ItemProperty;
use crate::item::{
    EntityToGroupBox, GroupsListBox, ItemId, ItemInfoBox, ItemInfoEntry, ItemLocationBox,
    ItemLocationEntry, ItemPropertiesBox, ItemPropertyAssociationBox, ItemPropertyContainerBox,
    ItemPropertyEntry, ItemReferenceBox, PrimaryItemBox, PropertyAssociation, PropertyIndex,
    ReferenceType, SingleItemTypeReferenceBox,
};
use crate::item_reader::{META, MetaChild, MetaRecord, PICT_HANDLER};
use crate::reader::ByteRange;
use crate::segments::FileTypeBox;

/// Four-CC of a media data box (ISO/IEC 14496-12:2026 §8.2.1).
pub const MDAT: [u8; BOX_TYPE_SIZE] = *b"mdat";
/// Offset width the writer chooses for `iloc`.
const ILOC_FIELD_SIZE: u8 = 4;
/// The largest item `ID` the 16-bit wire form can carry.
const WIDE_ID_THRESHOLD: u32 = u16::MAX as u32;
/// The largest property count the 7-bit `ipma` index can address.
const NARROW_PROPERTY_LIMIT: usize = 127;

/// One item the writer will emit.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WritableItem {
    info: ItemInfoEntry,
    data: Vec<u8>,
    properties: Vec<(ItemProperty, bool)>,
    references: Vec<SingleItemTypeReferenceBox>,
}

/// Builds one still-image file ([`ItemWriter::to_bytes`]).
///
/// Each item is added with its [`ItemInfoEntry`] and data; properties and
/// references are attached afterwards by [`ItemId`], so an item's identity is
/// the caller's choice, not the writer's.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ItemWriter {
    major_brand: [u8; BOX_TYPE_SIZE],
    minor_version: u32,
    compatible_brands: Vec<[u8; BOX_TYPE_SIZE]>,
    handler_type: [u8; BOX_TYPE_SIZE],
    items: Vec<WritableItem>,
    primary: Option<ItemId>,
    groups: Vec<EntityToGroupBox>,
}

impl ItemWriter {
    /// A writer for `major_brand`, with the `pict` handler ISO/IEC 23008-12
    /// 6.2 requires and no compatible brands yet.
    #[must_use]
    pub fn new(major_brand: [u8; BOX_TYPE_SIZE]) -> Self {
        Self {
            major_brand,
            minor_version: 0,
            compatible_brands: Vec::new(),
            handler_type: PICT_HANDLER,
            items: Vec::new(),
            primary: None,
            groups: Vec::new(),
        }
    }

    /// Set the compatible brands.
    #[must_use]
    pub fn with_compatible_brands(mut self, compatible_brands: Vec<[u8; BOX_TYPE_SIZE]>) -> Self {
        self.compatible_brands = compatible_brands;
        self
    }

    /// Set the `hdlr` handler type (defaults to `pict`).
    #[must_use]
    pub fn with_handler_type(mut self, handler_type: [u8; BOX_TYPE_SIZE]) -> Self {
        self.handler_type = handler_type;
        self
    }

    /// Add an item with `data`, returning the identifier its entry names.
    pub fn add_item(&mut self, info: ItemInfoEntry, data: Vec<u8>) -> ItemId {
        let id = info.item_id();
        self.items.push(WritableItem {
            info,
            data,
            properties: Vec::new(),
            references: Vec::new(),
        });
        id
    }

    /// Add an item whose data is the bytes at `range` of `source` (copied).
    pub fn add_item_from_range(
        &mut self,
        info: ItemInfoEntry,
        source: &[u8],
        range: ByteRange,
    ) -> Result<ItemId> {
        let start = usize::try_from(range.offset)
            .map_err(|_| Error::InvalidInput("item data range offset"))?;
        let len = usize::try_from(range.len)
            .map_err(|_| Error::InvalidInput("item data range length"))?;
        let bytes = source
            .get(start..start.saturating_add(len))
            .ok_or(Error::InvalidInput("item data range outside the source"))?;
        Ok(self.add_item(info, bytes.to_vec()))
    }

    /// Add a derived image item, writing its typed body.
    pub fn add_derived_item(
        &mut self,
        info: ItemInfoEntry,
        body: &crate::image::DerivedImage,
    ) -> Result<ItemId> {
        Ok(self.add_item(info, body.to_bytes()?))
    }

    /// Add a region item, writing its typed body.
    pub fn add_region_item(
        &mut self,
        info: ItemInfoEntry,
        body: &crate::region::RegionItem,
    ) -> Result<ItemId> {
        Ok(self.add_item(info, body.to_bytes()?))
    }

    /// Add a data-integrity item, writing its typed body.
    pub fn add_integrity_item(
        &mut self,
        info: ItemInfoEntry,
        body: &crate::integrity::IntegrityItem,
    ) -> Result<ItemId> {
        Ok(self.add_item(info, body.to_bytes()?))
    }

    /// Associate a property with an item (in call order), marking it
    /// essential by the item-property convention this crate applies.
    pub fn add_property(&mut self, item: ItemId, property: ItemProperty) -> Result<()> {
        let essential = property_is_essential(&property);
        self.add_property_with_essential(item, property, essential)
    }

    /// Associate a property with an item, stating the essential flag
    /// explicitly (what a re-read model carries).
    pub fn add_property_with_essential(
        &mut self,
        item: ItemId,
        property: ItemProperty,
        essential: bool,
    ) -> Result<()> {
        let entry = self
            .items
            .iter_mut()
            .find(|entry| entry.info.item_id() == item)
            .ok_or(Error::InvalidInput("unknown item for a property"))?;
        entry.properties.push((property, essential));
        Ok(())
    }

    /// Add one reference from `from` to `to`.
    pub fn add_reference(
        &mut self,
        reference_type: ReferenceType,
        from: ItemId,
        to: ItemId,
    ) -> Result<()> {
        let entry = self
            .items
            .iter_mut()
            .find(|entry| entry.info.item_id() == from)
            .ok_or(Error::InvalidInput("unknown item for a reference"))?;
        if let Some(existing) = entry
            .references
            .iter_mut()
            .find(|reference| reference.reference_type() == reference_type)
        {
            let mut targets = existing.to().to_vec();
            targets.push(to);
            *existing = SingleItemTypeReferenceBox::new(reference_type, from, targets);
        } else {
            entry.references.push(SingleItemTypeReferenceBox::new(
                reference_type,
                from,
                alloc::vec![to],
            ));
        }
        Ok(())
    }

    /// Name the primary item.
    pub fn set_primary(&mut self, item: ItemId) -> Result<()> {
        if self.items.iter().any(|entry| entry.info.item_id() == item) {
            self.primary = Some(item);
            Ok(())
        } else {
            Err(Error::InvalidInput("unknown primary item"))
        }
    }

    /// Add an entity group.
    pub fn add_group(&mut self, group: EntityToGroupBox) {
        self.groups.push(group);
    }

    /// Whether any item needs the 32-bit identifier form.
    fn needs_wide_ids(&self) -> bool {
        self.items
            .iter()
            .any(|entry| entry.info.item_id().get() > WIDE_ID_THRESHOLD)
    }

    fn sorted_items(&self) -> Vec<&WritableItem> {
        let mut items: Vec<&WritableItem> = self.items.iter().collect();
        items.sort_by_key(|entry| entry.info.item_id().get());
        items
    }

    fn build_info(&self) -> Result<ItemInfoBox> {
        let mut info = ItemInfoBox::new();
        if self.items.len() > usize::from(u16::MAX) {
            info = ItemInfoBox::new();
        }
        for entry in self.sorted_items() {
            info.push(entry.info.clone().with_selected_version());
        }
        Ok(info)
    }

    fn build_locations(&self, offsets: &[(ItemId, u64, u64)]) -> Result<ItemLocationBox> {
        let mut iloc = if self.needs_wide_ids() {
            ItemLocationBox::with_version(crate::item::ILOC_VERSION_WIDE_ID)
        } else {
            ItemLocationBox::new()
        };
        iloc.set_field_sizes(ILOC_FIELD_SIZE, ILOC_FIELD_SIZE, 0, 0)?;
        for (id, offset, length) in offsets {
            iloc.push(ItemLocationEntry::new(
                *id,
                crate::item::ConstructionMethod::File,
                alloc::vec![crate::item::ItemExtent::new(*offset, *length)],
            ));
        }
        Ok(iloc)
    }

    fn build_properties(&self) -> Result<Option<ItemPropertiesBox>> {
        let has_properties = self.items.iter().any(|entry| !entry.properties.is_empty());
        if !has_properties {
            return Ok(None);
        }
        let mut container = ItemPropertyContainerBox::new();
        let mut associations = ItemPropertyAssociationBox::new();
        let total: usize = self.items.iter().map(|entry| entry.properties.len()).sum();
        if total > NARROW_PROPERTY_LIMIT {
            associations = {
                let mut wide = ItemPropertyAssociationBox::new();
                wide.set_wide_index()?;
                wide
            };
        }
        if self.needs_wide_ids() {
            associations.set_wide_id()?;
        }
        for entry in self.sorted_items() {
            if entry.properties.is_empty() {
                continue;
            }
            let mut entry_associations = Vec::with_capacity(entry.properties.len());
            for (property, essential) in &entry.properties {
                container.push(property.clone());
                let index = u16::try_from(container.properties().len())
                    .map_err(|_| Error::InvalidInput("property count exceeds u16"))?;
                entry_associations.push(PropertyAssociation::new(
                    PropertyIndex::new(index),
                    *essential,
                ));
            }
            associations.push(ItemPropertyEntry::new(
                entry.info.item_id(),
                entry_associations,
            ));
        }
        let mut properties = ItemPropertiesBox::with_container(container);
        properties.push_associations(associations);
        Ok(Some(properties))
    }

    fn build_references(&self) -> Result<Option<ItemReferenceBox>> {
        let has_references = self.items.iter().any(|entry| !entry.references.is_empty());
        if !has_references {
            return Ok(None);
        }
        let mut iref = ItemReferenceBox::new();
        if self.needs_wide_ids() {
            iref.set_wide_id()?;
        }
        for entry in self.sorted_items() {
            for reference in &entry.references {
                iref.push(reference.clone());
            }
        }
        Ok(Some(iref))
    }

    /// Build the file bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let ftyp = FileTypeBox {
            major_brand: self.major_brand,
            minor_version: self.minor_version,
            compatible_brands: self.compatible_brands.clone(),
        };
        let ftyp_len = ftyp.serialized_len();
        let payload_len: u64 = self.items.iter().fold(0u64, |acc, entry| {
            acc.saturating_add(entry.data.len() as u64)
        });
        let mdat_total = box_len(BOX_HEADER_MIN_SIZE + payload_len as usize);
        let mdat_header_len = mdat_total.saturating_sub(payload_len as usize);
        let mut offsets = Vec::with_capacity(self.items.len());
        let mut running = 0u64;
        for entry in self.sorted_items() {
            if entry.data.is_empty() {
                continue;
            }
            offsets.push((entry.info.item_id(), running, entry.data.len() as u64));
            running = running.saturating_add(entry.data.len() as u64);
        }
        let placeholder = self.build_meta(&offsets)?;
        let meta_len = placeholder.serialized_len()?;
        let data_base = ftyp_len
            .saturating_add(meta_len)
            .saturating_add(mdat_header_len);
        let data_base =
            u64::try_from(data_base).map_err(|_| Error::InvalidInput("file layout exceeds u64"))?;
        let mut placed = Vec::with_capacity(offsets.len());
        for (id, relative, length) in &offsets {
            placed.push((*id, data_base.saturating_add(*relative), *length));
        }
        let meta = self.build_meta(&placed)?;
        if meta.serialized_len()? != meta_len {
            return Err(Error::InvalidInput("meta length changed with the offsets"));
        }
        let meta_body = meta.to_body()?;
        let mut out = Vec::with_capacity(
            ftyp_len
                .saturating_add(BOX_HEADER_MIN_SIZE)
                .saturating_add(meta_body.len())
                .saturating_add(mdat_total),
        );
        let meta_total = box_len(BOX_HEADER_MIN_SIZE + meta_body.len());
        out.resize(ftyp_len + meta_total, 0);
        ftyp.serialize_into(&mut out[..ftyp_len])?;
        let meta_start = ftyp_len;
        let mut cursor = write_box_header(&mut out[meta_start..], &META, meta_total)?;
        let body_start = meta_start + cursor;
        out[body_start..body_start + meta_body.len()].copy_from_slice(&meta_body);
        cursor += meta_body.len();
        if meta_start + cursor != ftyp_len + meta_total {
            return Err(Error::InvalidInput("meta layout changed"));
        }
        let mdat_start = out.len();
        out.resize(mdat_start + mdat_total, 0);
        let header = write_box_header(&mut out[mdat_start..], &MDAT, mdat_total)?;
        let mut at = mdat_start.saturating_add(header);
        for entry in self.sorted_items() {
            if entry.data.is_empty() {
                continue;
            }
            out[at..at + entry.data.len()].copy_from_slice(&entry.data);
            at += entry.data.len();
        }
        if at != out.len() {
            return Err(Error::InvalidInput("mdat payload length changed"));
        }
        Ok(out)
    }

    fn build_meta(&self, offsets: &[(ItemId, u64, u64)]) -> Result<MetaRecord> {
        let mut children = Vec::new();
        children.push(MetaChild::Hdlr(crate::metadata::Hdlr::new(
            self.handler_type,
        )));
        if let Some(primary) = self.primary {
            children.push(MetaChild::Pitm(PrimaryItemBox::new(primary)));
        }
        children.push(MetaChild::Iloc(self.build_locations(offsets)?));
        children.push(MetaChild::Iinf(self.build_info()?));
        if let Some(iref) = self.build_references()? {
            children.push(MetaChild::Iref(iref));
        }
        if let Some(properties) = self.build_properties()? {
            children.push(MetaChild::Iprp(properties));
        }
        if !self.groups.is_empty() {
            let mut groups = GroupsListBox::new();
            for group in &self.groups {
                groups.push(group.clone());
            }
            children.push(MetaChild::Grpl(groups));
        }
        Ok(MetaRecord::new(0, 0, children))
    }
}

fn property_is_essential(property: &ItemProperty) -> bool {
    matches!(
        property,
        ItemProperty::HvcC(_)
            | ItemProperty::AvcC(_)
            | ItemProperty::Av1C(_)
            | ItemProperty::A1op(_)
            | ItemProperty::Lsel(_)
            | ItemProperty::AuxC(_)
            | ItemProperty::Rref(_)
            | ItemProperty::Tols(_)
            | ItemProperty::MskC(_)
    )
}
