//! The QuickTime metadata item keys atom (`keys`).
//!
//! A full box: a 32-bit entry count, then each entry is a 32-bit `key_size`
//! (the whole entry, header included), a four-CC `key_namespace` and
//! `key_size - 8` bytes of `key_value`. Indexes into the table are one-based;
//! zero is reserved (Apple `Metadata item keys atom ('keys')`).
//!
//! The count is checked against the remaining bytes before anything is
//! allocated, so a table claiming 2^32 entries in a few bytes is a typed
//! error rather than an allocation the file dictates.

use alloc::string::String;
use alloc::vec::Vec;

use broadcast_core::len::{fit_u32, need_in, need_out};
use broadcast_core::{Parse, Serialize};

use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_OFFSET, box_len,
    write_box_header,
};
use crate::error::{Error, Result};

use super::U32_LEN;

/// The `keys` box type.
pub const KEYS: [u8; 4] = *b"keys";

/// The fixed part of one entry: `key_size` plus `key_namespace`.
const ENTRY_PREFIX_LEN: usize = U32_LEN + BOX_TYPE_SIZE;
/// The smallest entry: a prefix and an empty value.
const MIN_ENTRY_LEN: usize = ENTRY_PREFIX_LEN;
/// The full-box header plus the entry count.
const MIN_BODY_LEN: usize = FULLBOX_EXTRA_SIZE + U32_LEN;

/// One reverse-DNS key and the namespace it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct KeyEntry {
    /// The `key_namespace` four-CC (`mdta` for reverse-DNS keys).
    pub namespace: [u8; BOX_TYPE_SIZE],
    /// The key value itself.
    pub value: String,
}

/// The metadata item keys atom (`keys`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Keys {
    /// FullBox version.
    pub version: u8,
    /// FullBox flags.
    pub flags: u32,
    /// The entries in table order; entry 0 is one-based index 1.
    pub entries: Vec<KeyEntry>,
}

impl Keys {
    /// An empty `mdta` table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The key at a one-based index.
    #[must_use]
    pub fn key(&self, index: u32) -> Option<&str> {
        let zero_based = usize::try_from(index.checked_sub(1)?).ok()?;
        self.entries
            .get(zero_based)
            .map(|entry| entry.value.as_str())
    }

    /// Append `value` as an `mdta` key, returning its one-based index as the
    /// four-CC an `ilst` item is stored under. An existing key keeps its
    /// original index.
    pub fn push(&mut self, value: String) -> Result<[u8; BOX_TYPE_SIZE]> {
        if let Some(position) = self.entries.iter().position(|entry| entry.value == value) {
            let index = position + 1;
            let index = fit_u32(index, "keys index")?;
            return Ok(index.to_be_bytes());
        }
        let index = self.entries.len() + 1;
        let index = fit_u32(index, "keys index")?;
        self.entries.push(KeyEntry {
            namespace: *b"mdta",
            value,
        });
        Ok(index.to_be_bytes())
    }

    fn body_len(&self) -> usize {
        MIN_BODY_LEN
            + self
                .entries
                .iter()
                .map(|entry| ENTRY_PREFIX_LEN + entry.value.len())
                .sum::<usize>()
    }
}

impl<'a> Parse<'a> for Keys {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = crate::box_types::parse_box(bytes)?.0;
        if !item.header.box_type.is(&KEYS) {
            return Err(Error::UnexpectedBox { expected: "keys" });
        }
        Self::parse_body(item.body)
    }
}

impl Keys {
    /// Parse a `keys` from its body.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, MIN_BODY_LEN, "keys body")?;
        let version = body[0];
        let flags = broadcast_core::wire::u24_at(body, FULLBOX_FLAGS_OFFSET);
        let count = super::u32_at(body, FULLBOX_EXTRA_SIZE) as usize;
        let remaining = &body[MIN_BODY_LEN..];
        if count > remaining.len() / MIN_ENTRY_LEN {
            return Err(Error::InvalidInput(
                "keys entry count cannot fit the remaining bytes",
            ));
        }
        let mut entries = Vec::with_capacity(count);
        let mut cursor = MIN_BODY_LEN;
        for _ in 0..count {
            need_in(body, cursor + ENTRY_PREFIX_LEN, "keys entry")?;
            let key_size = super::u32_at(body, cursor) as usize;
            if key_size < MIN_ENTRY_LEN {
                return Err(Error::InvalidInput("keys entry is shorter than its prefix"));
            }
            let end = cursor
                .checked_add(key_size)
                .ok_or(Error::InvalidInput("keys entry size overflows"))?;
            need_in(body, end, "keys entry")?;
            let namespace = body[cursor + U32_LEN..cursor + ENTRY_PREFIX_LEN]
                .try_into()
                .map_err(|_| Error::InvalidInput("keys namespace"))?;
            let value_bytes = &body[cursor + ENTRY_PREFIX_LEN..end];
            let value = core::str::from_utf8(value_bytes)
                .map_err(|_| Error::InvalidInput("keys value is not UTF-8"))?
                .into();
            entries.push(KeyEntry { namespace, value });
            cursor = end;
        }
        if cursor != body.len() {
            return Err(Error::InvalidInput("keys has trailing bytes"));
        }
        Ok(Self {
            version,
            flags,
            entries,
        })
    }
}

impl Serialize for Keys {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.body_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &KEYS, need)?;
        buf[cursor] = self.version;
        cursor += 1;
        let flags = self.flags.to_be_bytes();
        buf[cursor..cursor + (FULLBOX_EXTRA_SIZE - 1)].copy_from_slice(&flags[1..]);
        cursor += FULLBOX_EXTRA_SIZE - 1;
        let count = fit_u32(self.entries.len(), "keys entry count")?;
        buf[cursor..cursor + U32_LEN].copy_from_slice(&count.to_be_bytes());
        cursor += U32_LEN;
        for entry in &self.entries {
            let size = fit_u32(ENTRY_PREFIX_LEN + entry.value.len(), "keys entry size")?;
            buf[cursor..cursor + U32_LEN].copy_from_slice(&size.to_be_bytes());
            cursor += U32_LEN;
            buf[cursor..cursor + BOX_TYPE_SIZE].copy_from_slice(&entry.namespace);
            cursor += BOX_TYPE_SIZE;
            buf[cursor..cursor + entry.value.len()].copy_from_slice(entry.value.as_bytes());
            cursor += entry.value.len();
        }
        Ok(cursor)
    }
}
