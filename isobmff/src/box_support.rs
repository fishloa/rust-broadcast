//! Shared box helper layer — ISO/IEC 14496-12:2026 §4.2, §8.5–§8.7.
//!
//! The flat-table parse/serialize helpers, the container child walker, the
//! typed-child ordering, and the opaque-box round-trip carrier reused by every
//! box module in this crate. Extracted from `init_segment` so a leaf box module
//! depends on this file rather than on the whole `moov` tree.

use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, LARGESIZE_SIZE,
    SIZE_INDICATES_LARGESIZE,
};
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_core::Serialize;
use broadcast_core::error::BufferTooShort;
use broadcast_core::len::need_out;
use broadcast_core::wire::{u24_at, u32_at};

/// Byte offset of a FullBox 24-bit `flags` field in a whole box.
pub const FULLBOX_FLAGS_WIRE_OFFSET: usize =
    BOX_HEADER_MIN_SIZE + crate::box_types::FULLBOX_FLAGS_OFFSET;
/// Width of a FullBox `version` field in bytes.
pub const FULLBOX_VERSION_LEN: usize = 1;
/// Width of a FullBox `flags` field in bytes.
pub const FULLBOX_FLAGS_WIDTH: usize = FULLBOX_EXTRA_SIZE - FULLBOX_VERSION_LEN;
/// Prefix length of a flat FullBox table: header + version/flags + count.
pub const FLAT_TABLE_HEADER_LEN: usize =
    BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>();
/// Byte offset of a flat table's `version` field.
pub const FLAT_TABLE_VERSION_OFFSET: usize = BOX_HEADER_MIN_SIZE;
/// Byte offset of a flat table's `flags` field.
pub const FLAT_TABLE_FLAGS_OFFSET: usize = FULLBOX_FLAGS_WIRE_OFFSET;
/// Byte offset of a flat table's `entry_count` field.
pub const FLAT_TABLE_COUNT_OFFSET: usize = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE;

/// Bound an untrusted wire entry `count` (the FullBox array-count fields of
/// ISO/IEC 14496-12:2026 §8.5.2.3, §8.6.1.2.3, §8.7.4.3, e.g.
/// `stsc`/`stsz`/`stco`/`co64`/`stss`/`stsd`/`dref`'s `entry_count`) against how many fixed-size `entry_len`-byte
/// records the bytes remaining after the count field could actually hold,
/// **before** it drives an allocation — the same discipline
/// [`crate::cenc::SampleEncryptionBox::parse_body`] applies against `senc`'s
/// `sample_count` (ISO/IEC 23001-7 §12.3). Without this, a 16-byte `co64`
/// declaring `count = 0xFFFFFFFF` reaches `Vec::with_capacity` asking for
/// ~32 GB up front — a remote denial of service, since every one of these
/// boxes is untrusted wire data (audit finding #4).
///
/// The per-entry parse loops already re-check their own bounds each
/// iteration and stop (rather than reading past the buffer) once bytes run
/// out, so capping the count fed to `Vec::with_capacity` changes no
/// successful parse's resulting `entries` — only how large the up-front
/// allocation is allowed to be.
pub fn bounded_entry_count(remaining: usize, entry_len: usize, count: usize) -> usize {
    if entry_len == 0 {
        // Zero-width entries: the buffer length bounds nothing, so never size
        // an up-front allocation from the wire count (#1130). The `Vec` still
        // grows normally if the entries really are that many.
        return count.min(ZERO_WIDTH_PREALLOC);
    }
    count.min(remaining / entry_len)
}

/// Largest up-front allocation (in entries) for a table whose entries are zero
/// bytes wide; see [`bounded_entry_count`].
pub const ZERO_WIDTH_PREALLOC: usize = 1024;

