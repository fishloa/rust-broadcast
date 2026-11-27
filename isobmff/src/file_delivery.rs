//! File-delivery boxes — ISO/IEC 14496-12:2026 §8.12 and §9.2.
//!
//! Transcription: `private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md`.
//!
//! | Box | Type | Clause | Container |
//! |-----|------|--------|-----------|
//! | [`FdItemInformationBox`] | `fiin` | §8.12.2 | `meta` |
//! | [`PartitionEntryBox`] | `paen` | §8.12.2.2 | `fiin` |
//! | [`FilePartitionBox`] | `fpar` | §8.12.3 | `paen` |
//! | [`FecReservoirBox`] | `fecr` | §8.12.4 | `paen` |
//! | [`FileReservoirBox`] | `fire` | §8.12.7 | `paen` |
//! | [`FdSessionGroupBox`] | `segr` | §8.12.5 | `fiin` |
//! | [`GroupIdToNameBox`] | `gitn` | §8.12.6 | `fiin` |
//! | [`FecInformationBox`] | `feci` | §9.2.4.7 | `extr` |
//! | [`ExtraDataBox`] | `extr` | §9.2.4.6 | FD sample |
//! | [`FdPacketBox`] | `fdpa` | §9.2.4.2 | FD sample |
//!
//! [`FdHintSampleEntry`](crate::hint::FdHintSampleEntry) is the `fdp ` sample
//! entry (§9.2.3); [`FdSample`] types the bytes of one FD hint sample
//! (§9.2.4.1). The reader reports samples as opaque bytes.
//!
//! OPEN (§9.2.4.5): `FDitemconstructorLarge` (constructor type 5) states
//! 1 + 4 + 4 + 8 + 3 = 20 bytes while the same clause states every constructor
//! is 16 bytes. The two cannot both hold, so that one constructor is left
//! untyped and an FD packet carrying it is a typed parse error.

use alloc::vec;
use alloc::vec::Vec;

use broadcast_core::len::{fit_u8, fit_u16, fit_u32, need_in, need_out};
use broadcast_core::wire::{be_u16, be_u32, be_u64, u24_at};
use broadcast_core::{Parse, Serialize};

use crate::box_support::{
    OpaqueBox, bounded_entry_count, check_entry_count, take_c_string, write_c_string,
};
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_MASK, box_len, parse_box,
    write_box_header, write_full_box_header,
};
use crate::error::{Error, Result};

/// Four-CC of the FD Item Information Box (`fiin`, §8.12.2).
pub const FIIN: [u8; BOX_TYPE_SIZE] = *b"fiin";
/// Four-CC of the Partition Entry Box (`paen`, §8.12.2.2).
pub const PAEN: [u8; BOX_TYPE_SIZE] = *b"paen";
/// Four-CC of the File Partition Box (`fpar`, §8.12.3).
pub const FPAR: [u8; BOX_TYPE_SIZE] = *b"fpar";
/// Four-CC of the FEC Reservoir Box (`fecr`, §8.12.4).
pub const FECR: [u8; BOX_TYPE_SIZE] = *b"fecr";
/// Four-CC of the File Reservoir Box (`fire`, §8.12.7).
pub const FIRE: [u8; BOX_TYPE_SIZE] = *b"fire";
/// Four-CC of the FD Session Group Box (`segr`, §8.12.5).
pub const SEGR: [u8; BOX_TYPE_SIZE] = *b"segr";
/// Four-CC of the Group ID to Name Box (`gitn`, §8.12.6).
pub const GITN: [u8; BOX_TYPE_SIZE] = *b"gitn";
/// Four-CC of the FD sample box (`fdsa`, §9.2.4.1).
pub const FDSA: [u8; BOX_TYPE_SIZE] = *b"fdsa";
/// Four-CC of the FD packet box (`fdpa`, §9.2.4.2).
pub const FDPA: [u8; BOX_TYPE_SIZE] = *b"fdpa";
/// Four-CC of the Extra Data Box (`extr`, §9.2.4.6).
pub const EXTR: [u8; BOX_TYPE_SIZE] = *b"extr";
/// Four-CC of the FEC Information Box (`feci`, §9.2.4.7).
pub const FECI: [u8; BOX_TYPE_SIZE] = *b"feci";

const U8_LEN: usize = 1;
const U16_LEN: usize = core::mem::size_of::<u16>();
const U32_LEN: usize = core::mem::size_of::<u32>();
/// One `fpar` `(block_count, block_size)` entry.
const BLOCK_ENTRY_LEN: usize = U16_LEN + U32_LEN;
/// One `fecr`/`fire` version-1 entry.
const RESERVOIR_ENTRY_LEN_V1: usize = U32_LEN + U32_LEN;
/// One `fecr`/`fire` version-0 entry.
const RESERVOIR_ENTRY_LEN_V0: usize = U16_LEN + U32_LEN;
/// `fpar` fixed body after the version-dependent identifier.
const FPAR_FIXED_BODY_LEN: usize =
    U16_LEN + U8_LEN + U8_LEN + U16_LEN + U16_LEN + U16_LEN + U16_LEN;
/// Width of an FD constructor's discriminator plus padding (§9.2.4.5).
const FD_CONSTRUCTOR_LEN: usize = 16;
/// `LctHeaderTemplate`: one flag byte plus the 16-bit TOI (§9.2.4.3).
const LCT_HEADER_TEMPLATE_LEN: usize = U8_LEN + U16_LEN;
/// `FdPacketBox` fixed fields: the template plus the two entry counts.
const FDPA_FIXED_LEN: usize = LCT_HEADER_TEMPLATE_LEN + U16_LEN;
const FEC_HEADER_LEN: usize = U8_LEN + U16_LEN + U16_LEN + U16_LEN;
const LCT_SHORT_FORM_LIMIT: u8 = 128;
const LCT_FLAG_SENDER_TIME: u8 = 0x80;
const LCT_FLAG_RESIDUAL_TIME: u8 = 0x40;
const LCT_FLAG_SESSION_CLOSE: u8 = 0x20;
const LCT_FLAG_OBJECT_CLOSE: u8 = 0x10;
const LCT_RESERVED_MASK: u8 = 0x0F;
const LCT_VARIABLE_CONTENT_BIAS: usize = 2;
const FIIN_OPTIONAL_CHILDREN: usize = 2;
const FD_TRACK_REF_OFFSET: usize = 1;
const FD_LENGTH_OFFSET: usize = 2;
const FD_SAMPLE_NUMBER_OFFSET: usize = 4;
const FD_SAMPLE_OFFSET: usize = 8;
const FD_BYTES_PER_BLOCK_OFFSET: usize = 12;
const FD_SAMPLES_PER_BLOCK_OFFSET: usize = 14;
const FD_ITEM_EXTENT_OFFSET: usize = 3;
const FD_ITEM_DATA_OFFSET_OFFSET: usize = 5;
const FD_ITEM_DATA_LENGTH_OFFSET: usize = 13;
const FD_XML_DATA_LENGTH_OFFSET: usize = 9;
const FD_XML_RESERVED_OFFSET: usize = 12;
const LCT_LONG_CONTENT_LEN: usize = 3;

const FD_ITEM_CONSTRUCTOR_TYPE: u8 = 3;
const FD_XML_CONSTRUCTOR_TYPE: u8 = 4;
const FD_SAMPLE_CONSTRUCTOR_TYPE: u8 = 2;
const FD_IMMEDIATE_CONSTRUCTOR_TYPE: u8 = 1;
const FD_IMMEDIATE_DATA_LEN: usize = 14;

fn opaque_from_child(child: &crate::box_types::BoxItem<'_>) -> OpaqueBox {
    OpaqueBox {
        box_type: child.four_cc(),
        data: child.payload().to_vec(),
        to_end: child.to_end(),
        largesize: child.has_largesize(),
    }
}

