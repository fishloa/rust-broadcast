//! Integrity checks for image sequences — ISO/IEC 23008-12:2025 8.4.
//!
//! A `mint` item in a *track-level* `MetaBox` carries one or more
//! [`Md5IntegrityBox`]es (`md5i`, 8.4.2). Each names the sample data the MD5
//! digest covers; this crate types the box and never computes or verifies a
//! digest, because doing so would need the whole track's sample bytes.
//!
//! Sources: `private/specs-verified/mpeg/heif/iso-iec-23008-12-2025.md` 8.4.

use alloc::vec::Vec;

use broadcast_core::len::{fit_u32, need_in, need_out};
use broadcast_core::wire::be_u32;
use broadcast_core::{Parse, Serialize};

use crate::box_support::OpaqueBox;
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, box_len, write_full_box_header,
};
use crate::error::{Error, Result};
use crate::image::full_flags;

/// Four-CC of the MD5 Integrity box (8.4.2).
pub const MD5I: [u8; BOX_TYPE_SIZE] = *b"md5i";
/// The `input_4cc` naming the concatenated sample sizes (8.4.3).
pub const INPUT_STSZ: [u8; BOX_TYPE_SIZE] = *b"stsz";
/// The `input_4cc` naming the concatenated sample bytes (8.4.3).
pub const INPUT_TRAK: [u8; BOX_TYPE_SIZE] = *b"trak";
/// The `input_4cc` naming samples mapped to a sample group (8.4.3).
pub const INPUT_SGPD: [u8; BOX_TYPE_SIZE] = *b"sgpd";
/// `flags & 1`: a `grouping_type_parameter` is present (8.4.2).
pub const MD5I_GROUPING_PARAMETER: u32 = 1;
/// Bytes of `input_MD5` (8.4.2).
const MD5_LEN: usize = 16;
/// The only `version` an `md5i` states (8.4.2).
const MD5I_VERSION: u8 = 0;

/// The sample group an [`Md5IntegrityBox`] covers (8.4.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SampleGroupDigest {
    grouping_type: [u8; BOX_TYPE_SIZE],
    grouping_type_parameter: Option<u32>,
    group_description_indices: Vec<u32>,
}

impl SampleGroupDigest {
    /// A digest over `grouping_type`, optionally narrowed by a parameter.
    #[must_use]
    pub fn new(grouping_type: [u8; BOX_TYPE_SIZE], grouping_type_parameter: Option<u32>) -> Self {
        Self {
            grouping_type,
            grouping_type_parameter,
            group_description_indices: Vec::new(),
        }
    }

    /// Restrict the digest to these sample group description indices.
    #[must_use]
    pub fn with_indices(mut self, indices: Vec<u32>) -> Self {
        self.group_description_indices = indices;
        self
    }

    /// `grouping_type`.
    #[must_use]
    pub const fn grouping_type(&self) -> [u8; BOX_TYPE_SIZE] {
        self.grouping_type
    }

    /// `grouping_type_parameter`.
    #[must_use]
    pub const fn grouping_type_parameter(&self) -> Option<u32> {
        self.grouping_type_parameter
    }

    /// `group_description_index[i]`.
    #[must_use]
    pub fn group_description_indices(&self) -> &[u32] {
        &self.group_description_indices
    }
}

/// MD5 Integrity box (`md5i`) — ISO/IEC 23008-12:2025 8.4.2.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Md5IntegrityBox {
    version: u8,
    flags: u32,
    input_md5: [u8; MD5_LEN],
    input_4cc: [u8; BOX_TYPE_SIZE],
    sample_group: Option<SampleGroupDigest>,
}

impl Md5IntegrityBox {
    /// A digest over `input_4cc` data.
    #[must_use]
    pub const fn new(input_md5: [u8; MD5_LEN], input_4cc: [u8; BOX_TYPE_SIZE]) -> Self {
        Self {
            version: MD5I_VERSION,
            flags: 0,
            input_md5,
            input_4cc,
            sample_group: None,
        }
    }