/// Reject a declared `count` that the body cannot hold, before its loop runs.
///
/// Every table's parse loop used to `break` when bytes ran out, so a truncated
/// `stsc`/`stco`/`stss`/`stsz` parsed to *fewer* entries than its own
/// `entry_count` claimed while still returning `Ok` — the sample tables then
/// disagreed with each other and demux produced misaligned samples with no
/// signal at all (audit r05-W13, second bullet; the W17/W25 class on the
/// output side). A declared count is a promise about the box's own size
/// (ISO/IEC 14496-12:2026 §8.7.4.3: `entry_count` "gives the number of entries
/// in the following table"), so a body that cannot hold it is a malformed box,
/// not a short one.
pub fn check_entry_count(
    remaining: usize,
    entry_len: usize,
    count: usize,
    what: &'static str,
) -> Result<()> {
    if entry_len == 0 {
        return Ok(());
    }
    let needed = count.saturating_mul(entry_len);
    if needed > remaining {
        return Err(Error::BufferTooShort(BufferTooShort {
            need: remaining.saturating_add(needed),
            have: remaining,
            what,
        }));
    }
    Ok(())
}

pub fn parse_flat_table<E>(
    bytes: &[u8],
    what: &'static str,
    count_field: &'static str,
    entry_len: usize,
    read: fn(&[u8]) -> E,
) -> Result<(u8, u32, Vec<E>)> {
    let compact = crate::box_types::compact_form(bytes)?;
    let bytes: &[u8] = &compact;
    if bytes.len() < FLAT_TABLE_HEADER_LEN {
        return Err(Error::BufferTooShort(BufferTooShort {
            need: FLAT_TABLE_HEADER_LEN,
            have: bytes.len(),
            what,
        }));
    }
    let flags = u24_at(bytes, FLAT_TABLE_FLAGS_OFFSET);
    let count = u32_at(bytes, FLAT_TABLE_COUNT_OFFSET) as usize;
    let body = &bytes[FLAT_TABLE_HEADER_LEN..];
    check_entry_count(body.len(), entry_len, count, count_field)?;
    let mut entries = Vec::with_capacity(bounded_entry_count(body.len(), entry_len, count));
    entries.extend(body.chunks_exact(entry_len).take(count).map(read));
    Ok((bytes[FLAT_TABLE_VERSION_OFFSET], flags, entries))
}

pub fn flat_table_len(entries: usize, entry_len: usize) -> usize {
    crate::box_types::box_len(FLAT_TABLE_HEADER_LEN + entries * entry_len)
}

pub fn serialize_flat_table<E>(
    buf: &mut [u8],
    fourcc: &[u8; BOX_TYPE_SIZE],
    (version, flags): (u8, u32),
    entries: &[E],
    entry_len: usize,
    mut write: impl FnMut(&E, &mut [u8]) -> Result<usize>,
) -> Result<usize> {
    let need = flat_table_len(entries.len(), entry_len);
    need_out(buf, need)?;
    let mut c = crate::box_types::write_full_box_header(buf, fourcc, need, version, flags, false)?;
    let entry_count = broadcast_core::len::fit_u32(entries.len(), "entry_count")?;
    buf[c..c + BOX_TYPE_SIZE].copy_from_slice(&entry_count.to_be_bytes());
    c += BOX_TYPE_SIZE;
    for entry in entries {
        c += write(entry, &mut buf[c..])?;
    }
    Ok(c)
}

/// The wire order of a container's children, recorded at parse time so the
/// serializer can reproduce it.
///
/// A container's typed fields have no order of their own, so serializing them
/// in declaration order silently *reorders* a file whose children were not
/// written that way — `moov` lost its `pssh` to after the `trak`s, and a
/// subtitle track's `sthd`/`nmhd` (which §7.3.4 places first in `minf`, and
/// which strict readers require there) ended up after `stbl` (audit r05-W12).
/// Every container built by this crate's own muxers writes the conventional
/// order, so the round-trip break showed up only on files other muxers wrote.
///
/// An empty `order` means "conventional order, once per populated typed
/// field" — the state a hand-built container is in; see
/// `resolve` (private)
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ChildOrder {
    /// One entry per child, in wire order.
    entries: Vec<[u8; BOX_TYPE_SIZE]>,
    /// `true` where the corresponding entry was stored in the container's
    /// `opaque` list rather than in a typed field.
    opaque: Vec<bool>,
}