fn require_type(bytes: &[u8], fourcc: &[u8; BOX_TYPE_SIZE], what: &'static str) -> Result<()> {
    let item = parse_box(bytes)?.0;
    if item.four_cc() != *fourcc {
        return Err(Error::InvalidInput(what));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// fiin / paen — §8.12.2
// ---------------------------------------------------------------------------

/// One child of the FD item information box, in wire order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum FiinChild {
    /// A `paen` partition entry; the value indexes
    /// [`FdItemInformationBox::partition_entries`].
    Partition(usize),
    /// The `segr` session information box.
    SessionInfo,
    /// The `gitn` group-name table.
    GroupIdToName,
    /// Any other child, verbatim.
    Opaque(OpaqueBox),
}

/// One child of a partition entry, in wire order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum PartitionEntryChild {
    /// The mandatory `fpar` file partition box.
    FilePartition,
    /// The optional `fecr` FEC reservoir box.
    FecReservoir,
    /// The optional `fire` file reservoir box.
    FileReservoir,
    /// Any other child, verbatim.
    Opaque(OpaqueBox),
}

/// A partition entry box (`paen`, §8.12.2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PartitionEntryBox {
    file_partition: FilePartitionBox,
    fec_reservoir: Option<FecReservoirBox>,
    file_reservoir: Option<FileReservoirBox>,
    order: Vec<PartitionEntryChild>,
}

impl PartitionEntryBox {
    /// Build one from its mandatory file partition box.
    #[must_use]
    pub fn new(file_partition: FilePartitionBox) -> Self {
        Self {
            file_partition,
            fec_reservoir: None,
            file_reservoir: None,
            order: Vec::new(),
        }
    }

    /// The mandatory file partition box (`fpar`).
    #[must_use]
    pub const fn file_partition(&self) -> &FilePartitionBox {
        &self.file_partition
    }

    /// The optional FEC reservoir box (`fecr`).
    #[must_use]
    pub const fn fec_reservoir(&self) -> Option<&FecReservoirBox> {
        self.fec_reservoir.as_ref()
    }

    /// The optional file reservoir box (`fire`).
    #[must_use]
    pub const fn file_reservoir(&self) -> Option<&FileReservoirBox> {
        self.file_reservoir.as_ref()
    }

    /// Attach a FEC reservoir box.
    #[must_use]
    pub fn with_fec_reservoir(mut self, reservoir: FecReservoirBox) -> Self {
        self.fec_reservoir = Some(reservoir);
        self
    }

    /// Attach a file reservoir box.
    #[must_use]
    pub fn with_file_reservoir(mut self, reservoir: FileReservoirBox) -> Self {
        self.file_reservoir = Some(reservoir);
        self
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let mut file_partition = None;
        let mut fec_reservoir = None;
        let mut file_reservoir = None;
        let mut order = Vec::new();
        for child in crate::box_types::BoxIter::new(body) {
            let child = child?;
            if child.four_cc() == FPAR {
                file_partition = Some(FilePartitionBox::parse_body(child.payload())?);
                order.push(PartitionEntryChild::FilePartition);
            } else if child.four_cc() == FECR {
                fec_reservoir = Some(FecReservoirBox::parse_body(child.payload())?);
                order.push(PartitionEntryChild::FecReservoir);
            } else if child.four_cc() == FIRE {
                file_reservoir = Some(FileReservoirBox::parse_body(child.payload())?);
                order.push(PartitionEntryChild::FileReservoir);
            } else {
                order.push(PartitionEntryChild::Opaque(opaque_from_child(&child)));
            }
        }
        Ok(Self {
            file_partition: file_partition.ok_or(Error::InvalidInput("paen missing fpar"))?,
            fec_reservoir,
            file_reservoir,
            order,
        })
    }

    fn child_order(&self) -> Vec<PartitionEntryChild> {
        if !self.order.is_empty() {
            return self.order.clone();
        }
        let mut out = vec![PartitionEntryChild::FilePartition];
        if self.fec_reservoir.is_some() {
            out.push(PartitionEntryChild::FecReservoir);
        }
        if self.file_reservoir.is_some() {
            out.push(PartitionEntryChild::FileReservoir);
        }
        out
    }
}

impl<'a> Parse<'a> for PartitionEntryBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &PAEN, "expected paen")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for PartitionEntryBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut len = BOX_HEADER_MIN_SIZE;
        for child in self.child_order() {
            len += match child {
                PartitionEntryChild::FilePartition => self.file_partition.serialized_len(),
                PartitionEntryChild::FecReservoir => self
                    .fec_reservoir
                    .as_ref()
                    .map_or(0, Serialize::serialized_len),
                PartitionEntryChild::FileReservoir => self
                    .file_reservoir
                    .as_ref()
                    .map_or(0, Serialize::serialized_len),
                PartitionEntryChild::Opaque(ref o) => o.serialized_len(),
            };
        }
        box_len(len)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &PAEN, need)?;
        let order = self.child_order();
        for (index, child) in order.iter().enumerate() {
            let is_last = index + 1 == order.len();
            cursor += match child {
                PartitionEntryChild::FilePartition => {
                    self.file_partition.serialize_into(&mut buf[cursor..])?
                }
                PartitionEntryChild::FecReservoir => match &self.fec_reservoir {
                    Some(box_) => box_.serialize_into(&mut buf[cursor..])?,
                    None => 0,
                },
                PartitionEntryChild::FileReservoir => match &self.file_reservoir {
                    Some(box_) => box_.serialize_into(&mut buf[cursor..])?,
                    None => 0,
                },
                PartitionEntryChild::Opaque(o) => {
                    o.serialize_child_into(&mut buf[cursor..], is_last)?
                }
            };
        }
        Ok(cursor)
    }
}

/// The FD item information box (`fiin`, §8.12.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct FdItemInformationBox {
    version: u8,
    flags: u32,
    partition_entries: Vec<PartitionEntryBox>,
    session_info: Option<FdSessionGroupBox>,
    group_id_to_name: Option<GroupIdToNameBox>,
    order: Vec<FiinChild>,
}

impl FdItemInformationBox {
    /// An empty box (zero partition entries).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The FullBox version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The partition entries, in wire order.
    #[must_use]
    pub fn partition_entries(&self) -> &[PartitionEntryBox] {
        &self.partition_entries
    }

    /// The optional session information box (`segr`).
    #[must_use]
    pub const fn session_info(&self) -> Option<&FdSessionGroupBox> {
        self.session_info.as_ref()
    }

    /// The optional group-name table (`gitn`).
    #[must_use]
    pub const fn group_id_to_name(&self) -> Option<&GroupIdToNameBox> {
        self.group_id_to_name.as_ref()
    }

    /// Append a partition entry.
    pub fn push_partition(&mut self, entry: PartitionEntryBox) {
        self.partition_entries.push(entry);
    }

    /// Attach the session information box.
    pub fn set_session_info(&mut self, session: FdSessionGroupBox) {
        self.session_info = Some(session);
    }

    /// Attach the group-name table.
    pub fn set_group_id_to_name(&mut self, names: GroupIdToNameBox) {
        self.group_id_to_name = Some(names);
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE + U16_LEN, "fiin")?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let entry_count = usize::from(be_u16(body, FULLBOX_EXTRA_SIZE, "fiin entry_count")?);
        let mut cursor = FULLBOX_EXTRA_SIZE + U16_LEN;
        let mut partition_entries = Vec::new();
        let mut session_info = None;
        let mut group_id_to_name = None;
        let mut order = Vec::new();
        for child in crate::box_types::BoxIter::new(&body[cursor..]) {
            let child = child?;
            if child.four_cc() == PAEN {
                order.push(FiinChild::Partition(partition_entries.len()));
                partition_entries.push(PartitionEntryBox::parse_body(child.payload())?);
            } else if child.four_cc() == SEGR {
                if session_info.is_some() {
                    return Err(Error::InvalidInput("fiin with two segr boxes"));
                }
                session_info = Some(FdSessionGroupBox::parse_body(child.payload())?);
                order.push(FiinChild::SessionInfo);
            } else if child.four_cc() == GITN {
                if group_id_to_name.is_some() {
                    return Err(Error::InvalidInput("fiin with two gitn boxes"));
                }
                group_id_to_name = Some(GroupIdToNameBox::parse_body(child.payload())?);
                order.push(FiinChild::GroupIdToName);
            } else {
                order.push(FiinChild::Opaque(opaque_from_child(&child)));
            }
            cursor = cursor.saturating_add(child.bytes.len());
        }
        if partition_entries.len() != entry_count {
            return Err(Error::InvalidInput(
                "fiin entry_count disagrees with its children",
            ));
        }
        Ok(Self {
            version,
            flags,
            partition_entries,
            session_info,
            group_id_to_name,
            order,
        })
    }

    fn child_order(&self) -> Vec<FiinChild> {
        if !self.order.is_empty() {
            return self.order.clone();
        }
        let mut out = Vec::with_capacity(self.partition_entries.len() + FIIN_OPTIONAL_CHILDREN);
        for index in 0..self.partition_entries.len() {
            out.push(FiinChild::Partition(index));
        }
        if self.session_info.is_some() {
            out.push(FiinChild::SessionInfo);
        }
        if self.group_id_to_name.is_some() {
            out.push(FiinChild::GroupIdToName);
        }
        out
    }
}