    /// A digest over a sample group.
    #[must_use]
    pub fn with_sample_group(mut self, sample_group: SampleGroupDigest) -> Self {
        self.input_4cc = INPUT_SGPD;
        if sample_group.grouping_type_parameter.is_some() {
            self.flags |= MD5I_GROUPING_PARAMETER;
        }
        self.sample_group = Some(sample_group);
        self
    }

    /// The FullBox `version`.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// `input_MD5`.
    #[must_use]
    pub const fn input_md5(&self) -> &[u8; MD5_LEN] {
        &self.input_md5
    }

    /// `input_4cc`.
    #[must_use]
    pub const fn input_4cc(&self) -> [u8; BOX_TYPE_SIZE] {
        self.input_4cc
    }

    /// The sample group, when `input_4cc` is `sgpd`.
    #[must_use]
    pub const fn sample_group(&self) -> Option<&SampleGroupDigest> {
        self.sample_group.as_ref()
    }
}

impl<'a> broadcast_core::Parse<'a> for Md5IntegrityBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, BOX_HEADER_MIN_SIZE, "md5i")?;
        if broadcast_core::wire::u32_at(bytes, crate::box_types::BOX_TYPE_OFFSET)
            != u32::from_be_bytes(MD5I)
        {
            return Err(Error::InvalidInput("expected md5i"));
        }
        let body = crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE + MD5_LEN + BOX_TYPE_SIZE,
            "md5i",
        )?;
        let version = body[0];
        if version != MD5I_VERSION {
            return Err(Error::InvalidInput("md5i version"));
        }
        let flags = full_flags(body);
        let mut input_md5 = [0u8; MD5_LEN];
        input_md5.copy_from_slice(&body[FULLBOX_EXTRA_SIZE..FULLBOX_EXTRA_SIZE + MD5_LEN]);
        let mut cursor = FULLBOX_EXTRA_SIZE + MD5_LEN;
        let mut input_4cc = [0u8; BOX_TYPE_SIZE];
        input_4cc.copy_from_slice(&body[cursor..cursor + BOX_TYPE_SIZE]);
        cursor += BOX_TYPE_SIZE;
        let sample_group = if input_4cc == INPUT_SGPD {
            need_in(body, cursor + BOX_TYPE_SIZE, "md5i grouping_type")?;
            let mut grouping_type = [0u8; BOX_TYPE_SIZE];
            grouping_type.copy_from_slice(&body[cursor..cursor + BOX_TYPE_SIZE]);
            cursor += BOX_TYPE_SIZE;
            let grouping_type_parameter = if flags & MD5I_GROUPING_PARAMETER != 0 {
                let value = be_u32(body, cursor, "md5i grouping_type_parameter")?;
                cursor += core::mem::size_of::<u32>();
                Some(value)
            } else {
                None
            };
            let num_entries = be_u32(body, cursor, "md5i num_entries")?;
            cursor += core::mem::size_of::<u32>();
            let count = usize::try_from(num_entries).map_err(|_| Error::InvalidInput("md5i"))?;
            crate::box_support::check_entry_count(
                body.len() - cursor,
                core::mem::size_of::<u32>(),
                count,
                "md5i indices",
            )?;
            let mut indices = Vec::with_capacity(count);
            for _ in 0..count {
                indices.push(be_u32(body, cursor, "md5i group_description_index")?);
                cursor += core::mem::size_of::<u32>();
            }
            Some(
                SampleGroupDigest::new(grouping_type, grouping_type_parameter)
                    .with_indices(indices),
            )
        } else {
            None
        };
        if cursor != body.len() {
            return Err(Error::InvalidInput("md5i trailing bytes"));
        }
        Ok(Self {
            version,
            flags,
            input_md5,
            input_4cc,
            sample_group,
        })
    }
}