impl ChildOrder {
    /// Record one typed child's four-CC, in wire order.
    pub fn push(&mut self, four_cc: [u8; BOX_TYPE_SIZE]) {
        self.entries.push(four_cc);
        self.opaque.push(false);
    }

    /// Record one opaque child's four-CC, in wire order.
    pub fn push_opaque(&mut self, four_cc: [u8; BOX_TYPE_SIZE]) {
        self.entries.push(four_cc);
        self.opaque.push(true);
    }

    /// Whether no wire order was recorded (a hand-built container).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Append one opaque child's four-CC to the recorded order.
    ///
    /// A caller that pushes onto a container's `opaque` list must call this
    /// too, and in the same position, or the two fall out of step: the
    /// serializer pairs the Nth `None` slot of the resolved order with the
    /// Nth opaque child. (A container whose `order` is empty emits its typed
    /// children first and then its opaque ones, so a hand-built container
    /// needs only the `opaque` push.)
    pub fn append_opaque(&mut self, four_cc: [u8; BOX_TYPE_SIZE]) {
        self.push_opaque(four_cc);
    }

    /// Resolve this order into the typed children to emit, in sequence, each
    /// named by its **position** in `typed_four_ccs`.
    ///
    /// `typed_four_ccs` lists the container's typed children in declaration
    /// order. Repeated four-CCs (`trak`) consume the typed entries in order; a
    /// four-CC with no remaining typed entry is an opaque child and resolves
    /// to `None`. Any typed child left over — one *added* to a parsed
    /// container, as `protect_init_segment` does when it rewrites a sample
    /// entry — is appended in declaration order, so a rewrite can never drop
    /// a child.
    pub fn resolve(&self, typed_four_ccs: &[&[u8; BOX_TYPE_SIZE]]) -> Vec<Option<usize>> {
        if self.entries.is_empty() {
            return (0..typed_four_ccs.len()).map(Some).collect();
        }
        let mut used = alloc::vec![false; typed_four_ccs.len()];
        let mut out: Vec<Option<usize>> = Vec::with_capacity(self.entries.len());
        for (i, four_cc) in self.entries.iter().enumerate() {
            if self.opaque.get(i).copied().unwrap_or(false) {
                out.push(None);
                continue;
            }
            // A *typed* wire entry whose typed child has since been removed
            // (as `progressive.rs` does when it drops `mvex`) emits nothing.
            let mut found = None;
            for (slot, cc) in typed_four_ccs.iter().enumerate() {
                if !used[slot] && four_cc == *cc {
                    found = Some(slot);
                    break;
                }
            }
            if let Some(slot) = found {
                used[slot] = true;
                out.push(Some(slot));
            }
        }
        for (slot, _) in typed_four_ccs.iter().enumerate() {
            if !used[slot] {
                out.push(Some(slot));
            }
        }
        out
    }
}