impl<'a> Parse<'a> for FdItemInformationBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &FIIN, "expected fiin")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for FdItemInformationBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut len = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U16_LEN;
        for child in self.child_order() {
            len += match child {
                FiinChild::Partition(index) => self
                    .partition_entries
                    .get(index)
                    .map_or(0, Serialize::serialized_len),
                FiinChild::SessionInfo => self
                    .session_info
                    .as_ref()
                    .map_or(0, Serialize::serialized_len),
                FiinChild::GroupIdToName => self
                    .group_id_to_name
                    .as_ref()
                    .map_or(0, Serialize::serialized_len),
                FiinChild::Opaque(ref o) => o.serialized_len(),
            };
        }
        box_len(len)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &FIIN, need, self.version, self.flags, false)?;
        let entry_count = fit_u16(self.partition_entries.len(), "fiin entry_count")?;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&entry_count.to_be_bytes());
        cursor += U16_LEN;
        let order = self.child_order();
        for (position, child) in order.iter().enumerate() {
            let is_last = position + 1 == order.len();
            cursor += match child {
                FiinChild::Partition(index) => match self.partition_entries.get(*index) {
                    Some(entry) => entry.serialize_into(&mut buf[cursor..])?,
                    None => 0,
                },
                FiinChild::SessionInfo => match &self.session_info {
                    Some(box_) => box_.serialize_into(&mut buf[cursor..])?,
                    None => 0,
                },
                FiinChild::GroupIdToName => match &self.group_id_to_name {
                    Some(box_) => box_.serialize_into(&mut buf[cursor..])?,
                    None => 0,
                },
                FiinChild::Opaque(o) => o.serialize_child_into(&mut buf[cursor..], is_last)?,
            };
        }
        Ok(cursor)
    }
}

/// One `(block_count, block_size)` pair of a file partition (`fpar`, §8.12.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct BlockPartition {
    block_count: u16,
    block_size: u32,
}

impl BlockPartition {
    /// Build one from a block count and size.
    #[must_use]
    pub const fn new(block_count: u16, block_size: u32) -> Self {
        Self {
            block_count,
            block_size,
        }
    }

    /// `block_count`.
    #[must_use]
    pub const fn block_count(&self) -> u16 {
        self.block_count
    }

    /// `block_size` in bytes.
    #[must_use]
    pub const fn block_size(&self) -> u32 {
        self.block_size
    }
}

/// The file partition box (`fpar`, §8.12.3).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct FilePartitionBox {
    version: u8,
    flags: u32,
    item_id: u32,
    packet_payload_size: u16,
    reserved: u8,
    fec_encoding_id: u8,
    fec_instance_id: u16,
    max_source_block_length: u16,
    encoding_symbol_length: u16,
    max_number_of_encoding_symbols: u16,
    scheme_specific_info: Vec<u8>,
    entries: Vec<BlockPartition>,
}

impl FilePartitionBox {
    /// Build a version-0 box with no partitioning entries.
    #[must_use]
    pub fn new(item_id: u32, packet_payload_size: u16) -> Self {
        Self {
            version: 0,
            flags: 0,
            item_id,
            packet_payload_size,
            reserved: 0,
            fec_encoding_id: 0,
            fec_instance_id: 0,
            max_source_block_length: 0,
            encoding_symbol_length: 0,
            max_number_of_encoding_symbols: 0,
            scheme_specific_info: Vec::new(),
            entries: Vec::new(),
        }
    }

    /// The FullBox version (0 or 1).
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// `item_ID`.
    #[must_use]
    pub const fn item_id(&self) -> u32 {
        self.item_id
    }

    /// `packet_payload_size`.
    #[must_use]
    pub const fn packet_payload_size(&self) -> u16 {
        self.packet_payload_size
    }

    /// The eight reserved bits, carried as read.
    #[must_use]
    pub const fn reserved(&self) -> u8 {
        self.reserved
    }

    /// `FEC_encoding_ID`.
    #[must_use]
    pub const fn fec_encoding_id(&self) -> u8 {
        self.fec_encoding_id
    }

    /// `FEC_instance_ID`.
    #[must_use]
    pub const fn fec_instance_id(&self) -> u16 {
        self.fec_instance_id
    }

    /// `max_source_block_length`.
    #[must_use]
    pub const fn max_source_block_length(&self) -> u16 {
        self.max_source_block_length
    }

    /// `encoding_symbol_length`.
    #[must_use]
    pub const fn encoding_symbol_length(&self) -> u16 {
        self.encoding_symbol_length
    }

    /// `max_number_of_encoding_symbols`.
    #[must_use]
    pub const fn max_number_of_encoding_symbols(&self) -> u16 {
        self.max_number_of_encoding_symbols
    }

    /// The `scheme_specific_info` base64 text, terminator excluded.
    #[must_use]
    pub fn scheme_specific_info(&self) -> &[u8] {
        &self.scheme_specific_info
    }

    /// The `(block_count, block_size)` partitioning entries.
    #[must_use]
    pub fn entries(&self) -> &[BlockPartition] {
        &self.entries
    }

    /// Set the FullBox version (0 or 1).
    #[must_use]
    pub const fn with_version(mut self, version: u8) -> Self {
        self.version = version;
        self
    }

    /// Set the FEC ids.
    #[must_use]
    pub const fn with_fec(mut self, encoding_id: u8, instance_id: u16) -> Self {
        self.fec_encoding_id = encoding_id;
        self.fec_instance_id = instance_id;
        self
    }

    /// Set the symbol sizes.
    #[must_use]
    pub const fn with_symbols(
        mut self,
        max_source_block_length: u16,
        encoding_symbol_length: u16,
        max_number_of_encoding_symbols: u16,
    ) -> Self {
        self.max_source_block_length = max_source_block_length;
        self.encoding_symbol_length = encoding_symbol_length;
        self.max_number_of_encoding_symbols = max_number_of_encoding_symbols;
        self
    }