impl broadcast_core::Serialize for Md5IntegrityBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let group = match &self.sample_group {
            Some(sample_group) => {
                BOX_TYPE_SIZE
                    + if sample_group.grouping_type_parameter.is_some() {
                        core::mem::size_of::<u32>()
                    } else {
                        0
                    }
                    + core::mem::size_of::<u32>()
                    + sample_group.group_description_indices.len() * core::mem::size_of::<u32>()
            }
            None => 0,
        };
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + MD5_LEN + BOX_TYPE_SIZE + group)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor =
            write_full_box_header(buf, &MD5I, need, self.version, self.flags, false)?;
        buf[cursor..cursor + MD5_LEN].copy_from_slice(&self.input_md5);
        cursor += MD5_LEN;
        buf[cursor..cursor + BOX_TYPE_SIZE].copy_from_slice(&self.input_4cc);
        cursor += BOX_TYPE_SIZE;
        if let Some(sample_group) = &self.sample_group {
            buf[cursor..cursor + BOX_TYPE_SIZE].copy_from_slice(&sample_group.grouping_type);
            cursor += BOX_TYPE_SIZE;
            if let Some(parameter) = sample_group.grouping_type_parameter {
                buf[cursor..cursor + core::mem::size_of::<u32>()]
                    .copy_from_slice(&parameter.to_be_bytes());
                cursor += core::mem::size_of::<u32>();
            }
            buf[cursor..cursor + core::mem::size_of::<u32>()].copy_from_slice(
                &fit_u32(sample_group.group_description_indices.len(), "md5i")?.to_be_bytes(),
            );
            cursor += core::mem::size_of::<u32>();
            for index in &sample_group.group_description_indices {
                buf[cursor..cursor + core::mem::size_of::<u32>()]
                    .copy_from_slice(&index.to_be_bytes());
                cursor += core::mem::size_of::<u32>();
            }
        }
        Ok(cursor)
    }
}

/// One box of a data-integrity (`mint`) item.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum IntegrityBox {
    /// An MD5 integrity box.
    Md5(Md5IntegrityBox),
    /// A box this crate does not model, preserved verbatim.
    Opaque(OpaqueBox),
}

/// The body of a data-integrity item (`mint`) — ISO/IEC 23008-12:2025 8.4.2.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct IntegrityItem {
    boxes: Vec<IntegrityBox>,
}

impl IntegrityItem {
    /// An item carrying these boxes.
    #[must_use]
    pub fn new(boxes: Vec<IntegrityBox>) -> Self {
        Self { boxes }
    }

    /// The boxes, in wire order.
    #[must_use]
    pub fn boxes(&self) -> &[IntegrityBox] {
        &self.boxes
    }

    /// Parse an item body.
    pub fn parse(body: &[u8]) -> Result<Self> {
        let mut boxes = Vec::new();
        let mut cursor = 0usize;
        while cursor < body.len() {
            let (item, consumed) = crate::box_types::parse_box(&body[cursor..])?;
            if consumed == 0 {
                return Err(Error::InvalidInput("integrity box length"));
            }
            if item.four_cc() == MD5I {
                boxes.push(IntegrityBox::Md5(Md5IntegrityBox::parse(
                    &body[cursor..cursor + consumed],
                )?));
            } else {
                let mut opaque = OpaqueBox::new(item.four_cc(), item.payload().to_vec());
                opaque.to_end = item.to_end();
                opaque.largesize = item.has_largesize();
                boxes.push(IntegrityBox::Opaque(opaque));
            }
            cursor += consumed;
        }
        Ok(Self { boxes })
    }

    /// The body length.
    pub fn serialized_len(&self) -> usize {
        self.boxes
            .iter()
            .map(|entry| match entry {
                IntegrityBox::Md5(value) => value.serialized_len(),
                IntegrityBox::Opaque(value) => value.serialized_len(),
            })
            .sum()
    }

    /// Write the item body.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = 0usize;
        for entry in &self.boxes {
            let written = match entry {
                IntegrityBox::Md5(value) => value.serialize_into(&mut buf[cursor..])?,
                IntegrityBox::Opaque(value) => value.serialize_into(&mut buf[cursor..])?,
            };
            cursor += written;
        }
        Ok(cursor)
    }
}

impl IntegrityItem {
    /// The body bytes, for a writer that adds an integrity item whole.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = alloc::vec![0u8; self.serialized_len()];
        let written = self.serialize_into(&mut out)?;
        out.truncate(written);
        Ok(out)
    }
}