/// Serialize a container's children in wire order: `typed` maps a child slot
/// to its bytes, `opaque` supplies the children the container does not model.
///
/// Shared by every container in this module so the "typed field or opaque
/// blob, at the position the file had it" walk exists once.
fn serialize_children<E: From<Error>>(
    order: &[Option<usize>],
    typed: &[Vec<u8>],
    opaque: &[OpaqueBox],
    buf: &mut [u8],
    c: &mut usize,
) -> core::result::Result<(), E> {
    let mut opaque_index = 0usize;
    for (i, slot) in order.iter().enumerate() {
        // A `size == 0` opaque child may only use that form when nothing
        // follows it in the container (§4.2: "extends to the end of the
        // enclosing container"); otherwise it would swallow the sibling after
        // it — which is exactly what happens when `protect_init_segment`
        // appends a `pssh` after a parsed size-0 child (audit item 2).
        let is_last = i + 1 == order.len();
        match slot {
            Some(index) => match typed.get(*index) {
                Some(bytes) => {
                    buf[*c..*c + bytes.len()].copy_from_slice(bytes);
                    *c += bytes.len();
                }
                None => {
                    return Err(E::from(Error::InvalidInput(
                        "container child order names a typed child that is not present",
                    )));
                }
            },
            None => match opaque.get(opaque_index) {
                Some(o) => {
                    *c += o
                        .serialize_child_into(&mut buf[*c..], is_last)
                        .map_err(E::from)?
                }
                None => {
                    return Err(E::from(Error::InvalidInput(
                        "container child order names an opaque child that is not present",
                    )));
                }
            },
        }
        if slot.is_none() {
            opaque_index += 1;
        }
    }
    Ok(())
}

/// Serialize every typed child of a container to its own box bytes, in
/// declaration order, so [`serialize_children`] can place each at the wire
/// position [`ChildOrder::resolve`] chose for it.
fn typed_child_bytes<E>(
    children: &[&dyn SerializeBox<Error = E>],
) -> core::result::Result<Vec<Vec<u8>>, E> {
    children.iter().map(|b| b.serialize_box()).collect()
}

/// Serialize a container box: header, then its children in wire order.
///
/// Generic over the child serializer's error `E` so a container whose typed
/// children still live in a crate above this one (their `Serialize` impl uses
/// that crate's error) can use the same walk: `E: From<Error>` is the only
/// requirement, and the crate that owns the children supplies its own error.
pub fn serialize_container<E: From<Error>>(
    fourcc: &[u8; BOX_TYPE_SIZE],
    need: usize,
    order: &[Option<usize>],
    typed_children: &[&dyn SerializeBox<Error = E>],
    opaque: &[OpaqueBox],
    buf: &mut [u8],
) -> core::result::Result<usize, E> {
    need_out(buf, need).map_err(Error::from).map_err(E::from)?;
    let mut c = crate::box_types::write_box_header(buf, fourcc, need).map_err(E::from)?;
    let typed = typed_child_bytes(typed_children)?;
    serialize_children(order, &typed, opaque, buf, &mut c)?;
    if c != need {
        return Err(E::from(Error::InvalidInput(
            "container child order does not account for the whole box",
        )));
    }
    Ok(c)
}

/// A typed child of one of this module's containers, which the shared child
/// walker serializes positionally.
pub trait SerializeBox {
    /// The child's own error type (the `Serialize` impl it wraps).
    type Error;

    /// This child's own box bytes (header + body).
    fn serialize_box(&self) -> core::result::Result<Vec<u8>, Self::Error>;

    /// The length [`SerializeBox::serialize_box`] will produce.
    fn box_len(&self) -> usize;
}

impl<T: Serialize> SerializeBox for T {
    type Error = <T as Serialize>::Error;

    fn serialize_box(&self) -> core::result::Result<Vec<u8>, Self::Error> {
        self.try_to_bytes()
    }

    fn box_len(&self) -> usize {
        self.serialized_len()
    }
}

// ---------------------------------------------------------------------------
// OpaqueBox — round-trip unknown child boxes
// ---------------------------------------------------------------------------