    /// Append one partitioning entry.
    pub fn push_entry(&mut self, entry: BlockPartition) {
        self.entries.push(entry);
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "fpar")?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let mut cursor = FULLBOX_EXTRA_SIZE;
        let item_id = if version == 0 {
            let value = u32::from(be_u16(body, cursor, "fpar item_ID")?);
            cursor += U16_LEN;
            value
        } else {
            let value = be_u32(body, cursor, "fpar item_ID")?;
            cursor += U32_LEN;
            value
        };
        need_in(body, cursor + FPAR_FIXED_BODY_LEN, "fpar fields")?;
        let packet_payload_size = be_u16(body, cursor, "fpar packet_payload_size")?;
        cursor += U16_LEN;
        let reserved = body[cursor];
        cursor += U8_LEN;
        let fec_encoding_id = body[cursor];
        cursor += U8_LEN;
        let fec_instance_id = be_u16(body, cursor, "fpar FEC_instance_ID")?;
        cursor += U16_LEN;
        let max_source_block_length = be_u16(body, cursor, "fpar max_source_block_length")?;
        cursor += U16_LEN;
        let encoding_symbol_length = be_u16(body, cursor, "fpar encoding_symbol_length")?;
        cursor += U16_LEN;
        let max_number_of_encoding_symbols =
            be_u16(body, cursor, "fpar max_number_of_encoding_symbols")?;
        cursor += U16_LEN;
        let scheme_specific_info = take_c_string(body, &mut cursor, "fpar scheme_specific_info")?;
        let entry_count = if version == 0 {
            let value = usize::from(be_u16(body, cursor, "fpar entry_count")?);
            cursor += U16_LEN;
            value
        } else {
            let value = be_u32(body, cursor, "fpar entry_count")? as usize;
            cursor += U32_LEN;
            value
        };
        check_entry_count(
            body.len().saturating_sub(cursor),
            BLOCK_ENTRY_LEN,
            entry_count,
            "fpar entries",
        )?;
        let mut entries = Vec::with_capacity(bounded_entry_count(
            body.len().saturating_sub(cursor),
            BLOCK_ENTRY_LEN,
            entry_count,
        ));
        for _ in 0..entry_count {
            let block_count = be_u16(body, cursor, "fpar block_count")?;
            cursor += U16_LEN;
            let block_size = be_u32(body, cursor, "fpar block_size")?;
            cursor += U32_LEN;
            entries.push(BlockPartition {
                block_count,
                block_size,
            });
        }
        Ok(Self {
            version,
            flags,
            item_id,
            packet_payload_size,
            reserved,
            fec_encoding_id,
            fec_instance_id,
            max_source_block_length,
            encoding_symbol_length,
            max_number_of_encoding_symbols,
            scheme_specific_info,
            entries,
        })
    }
}

impl<'a> Parse<'a> for FilePartitionBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &FPAR, "expected fpar")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for FilePartitionBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let id_len = if self.version == 0 { U16_LEN } else { U32_LEN };
        let count_len = if self.version == 0 { U16_LEN } else { U32_LEN };
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + id_len
                + FPAR_FIXED_BODY_LEN
                + self.scheme_specific_info.len()
                + 1
                + count_len
                + self.entries.len() * BLOCK_ENTRY_LEN,
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &FPAR, need, self.version, self.flags, false)?;
        if self.version == 0 {
            let id = fit_u16(self.item_id as usize, "fpar item_ID")?;
            buf[cursor..cursor + U16_LEN].copy_from_slice(&id.to_be_bytes());
            cursor += U16_LEN;
        } else {
            buf[cursor..cursor + U32_LEN].copy_from_slice(&self.item_id.to_be_bytes());
            cursor += U32_LEN;
        }
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.packet_payload_size.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor] = self.reserved;
        cursor += U8_LEN;
        buf[cursor] = self.fec_encoding_id;
        cursor += U8_LEN;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.fec_instance_id.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.max_source_block_length.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.encoding_symbol_length.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor..cursor + U16_LEN]
            .copy_from_slice(&self.max_number_of_encoding_symbols.to_be_bytes());
        cursor += U16_LEN;
        cursor += write_c_string(buf, cursor, &self.scheme_specific_info)?;
        if self.version == 0 {
            let count = fit_u16(self.entries.len(), "fpar entry_count")?;
            buf[cursor..cursor + U16_LEN].copy_from_slice(&count.to_be_bytes());
            cursor += U16_LEN;
        } else {
            let count = fit_u32(self.entries.len(), "fpar entry_count")?;
            buf[cursor..cursor + U32_LEN].copy_from_slice(&count.to_be_bytes());
            cursor += U32_LEN;
        }
        for entry in &self.entries {
            buf[cursor..cursor + U16_LEN].copy_from_slice(&entry.block_count.to_be_bytes());
            cursor += U16_LEN;
            buf[cursor..cursor + U32_LEN].copy_from_slice(&entry.block_size.to_be_bytes());
            cursor += U32_LEN;
        }
        Ok(cursor)
    }
}

/// One `(item_ID, symbol_count)` reservoir entry of `fecr`/`fire` (§8.12.4, §8.12.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ReservoirEntry {
    item_id: u32,
    symbol_count: u32,
}

impl ReservoirEntry {
    /// Build one from an item identifier and symbol count.
    #[must_use]
    pub const fn new(item_id: u32, symbol_count: u32) -> Self {
        Self {
            item_id,
            symbol_count,
        }
    }

    /// The reservoir's `item_ID`.
    #[must_use]
    pub const fn item_id(&self) -> u32 {
        self.item_id
    }

    /// The number of symbols the reservoir holds.
    #[must_use]
    pub const fn symbol_count(&self) -> u32 {
        self.symbol_count
    }
}

macro_rules! reservoir_box {
    ($( $ty:ident , $fourcc:expr , $doc:literal ),* $(,)?) => {
        $(
            #[doc = $doc]
            #[derive(Debug, Clone, PartialEq, Eq, Default)]
            #[cfg_attr(feature = "serde", derive(serde::Serialize))]
            #[non_exhaustive]
            pub struct $ty {
                version: u8,
                flags: u32,
                entries: Vec<ReservoirEntry>,
            }

            impl $ty {
                /// An empty version-0 reservoir.
                #[must_use]
                pub fn new() -> Self {
                    Self::default()
                }

                /// The FullBox version (0 or 1).
                #[must_use]
                pub const fn version(&self) -> u8 {
                    self.version
                }

                /// The FullBox flags.
                #[must_use]
                pub const fn flags(&self) -> u32 {
                    self.flags
                }

                /// The `(item_ID, symbol_count)` entries.
                #[must_use]
                pub fn entries(&self) -> &[ReservoirEntry] {
                    &self.entries
                }

                /// Append one entry.
                pub fn push_entry(&mut self, entry: ReservoirEntry) {
                    self.entries.push(entry);
                }

                /// Parse the payload after the 8-byte box header.
                pub fn parse_body(body: &[u8]) -> Result<Self> {
                    need_in(body, FULLBOX_EXTRA_SIZE, stringify!($ty))?;
                    let version = body[0];
                    let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
                    let mut cursor = FULLBOX_EXTRA_SIZE;
                    let entry_count = if version == 0 {
                        let value = usize::from(be_u16(body, cursor, "reservoir entry_count")?);
                        cursor += U16_LEN;
                        value
                    } else {
                        let value = be_u32(body, cursor, "reservoir entry_count")? as usize;
                        cursor += U32_LEN;
                        value
                    };
                    let entry_len = if version == 0 {
                        RESERVOIR_ENTRY_LEN_V0
                    } else {
                        RESERVOIR_ENTRY_LEN_V1
                    };
                    check_entry_count(
                        body.len().saturating_sub(cursor),
                        entry_len,
                        entry_count,
                        "reservoir entries",
                    )?;
                    let mut entries = Vec::with_capacity(bounded_entry_count(
                        body.len().saturating_sub(cursor),
                        entry_len,
                        entry_count,
                    ));
                    for _ in 0..entry_count {
                        let item_id = if version == 0 {
                            let value = u32::from(be_u16(body, cursor, "reservoir item_ID")?);
                            cursor += U16_LEN;
                            value
                        } else {
                            let value = be_u32(body, cursor, "reservoir item_ID")?;
                            cursor += U32_LEN;
                            value
                        };
                        let symbol_count = be_u32(body, cursor, "reservoir symbol_count")?;
                        cursor += U32_LEN;
                        entries.push(ReservoirEntry {
                            item_id,
                            symbol_count,
                        });
                    }
                    Ok(Self {
                        version,
                        flags,
                        entries,
                    })
                }
            }

            impl<'a> Parse<'a> for $ty {
                type Error = Error;

                fn parse(bytes: &'a [u8]) -> Result<Self> {
                    require_type(bytes, &$fourcc, concat!("expected ", stringify!($ty)))?;
                    Self::parse_body(parse_box(bytes)?.0.payload())
                }
            }

            impl Serialize for $ty {
                type Error = Error;

                fn serialized_len(&self) -> usize {
                    let count_len = if self.version == 0 { U16_LEN } else { U32_LEN };
                    let entry_len = if self.version == 0 {
                        RESERVOIR_ENTRY_LEN_V0
                    } else {
                        RESERVOIR_ENTRY_LEN_V1
                    };
                    box_len(
                        BOX_HEADER_MIN_SIZE
                            + FULLBOX_EXTRA_SIZE
                            + count_len
                            + self.entries.len() * entry_len,
                    )
                }

                fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                    let need = self.serialized_len();
                    need_out(buf, need)?;
                    let mut cursor =
                        write_full_box_header(buf, &$fourcc, need, self.version, self.flags, false)?;
                    if self.version == 0 {
                        let count = fit_u16(self.entries.len(), "reservoir entry_count")?;
                        buf[cursor..cursor + U16_LEN].copy_from_slice(&count.to_be_bytes());
                        cursor += U16_LEN;
                    } else {
                        let count = fit_u32(self.entries.len(), "reservoir entry_count")?;
                        buf[cursor..cursor + U32_LEN].copy_from_slice(&count.to_be_bytes());
                        cursor += U32_LEN;
                    }
                    for entry in &self.entries {
                        if self.version == 0 {
                            let id = fit_u16(entry.item_id as usize, "reservoir item_ID")?;
                            buf[cursor..cursor + U16_LEN].copy_from_slice(&id.to_be_bytes());
                            cursor += U16_LEN;
                        } else {
                            buf[cursor..cursor + U32_LEN]
                                .copy_from_slice(&entry.item_id.to_be_bytes());
                            cursor += U32_LEN;
                        }
                        buf[cursor..cursor + U32_LEN]
                            .copy_from_slice(&entry.symbol_count.to_be_bytes());
                        cursor += U32_LEN;
                    }
                    Ok(cursor)
                }
            }
        )*
    };
}

reservoir_box! {
    FecReservoirBox, FECR, "The FEC reservoir box (`fecr`, §8.12.4).",
    FileReservoirBox, FIRE, "The file reservoir box (`fire`, §8.12.7).",
}

/// One session group: its file groups and its hint-track channels (`segr`, §8.12.5).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SessionGroup {
    group_ids: Vec<u32>,
    hint_track_ids: Vec<u32>,
}

impl SessionGroup {
    /// Build one from its file group identifiers and hint-track channel list.
    #[must_use]
    pub fn new(group_ids: Vec<u32>, hint_track_ids: Vec<u32>) -> Self {
        Self {
            group_ids,
            hint_track_ids,
        }
    }

    /// The file groups this session complies with.
    #[must_use]
    pub fn group_ids(&self) -> &[u32] {
        &self.group_ids
    }

    /// The FD hint tracks (one LCT channel each).
    #[must_use]
    pub fn hint_track_ids(&self) -> &[u32] {
        &self.hint_track_ids
    }
}

/// The FD session group box (`segr`, §8.12.5).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct FdSessionGroupBox {
    sessions: Vec<SessionGroup>,
}

impl FdSessionGroupBox {
    /// An empty box.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The session groups.
    #[must_use]
    pub fn sessions(&self) -> &[SessionGroup] {
        &self.sessions
    }

    /// Append one session group.
    pub fn push_session(&mut self, session: SessionGroup) {
        self.sessions.push(session);
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, U16_LEN, "segr")?;
        let session_count = usize::from(be_u16(body, 0, "segr num_session_groups")?);
        let mut cursor = U16_LEN;
        let mut sessions = Vec::with_capacity(bounded_entry_count(
            body.len().saturating_sub(cursor),
            U8_LEN,
            session_count,
        ));
        for _ in 0..session_count {
            need_in(body, cursor + U8_LEN, "segr entry_count")?;
            let group_count = usize::from(body[cursor]);
            cursor += U8_LEN;
            check_entry_count(
                body.len().saturating_sub(cursor),
                U32_LEN,
                group_count,
                "segr group_IDs",
            )?;
            let mut group_ids = Vec::with_capacity(group_count);
            for _ in 0..group_count {
                group_ids.push(be_u32(body, cursor, "segr group_ID")?);
                cursor += U32_LEN;
            }
            let channel_count =
                usize::from(be_u16(body, cursor, "segr num_channels_in_session_group")?);
            cursor += U16_LEN;
            check_entry_count(
                body.len().saturating_sub(cursor),
                U32_LEN,
                channel_count,
                "segr hint_track_IDs",
            )?;
            let mut hint_track_ids = Vec::with_capacity(channel_count);
            for _ in 0..channel_count {
                hint_track_ids.push(be_u32(body, cursor, "segr hint_track_ID")?);
                cursor += U32_LEN;
            }
            sessions.push(SessionGroup {
                group_ids,
                hint_track_ids,
            });
        }
        Ok(Self { sessions })
    }
}

impl<'a> Parse<'a> for FdSessionGroupBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &SEGR, "expected segr")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for FdSessionGroupBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + U16_LEN
                + self
                    .sessions
                    .iter()
                    .map(|session| {
                        U8_LEN
                            + session.group_ids.len() * U32_LEN
                            + U16_LEN
                            + session.hint_track_ids.len() * U32_LEN
                    })
                    .sum::<usize>(),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &SEGR, need)?;
        let count = fit_u16(self.sessions.len(), "segr num_session_groups")?;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&count.to_be_bytes());
        cursor += U16_LEN;
        for session in &self.sessions {
            buf[cursor] = fit_u8(session.group_ids.len(), "segr entry_count")?;
            cursor += U8_LEN;
            for group_id in &session.group_ids {
                buf[cursor..cursor + U32_LEN].copy_from_slice(&group_id.to_be_bytes());
                cursor += U32_LEN;
            }
            let channels = fit_u16(
                session.hint_track_ids.len(),
                "segr num_channels_in_session_group",
            )?;
            buf[cursor..cursor + U16_LEN].copy_from_slice(&channels.to_be_bytes());
            cursor += U16_LEN;
            for track_id in &session.hint_track_ids {
                buf[cursor..cursor + U32_LEN].copy_from_slice(&track_id.to_be_bytes());
                cursor += U32_LEN;
            }
        }
        Ok(cursor)
    }
}

/// One `(group_ID, group_name)` pair of the group-name table (`gitn`, §8.12.6).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct GroupName {
    group_id: u32,
    name: Vec<u8>,
}

impl GroupName {
    /// Build one from a group identifier and its name.
    #[must_use]
    pub fn new(group_id: u32, name: &[u8]) -> Self {
        Self {
            group_id,
            name: name.to_vec(),
        }
    }

    /// The file group identifier.
    #[must_use]
    pub const fn group_id(&self) -> u32 {
        self.group_id
    }

    /// The group name bytes, terminator excluded.
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    /// The group name as UTF-8, when valid.
    #[must_use]
    pub fn name_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.name).ok()
    }
}

/// The group ID to name box (`gitn`, §8.12.6).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct GroupIdToNameBox {
    version: u8,
    flags: u32,
    names: Vec<GroupName>,
}

impl GroupIdToNameBox {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The FullBox version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The `(group_ID, group_name)` pairs, in wire order.
    #[must_use]
    pub fn names(&self) -> &[GroupName] {
        &self.names
    }

    /// Append one pair.
    pub fn push_name(&mut self, name: GroupName) {
        self.names.push(name);
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE + U16_LEN, "gitn")?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let entry_count = usize::from(be_u16(body, FULLBOX_EXTRA_SIZE, "gitn entry_count")?);
        let mut cursor = FULLBOX_EXTRA_SIZE + U16_LEN;
        let mut names = Vec::with_capacity(bounded_entry_count(
            body.len().saturating_sub(cursor),
            U32_LEN + U8_LEN,
            entry_count,
        ));
        for _ in 0..entry_count {
            let group_id = be_u32(body, cursor, "gitn group_ID")?;
            cursor += U32_LEN;
            let name = take_c_string(body, &mut cursor, "gitn group_name")?;
            names.push(GroupName { group_id, name });
        }
        Ok(Self {
            version,
            flags,
            names,
        })
    }
}