/// An opaque box whose contents we do not parse — round-tripped verbatim.
/// Preserves the exact bytes so the real-fixture test stays byte-identical.
///
/// `data` is the payload *after* the 8-byte `size`+`type` header;
/// [`OpaqueBox::serialize_into`] writes `size`, `type`, then `data`. For a
/// `uuid` child that includes the 16-byte `usertype` (§4.2), which is what
/// keeps a PlayReady/ISMV `uuid` (and a Smooth `tfxd`) intact through a
/// rewrite (audit item 1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OpaqueBox {
    pub box_type: [u8; BOX_TYPE_SIZE],
    pub data: Vec<u8>,
    /// Whether the wire used the `size == 0` form ("extends to the end of the
    /// enclosing container", §4.2). Writing the length out instead makes the
    /// box semantically identical but changes its bytes, which breaks the
    /// real-fixture round-trip invariant.
    pub to_end: bool,
    /// Whether the wire used the `size == 1` + 64-bit `largesize` form
    /// (§4.2), so the round trip re-emits that form rather than the compact
    /// one.
    pub largesize: bool,
}

impl OpaqueBox {
    /// Build one from a child's four-CC and payload (both form flags clear —
    /// the box states its own 32-bit length).
    pub fn new(box_type: [u8; BOX_TYPE_SIZE], data: Vec<u8>) -> Self {
        Self {
            box_type,
            data,
            to_end: false,
            largesize: false,
        }
    }
}

impl OpaqueBox {
    /// The header this box writes. 8 bytes, or 16 under
    /// [`OpaqueBox::largesize`] (§4.2).
    fn header_len(&self) -> usize {
        BOX_HEADER_MIN_SIZE + if self.wide() { LARGESIZE_SIZE } else { 0 }
    }

    /// Whether the header uses the 64-bit `largesize` form: either the wire
    /// had it ([`OpaqueBox::largesize`]) or the box is too big for the 32-bit
    /// `size` field (§4.2, #1147) — never a silently wrapped size.
    fn wide(&self) -> bool {
        self.largesize
            || (BOX_HEADER_MIN_SIZE as u64 + self.data.len() as u64) > u64::from(u32::MAX)
    }

    /// The size this box declares.
    ///
    /// `is_last` says whether it is the final child of its container: only
    /// then may the `size == 0` form be used, because §4.2 defines it as
    /// "extends to the end of the enclosing container". Writing `0` for a
    /// child with a sibling after it would swallow that sibling.
    fn declared_size(&self, is_last: bool) -> Result<u64> {
        if self.to_end && is_last {
            return Ok(0);
        }
        let need = self.header_len() as u64 + self.data.len() as u64;
        Ok(need)
    }
}

impl Serialize for OpaqueBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        self.header_len() + self.data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.serialize_child_into(buf, true)
    }
}

impl OpaqueBox {
    /// Serialize as a child of a container, telling the writer whether it is
    /// the container's last child (see [`OpaqueBox::declared_size`]).
    fn serialize_child_into(&self, buf: &mut [u8], is_last: bool) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let size = self.declared_size(is_last)?;
        // The header is written here, not through `BoxHeader`'s own
        // `Serialize`: that impl requires `usertype` to be present for a
        // `uuid` box, and this payload already *contains* the usertype (§4.2
        // puts it after `size`/`type`/`largesize`) — carrying it twice would
        // duplicate it.
        let mut c = 0usize;
        if self.wide() {
            buf[c..c + BOX_TYPE_SIZE].copy_from_slice(&SIZE_INDICATES_LARGESIZE.to_be_bytes());
            c += BOX_TYPE_SIZE;
            buf[c..c + BOX_TYPE_SIZE].copy_from_slice(&self.box_type);
            c += BOX_TYPE_SIZE;
            buf[c..c + LARGESIZE_SIZE].copy_from_slice(&size.to_be_bytes());
            c += LARGESIZE_SIZE;
        } else {
            let size32 = broadcast_core::len::fit_u32(size as usize, "opaque box size")?;
            buf[c..c + BOX_TYPE_SIZE].copy_from_slice(&size32.to_be_bytes());
            c += BOX_TYPE_SIZE;
            buf[c..c + BOX_TYPE_SIZE].copy_from_slice(&self.box_type);
            c += BOX_TYPE_SIZE;
        }
        buf[c..c + self.data.len()].copy_from_slice(&self.data);
        Ok(c + self.data.len())
    }
}