impl<'a> Parse<'a> for GroupIdToNameBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &GITN, "expected gitn")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for GroupIdToNameBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + U16_LEN
                + self
                    .names
                    .iter()
                    .map(|name| U32_LEN + name.name.len() + 1)
                    .sum::<usize>(),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &GITN, need, self.version, self.flags, false)?;
        let count = fit_u16(self.names.len(), "gitn entry_count")?;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&count.to_be_bytes());
        cursor += U16_LEN;
        for name in &self.names {
            buf[cursor..cursor + U32_LEN].copy_from_slice(&name.group_id.to_be_bytes());
            cursor += U32_LEN;
            cursor += write_c_string(buf, cursor, &name.name)?;
        }
        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// 9.2.4 FD sample format
// ---------------------------------------------------------------------------

/// The FEC information box (`feci`, §9.2.4.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct FecInformationBox {
    fec_encoding_id: u8,
    fec_instance_id: u16,
    source_block_number: u16,
    encoding_symbol_id: u16,
}

impl FecInformationBox {
    /// Build one from its encoding identity and symbol position.
    #[must_use]
    pub const fn new(
        fec_encoding_id: u8,
        fec_instance_id: u16,
        source_block_number: u16,
        encoding_symbol_id: u16,
    ) -> Self {
        Self {
            fec_encoding_id,
            fec_instance_id,
            source_block_number,
            encoding_symbol_id,
        }
    }

    /// `FEC_encoding_ID`.
    #[must_use]
    pub const fn fec_encoding_id(&self) -> u8 {
        self.fec_encoding_id
    }

    /// `FEC_instance_ID`.
    #[must_use]
    pub const fn fec_instance_id(&self) -> u16 {
        self.fec_instance_id
    }

    /// `source_block_number`.
    #[must_use]
    pub const fn source_block_number(&self) -> u16 {
        self.source_block_number
    }

    /// `encoding_symbol_ID`.
    #[must_use]
    pub const fn encoding_symbol_id(&self) -> u16 {
        self.encoding_symbol_id
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FEC_HEADER_LEN, "feci")?;
        Ok(Self {
            fec_encoding_id: body[0],
            fec_instance_id: be_u16(body, U8_LEN, "feci FEC_instance_ID")?,
            source_block_number: be_u16(body, U8_LEN + U16_LEN, "feci source_block_number")?,
            encoding_symbol_id: be_u16(
                body,
                U8_LEN + U16_LEN + U16_LEN,
                "feci encoding_symbol_ID",
            )?,
        })
    }
}

impl<'a> Parse<'a> for FecInformationBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &FECI, "expected feci")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for FecInformationBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FEC_HEADER_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &FECI, need)?;
        buf[cursor] = self.fec_encoding_id;
        cursor += U8_LEN;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.fec_instance_id.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.source_block_number.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.encoding_symbol_id.to_be_bytes());
        cursor += U16_LEN;
        Ok(cursor)
    }
}

/// The extra data box (`extr`, §9.2.4.6).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ExtraDataBox {
    fec_information: Option<FecInformationBox>,
    extra_data: Vec<u8>,
}

impl ExtraDataBox {
    /// Build one from the bytes after the optional FEC information box.
    #[must_use]
    pub fn new(extra_data: &[u8]) -> Self {
        Self {
            fec_information: None,
            extra_data: extra_data.to_vec(),
        }
    }

    /// The optional `feci` box.
    #[must_use]
    pub const fn fec_information(&self) -> Option<&FecInformationBox> {
        self.fec_information.as_ref()
    }

    /// The `extradata` bytes after the `feci` box.
    #[must_use]
    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }

    /// Attach the FEC information box.
    #[must_use]
    pub fn with_fec_information(mut self, information: FecInformationBox) -> Self {
        self.fec_information = Some(information);
        self
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        if body.len() >= BOX_HEADER_MIN_SIZE && &body[4..BOX_HEADER_MIN_SIZE] == b"feci" {
            let (item, used) = parse_box(body)?;
            let information = FecInformationBox::parse_body(item.payload())?;
            return Ok(Self {
                fec_information: Some(information),
                extra_data: body[used..].to_vec(),
            });
        }
        Ok(Self {
            fec_information: None,
            extra_data: body.to_vec(),
        })
    }
}

impl<'a> Parse<'a> for ExtraDataBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &EXTR, "expected extr")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for ExtraDataBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + self
                    .fec_information
                    .as_ref()
                    .map_or(0, Serialize::serialized_len)
                + self.extra_data.len(),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &EXTR, need)?;
        if let Some(information) = &self.fec_information {
            cursor += information.serialize_into(&mut buf[cursor..])?;
        }
        buf[cursor..cursor + self.extra_data.len()].copy_from_slice(&self.extra_data);
        cursor += self.extra_data.len();
        Ok(cursor)
    }
}

/// The LCT header template (`LCTheaderTemplate`, §9.2.4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LctHeaderTemplate {
    sender_current_time_present: bool,
    expected_residual_time_present: bool,
    session_close_bit: bool,
    object_close_bit: bool,
    reserved: u8,
    transport_object_identifier: u16,
}

impl LctHeaderTemplate {
    /// Build one from its TOI, all flags clear.
    #[must_use]
    pub const fn new(transport_object_identifier: u16) -> Self {
        Self {
            sender_current_time_present: false,
            expected_residual_time_present: false,
            session_close_bit: false,
            object_close_bit: false,
            reserved: 0,
            transport_object_identifier,
        }
    }

    /// `sender_current_time_present`.
    #[must_use]
    pub const fn sender_current_time_present(&self) -> bool {
        self.sender_current_time_present
    }

    /// `expected_residual_time_present`.
    #[must_use]
    pub const fn expected_residual_time_present(&self) -> bool {
        self.expected_residual_time_present
    }

    /// `session_close_bit`.
    #[must_use]
    pub const fn session_close_bit(&self) -> bool {
        self.session_close_bit
    }

    /// `object_close_bit`.
    #[must_use]
    pub const fn object_close_bit(&self) -> bool {
        self.object_close_bit
    }

    /// The four reserved bits, carried as read.
    #[must_use]
    pub const fn reserved(&self) -> u8 {
        self.reserved
    }

    /// `transport_object_identifier`.
    #[must_use]
    pub const fn transport_object_identifier(&self) -> u16 {
        self.transport_object_identifier
    }

    fn parse(bytes: &[u8], cursor: &mut usize) -> Result<Self> {
        need_in(
            bytes,
            *cursor + LCT_HEADER_TEMPLATE_LEN,
            "LCT header template",
        )?;
        let flags = bytes[*cursor];
        *cursor += U8_LEN;
        let toi = be_u16(bytes, *cursor, "LCT TOI")?;
        *cursor += U16_LEN;
        Ok(Self {
            sender_current_time_present: (flags & LCT_FLAG_SENDER_TIME) != 0,
            expected_residual_time_present: (flags & LCT_FLAG_RESIDUAL_TIME) != 0,
            session_close_bit: (flags & LCT_FLAG_SESSION_CLOSE) != 0,
            object_close_bit: (flags & LCT_FLAG_OBJECT_CLOSE) != 0,
            reserved: flags & LCT_RESERVED_MASK,
            transport_object_identifier: toi,
        })
    }

    fn serialize_into(&self, buf: &mut [u8], cursor: &mut usize) -> Result<()> {
        need_out(buf, *cursor + LCT_HEADER_TEMPLATE_LEN)?;
        let flags = if self.sender_current_time_present {
            LCT_FLAG_SENDER_TIME
        } else {
            0
        } | if self.expected_residual_time_present {
            LCT_FLAG_RESIDUAL_TIME
        } else {
            0
        } | if self.session_close_bit {
            LCT_FLAG_SESSION_CLOSE
        } else {
            0
        } | if self.object_close_bit {
            LCT_FLAG_OBJECT_CLOSE
        } else {
            0
        } | (self.reserved & LCT_RESERVED_MASK);
        buf[*cursor] = flags;
        *cursor += U8_LEN;
        buf[*cursor..*cursor + U16_LEN]
            .copy_from_slice(&self.transport_object_identifier.to_be_bytes());
        *cursor += U16_LEN;
        Ok(())
    }
}

/// An LCT header extension constructor (`LCTheaderextension`, §9.2.4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum LctHeaderExtension {
    /// A variable-length extension, `header_extension_type` below 128.
    Variable {
        /// The LCT header extension type.
        header_extension_type: u8,
        /// The `length` field, in 32-bit words.
        length: u8,
        /// The `content` bytes (`length * 4 - 2` of them when `length > 0`).
        content: Vec<u8>,
    },
    /// A fixed one-word extension, `header_extension_type` 128 or above.
    Fixed {
        /// The LCT header extension type.
        header_extension_type: u8,
        /// The three content bytes.
        content: [u8; LCT_LONG_CONTENT_LEN],
    },
}

impl LctHeaderExtension {
    /// The LCT header extension type.
    #[must_use]
    pub const fn header_extension_type(&self) -> u8 {
        match self {
            Self::Variable {
                header_extension_type,
                ..
            }
            | Self::Fixed {
                header_extension_type,
                ..
            } => *header_extension_type,
        }
    }

    fn parse(bytes: &[u8], cursor: &mut usize) -> Result<Self> {
        need_in(bytes, *cursor + U8_LEN, "LCT header extension")?;
        let header_extension_type = bytes[*cursor];
        *cursor += U8_LEN;
        if header_extension_type > LCT_SHORT_FORM_LIMIT {
            need_in(
                bytes,
                *cursor + LCT_LONG_CONTENT_LEN,
                "LCT header extension",
            )?;
            let mut content = [0u8; LCT_LONG_CONTENT_LEN];
            content.copy_from_slice(&bytes[*cursor..*cursor + LCT_LONG_CONTENT_LEN]);
            *cursor += LCT_LONG_CONTENT_LEN;
            Ok(Self::Fixed {
                header_extension_type,
                content,
            })
        } else {
            need_in(bytes, *cursor + U8_LEN, "LCT header extension length")?;
            let length = bytes[*cursor];
            *cursor += U8_LEN;
            let content_len = if length == 0 {
                0
            } else {
                usize::from(length) * U32_LEN - LCT_VARIABLE_CONTENT_BIAS
            };
            need_in(bytes, *cursor + content_len, "LCT header extension content")?;
            let content = bytes[*cursor..*cursor + content_len].to_vec();
            *cursor += content_len;
            Ok(Self::Variable {
                header_extension_type,
                length,
                content,
            })
        }
    }

    fn serialized_len(&self) -> usize {
        match self {
            Self::Variable { content, .. } => U8_LEN + U8_LEN + content.len(),
            Self::Fixed { .. } => U8_LEN + LCT_LONG_CONTENT_LEN,
        }
    }

    fn serialize_into(&self, buf: &mut [u8], cursor: &mut usize) -> Result<()> {
        need_out(buf, *cursor + self.serialized_len())?;
        match self {
            Self::Variable {
                header_extension_type,
                length,
                content,
            } => {
                buf[*cursor] = *header_extension_type;
                *cursor += U8_LEN;
                buf[*cursor] = *length;
                *cursor += U8_LEN;
                buf[*cursor..*cursor + content.len()].copy_from_slice(content);
                *cursor += content.len();
            }
            Self::Fixed {
                header_extension_type,
                content,
            } => {
                buf[*cursor] = *header_extension_type;
                *cursor += U8_LEN;
                buf[*cursor..*cursor + LCT_LONG_CONTENT_LEN].copy_from_slice(content);
                *cursor += LCT_LONG_CONTENT_LEN;
            }
        }
        Ok(())
    }
}

/// One FD packet constructor (`FDconstructor`, §9.2.4.5). Every form is 16 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum FdConstructor {
    /// `type` 0: nothing to insert.
    Noop {
        /// The 15 padding bytes, carried as read.
        pad: [u8; FD_CONSTRUCTOR_LEN - 1],
    },
    /// `type` 1: inline payload bytes.
    Immediate {
        /// `count`, the number of meaningful payload bytes.
        count: u8,
        /// The 14 bytes after `count`: `data[count]` then padding.
        data: [u8; FD_IMMEDIATE_DATA_LEN],
    },
    /// `type` 2: bytes copied from a media sample.
    Sample {
        /// `trackrefindex`.
        track_ref_index: i8,
        /// `length` bytes to copy.
        length: u16,
        /// `samplenumber` in the referenced track.
        sample_number: u32,
        /// `sampleoffset` within that sample.
        sample_offset: u32,
        /// `bytesperblock`, always 1 in MP4 files.
        bytes_per_block: u16,
        /// `samplesperblock`, always 1 in MP4 files.
        samples_per_block: u16,
    },
    /// `type` 3: bytes copied from an item extent.
    Item {
        /// `item_ID` (16-bit form).
        item_id: u16,
        /// `extent_index`.
        extent_index: u16,
        /// `data_offset` within the extent.
        data_offset: u64,
        /// `data_length` (24 bits; 0 means the rest of the extent).
        data_length: u32,
    },
    /// `type` 4: bytes copied from an `xml `/`bxml` box.
    Xml {
        /// `data_offset` within the XML box.
        data_offset: u64,
        /// `data_length`.
        data_length: u32,
        /// The trailing 24 reserved bits, carried as read.
        reserved: u32,
    },
}

impl FdConstructor {
    /// The constructor's union discriminator.
    #[must_use]
    pub const fn constructor_type(&self) -> u8 {
        match self {
            Self::Noop { .. } => 0,
            Self::Immediate { .. } => FD_IMMEDIATE_CONSTRUCTOR_TYPE,
            Self::Sample { .. } => FD_SAMPLE_CONSTRUCTOR_TYPE,
            Self::Item { .. } => FD_ITEM_CONSTRUCTOR_TYPE,
            Self::Xml { .. } => FD_XML_CONSTRUCTOR_TYPE,
        }
    }

    /// Parse one 16-byte constructor.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        need_in(bytes, FD_CONSTRUCTOR_LEN, "FD constructor")?;
        let kind = bytes[0];
        let data_length = u24_at(bytes, FD_ITEM_DATA_LENGTH_OFFSET);
        Ok(match kind {
            0 => {
                let mut pad = [0u8; FD_CONSTRUCTOR_LEN - 1];
                pad.copy_from_slice(&bytes[1..FD_CONSTRUCTOR_LEN]);
                Self::Noop { pad }
            }
            FD_IMMEDIATE_CONSTRUCTOR_TYPE => {
                let count = bytes[1];
                let mut data = [0u8; FD_IMMEDIATE_DATA_LEN];
                data.copy_from_slice(&bytes[2..FD_CONSTRUCTOR_LEN]);
                Self::Immediate { count, data }
            }
            FD_SAMPLE_CONSTRUCTOR_TYPE => Self::Sample {
                track_ref_index: bytes[FD_TRACK_REF_OFFSET] as i8,
                length: be_u16(bytes, FD_LENGTH_OFFSET, "FD constructor length")?,
                sample_number: be_u32(
                    bytes,
                    FD_SAMPLE_NUMBER_OFFSET,
                    "FD constructor samplenumber",
                )?,
                sample_offset: be_u32(bytes, FD_SAMPLE_OFFSET, "FD constructor sampleoffset")?,
                bytes_per_block: be_u16(
                    bytes,
                    FD_BYTES_PER_BLOCK_OFFSET,
                    "FD constructor bytesperblock",
                )?,
                samples_per_block: be_u16(
                    bytes,
                    FD_SAMPLES_PER_BLOCK_OFFSET,
                    "FD constructor samplesperblock",
                )?,
            },
            FD_ITEM_CONSTRUCTOR_TYPE => Self::Item {
                item_id: be_u16(bytes, 1, "FD constructor item_ID")?,
                extent_index: be_u16(bytes, FD_ITEM_EXTENT_OFFSET, "FD constructor extent_index")?,
                data_offset: be_u64(
                    bytes,
                    FD_ITEM_DATA_OFFSET_OFFSET,
                    "FD constructor data_offset",
                )?,
                data_length,
            },
            FD_XML_CONSTRUCTOR_TYPE => Self::Xml {
                data_offset: be_u64(bytes, 1, "FD constructor data_offset")?,
                data_length: u24_at(bytes, FD_XML_DATA_LENGTH_OFFSET),
                reserved: u24_at(bytes, FD_XML_RESERVED_OFFSET),
            },
            _ => return Err(Error::InvalidInput("unknown FD constructor type")),
        })
    }

    /// Serialize one constructor into exactly 16 bytes.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        need_out(buf, FD_CONSTRUCTOR_LEN)?;
        buf[0] = self.constructor_type();
        match self {
            Self::Noop { pad } => buf[1..FD_CONSTRUCTOR_LEN].copy_from_slice(pad),
            Self::Immediate { count, data } => {
                buf[1] = *count;
                buf[2..FD_CONSTRUCTOR_LEN].copy_from_slice(data);
            }
            Self::Sample {
                track_ref_index,
                length,
                sample_number,
                sample_offset,
                bytes_per_block,
                samples_per_block,
            } => {
                buf[1] = *track_ref_index as u8;
                buf[2..4].copy_from_slice(&length.to_be_bytes());
                buf[4..8].copy_from_slice(&sample_number.to_be_bytes());
                buf[8..12].copy_from_slice(&sample_offset.to_be_bytes());
                buf[12..14].copy_from_slice(&bytes_per_block.to_be_bytes());
                buf[14..16].copy_from_slice(&samples_per_block.to_be_bytes());
            }
            Self::Item {
                item_id,
                extent_index,
                data_offset,
                data_length,
            } => {
                buf[1..3].copy_from_slice(&item_id.to_be_bytes());
                buf[3..5].copy_from_slice(&extent_index.to_be_bytes());
                buf[5..13].copy_from_slice(&data_offset.to_be_bytes());
                buf[13..16].copy_from_slice(&data_length.to_be_bytes()[1..]);
            }
            Self::Xml {
                data_offset,
                data_length,
                reserved,
            } => {
                buf[1..9].copy_from_slice(&data_offset.to_be_bytes());
                buf[9..12].copy_from_slice(&data_length.to_be_bytes()[1..]);
                buf[12..15].copy_from_slice(&reserved.to_be_bytes()[1..]);
            }
        }
        Ok(FD_CONSTRUCTOR_LEN)
    }
}

/// One FD packet (`FDpacketBox`, §9.2.4.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct FdPacketBox {
    header_template: LctHeaderTemplate,
    header_extensions: Vec<LctHeaderExtension>,
    constructors: Vec<FdConstructor>,
}

impl FdPacketBox {
    /// Build one from its LCT header template.
    #[must_use]
    pub fn new(header_template: LctHeaderTemplate) -> Self {
        Self {
            header_template,
            header_extensions: Vec::new(),
            constructors: Vec::new(),
        }
    }

    /// The LCT header template.
    #[must_use]
    pub const fn header_template(&self) -> &LctHeaderTemplate {
        &self.header_template
    }

    /// The header extension constructors.
    #[must_use]
    pub fn header_extensions(&self) -> &[LctHeaderExtension] {
        &self.header_extensions
    }

    /// The packet constructors.
    #[must_use]
    pub fn constructors(&self) -> &[FdConstructor] {
        &self.constructors
    }

    /// Append a header extension constructor.
    pub fn push_header_extension(&mut self, extension: LctHeaderExtension) {
        self.header_extensions.push(extension);
    }

    /// Append a packet constructor.
    pub fn push_constructor(&mut self, constructor: FdConstructor) {
        self.constructors.push(constructor);
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FDPA_FIXED_LEN, "fdpa")?;
        let mut cursor = 0usize;
        let header_template = LctHeaderTemplate::parse(body, &mut cursor)?;
        let extension_count = usize::from(be_u16(body, cursor, "fdpa entrycount1")?);
        cursor += U16_LEN;
        let mut header_extensions = Vec::with_capacity(bounded_entry_count(
            body.len().saturating_sub(cursor),
            U8_LEN + U8_LEN,
            extension_count,
        ));
        for _ in 0..extension_count {
            header_extensions.push(LctHeaderExtension::parse(body, &mut cursor)?);
        }
        let constructor_count = usize::from(be_u16(body, cursor, "fdpa entrycount2")?);
        cursor += U16_LEN;
        check_entry_count(
            body.len().saturating_sub(cursor),
            FD_CONSTRUCTOR_LEN,
            constructor_count,
            "fdpa constructors",
        )?;
        let mut constructors = Vec::with_capacity(constructor_count);
        for _ in 0..constructor_count {
            constructors.push(FdConstructor::parse(&body[cursor..])?);
            cursor += FD_CONSTRUCTOR_LEN;
        }
        Ok(Self {
            header_template,
            header_extensions,
            constructors,
        })
    }
}

impl<'a> Parse<'a> for FdPacketBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &FDPA, "expected fdpa")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for FdPacketBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + LCT_HEADER_TEMPLATE_LEN
                + U16_LEN
                + self
                    .header_extensions
                    .iter()
                    .map(LctHeaderExtension::serialized_len)
                    .sum::<usize>()
                + U16_LEN
                + self.constructors.len() * FD_CONSTRUCTOR_LEN,
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &FDPA, need)?;
        self.header_template.serialize_into(buf, &mut cursor)?;
        let extensions = fit_u16(self.header_extensions.len(), "fdpa entrycount1")?;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&extensions.to_be_bytes());
        cursor += U16_LEN;
        for extension in &self.header_extensions {
            extension.serialize_into(buf, &mut cursor)?;
        }
        let constructors = fit_u16(self.constructors.len(), "fdpa entrycount2")?;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&constructors.to_be_bytes());
        cursor += U16_LEN;
        for constructor in &self.constructors {
            constructor.serialize_into(&mut buf[cursor..])?;
            cursor += FD_CONSTRUCTOR_LEN;
        }
        Ok(cursor)
    }
}

/// One FD hint sample (`FDsample`, §9.2.4.1) — the sample's bytes typed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct FdSample {
    packets: Vec<FdPacketBox>,
    extra_data: Option<ExtraDataBox>,
}

impl FdSample {
    /// An empty sample.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The FD packets, in processing order.
    #[must_use]
    pub fn packets(&self) -> &[FdPacketBox] {
        &self.packets
    }

    /// The optional extra data box.
    #[must_use]
    pub const fn extra_data(&self) -> Option<&ExtraDataBox> {
        self.extra_data.as_ref()
    }

    /// Append a packet.
    pub fn push_packet(&mut self, packet: FdPacketBox) {
        self.packets.push(packet);
    }

    /// Attach the extra data box.
    pub fn set_extra_data(&mut self, extra_data: ExtraDataBox) {
        self.extra_data = Some(extra_data);
    }

    /// Parse one sample's bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut packets = Vec::new();
        let mut extra_data = None;
        for item in crate::box_types::BoxIter::new(bytes) {
            let item = item?;
            if item.four_cc() == FDPA {
                packets.push(FdPacketBox::parse_body(item.payload())?);
            } else if item.four_cc() == EXTR {
                extra_data = Some(ExtraDataBox::parse_body(item.payload())?);
            } else {
                return Err(Error::InvalidInput("unexpected box in an FD sample"));
            }
        }
        Ok(Self {
            packets,
            extra_data,
        })
    }
}

impl Serialize for FdSample {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.packets
            .iter()
            .map(Serialize::serialized_len)
            .sum::<usize>()
            + self
                .extra_data
                .as_ref()
                .map_or(0, Serialize::serialized_len)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = 0usize;
        for packet in &self.packets {
            cursor += packet.serialize_into(&mut buf[cursor..])?;
        }
        if let Some(extra) = &self.extra_data {
            cursor += extra.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}
