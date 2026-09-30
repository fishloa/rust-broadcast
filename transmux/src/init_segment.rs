//! Typed init-segment (moov) box tree — ISO/IEC 14496-12:2015 §8.2–8.7.
//!
//! Complete typed representation of the `moov` hierarchy found in ISOBMFF
//! initialisation segments. Container sizes are **computed** from children
//! (no `self.raw` passthrough). Unknown/opaque child boxes are preserved as
//! [`OpaqueBox`] for byte-exact round-trip.
//!
//! Reuses `TimeToSampleBox`, `CompositionOffsetBox`, `EditListBox` from
//! the `timing` module and `AVCSampleEntry` etc. from
//! `sample_entries`.

use crate::box_types::box_iter;
use crate::error::{Error, Result};
use crate::media::TrackEncryption;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

const BOX_HDR: usize = 8;
const FULL_HDR: usize = 4;

/// Bound an untrusted wire entry `count` (ISO/IEC 14496-12:2015 §8.1.1's
/// FullBox array-count fields, e.g. `stsc`/`stsz`/`stco`/`co64`/`stss`/`stsd`/
/// `dref`'s `entry_count`) against how many fixed-size `entry_len`-byte
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
pub(crate) fn bounded_entry_count(remaining: usize, entry_len: usize, count: usize) -> usize {
    if entry_len == 0 {
        return count;
    }
    count.min(remaining / entry_len)
}

/// Reject a declared `count` that the body cannot hold, before its loop runs.
///
/// Every table's parse loop used to `break` when bytes ran out, so a truncated
/// `stsc`/`stco`/`stss`/`stsz` parsed to *fewer* entries than its own
/// `entry_count` claimed while still returning `Ok` — the sample tables then
/// disagreed with each other and demux produced misaligned samples with no
/// signal at all (audit r05-W13, second bullet; the W17/W25 class on the
/// output side). A declared count is a promise about the box's own size
/// (ISO/IEC 14496-12:2015 §8.1.1: `entry_count` "gives the number of entries
/// in the following table"), so a body that cannot hold it is a malformed box,
/// not a short one.
pub(crate) fn check_entry_count(
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
        return Err(Error::BufferTooShort {
            need: remaining.saturating_add(needed),
            have: remaining,
            what,
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Container child walker — ISO/IEC 14496-12:2015 §4.2
// ---------------------------------------------------------------------------

/// Size of the smallest legal box header (`size` + `type`), §4.2.
const MIN_BOX_HEADER: usize = 8;

/// Size of the optional 64-bit `largesize` field that follows a `size` of 1
/// (§4.2).
const LARGESIZE_LEN: usize = 8;

/// One child box of a container, borrowed from the container's body.
pub(crate) struct ChildBox<'a> {
    /// The child's four-CC (`type`).
    pub four_cc: [u8; 4],
    /// The child's whole bytes, header included.
    pub bytes: &'a [u8],
    /// The child's *opaque payload*: the 16-byte `usertype` of a `uuid` box
    /// (ISO/IEC 14496-12:2015 §4.2) followed by its body — i.e. everything
    /// after `size`/`type`, **except** the 8 `largesize` bytes when the
    /// `size == 1` form was used.
    ///
    /// Deliberately **not** `BoxRef::body` (which starts after the *whole*
    /// header, usertype included): an opaque child is round-tripped by
    /// re-emitting `size`+`type`+payload, so dropping the usertype would
    /// corrupt every `uuid` box — the shape a PlayReady/ISMV `moov`/`trak`
    /// uuid and a Smooth `tfxd`/`tfrf` use (audit item 1). Keeping the
    /// `largesize` bytes would be just as wrong in the other direction: the
    /// serializer writes its own `size`/`type` (and its own `largesize` when
    /// [`ChildBox::largesize`] is set), so those 8 bytes would land in the
    /// body as junk (audit item 1, round 3).
    pub payload: &'a [u8],
    /// Whether this child used the `size == 0` form (§4.2) — see
    /// [`OpaqueBox::to_end`].
    pub to_end: bool,
    /// Whether this child used the `size == 1` + 64-bit `largesize` form
    /// (§4.2), so the round trip re-emits that form.
    pub largesize: bool,
}

/// Walk the children of a container body (the bytes after the container's own
/// 8-byte header), calling `f` for each in wire order.
///
/// Every existing container loop in this module read the four-byte `size`
/// itself and clamped with `size.min(remaining)`, `break`ing on anything
/// unusual. Three conformant shapes were mis-handled as a result (audit
/// r05-W13):
///
/// - a child written with 64-bit `largesize` (`size == 1`) — its leading `1`
///   failed the `size < 8` test and every *following* sibling was dropped;
/// - a truncated child — the clamp silently shortened it, so the container
///   parsed "successfully" with a child shorter than it claims;
/// - trailing bytes too short to hold a header — silently ignored.
///
/// `parse_box` handles largesize and usertype, and rejects a declared size
/// past the buffer, so the walk is size-driven and exact.
///
/// A `size == 0` child means "the box extends to the end of the enclosing
/// container" (ISO/IEC 14496-12:2015 §4.2 — for a child box, the end of its
/// parent, not of the file). `parse_box` implements exactly that against the
/// slice it is handed, which here *is* the container body, so such a child
/// consumes the remaining bytes and ends the walk by construction — there is
/// no following sibling to drop.
pub(crate) fn walk_children<'a, F>(body: &'a [u8], mut f: F) -> Result<()>
where
    F: FnMut(ChildBox<'a>) -> Result<()>,
{
    let mut off = 0usize;
    while off < body.len() {
        if body.len() - off < MIN_BOX_HEADER {
            return Err(Error::BufferTooShort {
                need: off + MIN_BOX_HEADER,
                have: body.len(),
                what: "container child header",
            });
        }
        let (bx, consumed) = crate::box_types::parse_box(&body[off..])?;
        let bytes = &body[off..off + consumed];
        // The payload starts after `size`+`type` plus the 8 `largesize` bytes
        // *only when that form was used*; the serializer re-emits the header
        // itself (see `ChildBox::payload`).
        let payload_start = MIN_BOX_HEADER
            + if bx.header.has_largesize() {
                LARGESIZE_LEN
            } else {
                0
            };
        f(ChildBox {
            four_cc: bx.header.box_type.0,
            bytes,
            payload: bytes.get(payload_start..).unwrap_or(&[]),
            to_end: bx.header.size == 0,
            largesize: bx.header.has_largesize(),
        })?;
        off += consumed;
    }
    Ok(())
}

/// The four-CC is the key a container's parse and serialize directions use to
/// line a child back up with the typed field (or opaque blob) it came from.
fn is_four_cc(four_cc: &[u8; 4], expected: &[u8; 4]) -> bool {
    four_cc == expected
}

/// The body bytes of a whole box (`size` + `type` + optional largesize/usertype
/// + body).
///
/// A container's own `Parse` impl used to slice `bytes[8..]` — correct only
/// for the 8-byte header form. A container written with 64-bit `largesize`
/// (`size == 1`, §4.2) has a 16-byte header, so the hardcoded slice swallowed
/// its first 8 body bytes and the child walk started mid-box (audit r05-W13).
fn box_body(bytes: &[u8]) -> Result<&[u8]> {
    Ok(crate::box_types::parse_box(bytes)?.0.body)
}

/// The wire order of a container's children, recorded at parse time so the
/// serializer can reproduce it.
///
/// A container's typed fields have no order of their own, so serializing them
/// in declaration order silently *reorders* a file whose children were not
/// written that way — `moov` lost its `pssh` to after the `trak`s, and a
/// subtitle track's `sthd`/`nmhd` (which §6.2.3 places first in `minf`, and
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
    entries: Vec<[u8; 4]>,
    /// `true` where the corresponding entry was stored in the container's
    /// `opaque` list rather than in a typed field.
    opaque: Vec<bool>,
}

impl ChildOrder {
    /// Record one typed child's four-CC, in wire order.
    fn push(&mut self, four_cc: [u8; 4]) {
        self.entries.push(four_cc);
        self.opaque.push(false);
    }

    /// Record one opaque child's four-CC, in wire order.
    fn push_opaque(&mut self, four_cc: [u8; 4]) {
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
    pub fn append_opaque(&mut self, four_cc: [u8; 4]) {
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
    fn resolve(&self, typed_four_ccs: &[&[u8; 4]]) -> Vec<Option<usize>> {
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
            if let Some((slot, _)) = typed_four_ccs
                .iter()
                .enumerate()
                .find(|(slot, cc)| !used[*slot] && is_four_cc(four_cc, cc))
            {
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
fn serialize_children(
    order: &[Option<usize>],
    typed: &[Vec<u8>],
    opaque: &[OpaqueBox],
    buf: &mut [u8],
    c: &mut usize,
) -> Result<()> {
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
                    return Err(Error::InvalidInput(
                        "container child order names a typed child that is not present",
                    ));
                }
            },
            None => match opaque.get(opaque_index) {
                Some(o) => *c += o.serialize_child_into(&mut buf[*c..], is_last)?,
                None => {
                    return Err(Error::InvalidInput(
                        "container child order names an opaque child that is not present",
                    ));
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
fn typed_child_bytes(children: &[&dyn SerializeBox]) -> Result<Vec<Vec<u8>>> {
    children
        .iter()
        .map(|b| b.serialize_box())
        .collect::<Result<Vec<_>>>()
}

/// A typed child of one of this module's containers, which the shared child
/// walker serializes positionally.
pub(crate) trait SerializeBox {
    /// This child's own box bytes (header + body).
    fn serialize_box(&self) -> Result<Vec<u8>>;

    /// The length [`SerializeBox::serialize_box`] will produce.
    fn box_len(&self) -> usize;
}

impl<T: Serialize<Error = Error>> SerializeBox for T {
    fn serialize_box(&self) -> Result<Vec<u8>> {
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
    pub box_type: [u8; 4],
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
    pub fn new(box_type: [u8; 4], data: Vec<u8>) -> Self {
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
        BOX_HDR + if self.largesize { LARGESIZE_LEN } else { 0 }
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
        if !self.largesize {
            // The compact form's 32-bit field must be able to hold it.
            broadcast_common::len::fit_u32(need as usize, "opaque box size")?;
        }
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
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let size = self.declared_size(is_last)?;
        // The header is written here, not through `BoxHeader`'s own
        // `Serialize`: that impl requires `usertype` to be present for a
        // `uuid` box, and this payload already *contains* the usertype (§4.2
        // puts it after `size`/`type`/`largesize`) — carrying it twice would
        // duplicate it.
        let mut c = 0usize;
        if self.largesize {
            buf[c..c + 4].copy_from_slice(&1u32.to_be_bytes());
            c += 4;
            buf[c..c + 4].copy_from_slice(&self.box_type);
            c += 4;
            buf[c..c + 8].copy_from_slice(&size.to_be_bytes());
            c += 8;
        } else {
            let size32 = broadcast_common::len::fit_u32(size as usize, "opaque box size")?;
            buf[c..c + 4].copy_from_slice(&size32.to_be_bytes());
            c += 4;
            buf[c..c + 4].copy_from_slice(&self.box_type);
            c += 4;
        }
        buf[c..c + self.data.len()].copy_from_slice(&self.data);
        Ok(c + self.data.len())
    }
}

// ---------------------------------------------------------------------------
// MovieHeaderBox — mvhd (ISO/IEC 14496-12:2015 §8.2.2)
// ---------------------------------------------------------------------------

/// Movie Header Box (`mvhd`) — §8.2.2.
/// v0: 32-bit creation_time, modification_time, duration.
/// v1: 64-bit equivalents.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MovieHeaderBox {
    pub version: u8,
    pub flags: u32,
    pub creation_time: u64,
    pub modification_time: u64,
    pub timescale: u32,
    pub duration: u64,
    pub rate: u32,
    pub volume: u16,
    pub matrix: [i32; 9],
    pub next_track_id: u32,
}

impl<'a> Parse<'a> for MovieHeaderBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 12 {
            return Err(Error::BufferTooShort {
                need: 12,
                have: bytes.len(),
                what: "mvhd",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        if ver == 0 {
            let need = 108;
            if bytes.len() < need {
                return Err(Error::BufferTooShort {
                    need,
                    have: bytes.len(),
                    what: "mvhd v0",
                });
            }
            Ok(Self {
                version: 0,
                flags,
                creation_time: u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]])
                    as u64,
                modification_time: u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]])
                    as u64,
                timescale: u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]),
                duration: u32::from_be_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]) as u64,
                rate: u32::from_be_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]),
                volume: u16::from_be_bytes([bytes[32], bytes[33]]),
                matrix: [
                    i32::from_be_bytes([bytes[44], bytes[45], bytes[46], bytes[47]]),
                    i32::from_be_bytes([bytes[48], bytes[49], bytes[50], bytes[51]]),
                    i32::from_be_bytes([bytes[52], bytes[53], bytes[54], bytes[55]]),
                    i32::from_be_bytes([bytes[56], bytes[57], bytes[58], bytes[59]]),
                    i32::from_be_bytes([bytes[60], bytes[61], bytes[62], bytes[63]]),
                    i32::from_be_bytes([bytes[64], bytes[65], bytes[66], bytes[67]]),
                    i32::from_be_bytes([bytes[68], bytes[69], bytes[70], bytes[71]]),
                    i32::from_be_bytes([bytes[72], bytes[73], bytes[74], bytes[75]]),
                    i32::from_be_bytes([bytes[76], bytes[77], bytes[78], bytes[79]]),
                ],
                next_track_id: u32::from_be_bytes([bytes[104], bytes[105], bytes[106], bytes[107]]),
            })
        } else {
            // v1 body (ISO/IEC 14496-12 §8.2.2.2): the 8-byte creation/
            // modification_time + duration widening (+12 bytes over v0) is
            // the ONLY size change — `pre_defined[6]` still ends at byte 116,
            // so `next_track_id` sits at `116..120` and the whole box is
            // **120** bytes, not 124 (issue #1015).
            let need = 120;
            if bytes.len() < need {
                return Err(Error::BufferTooShort {
                    need,
                    have: bytes.len(),
                    what: "mvhd v1",
                });
            }
            Ok(Self {
                version: 1,
                flags,
                creation_time: u64::from_be_bytes([
                    bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17], bytes[18],
                    bytes[19],
                ]),
                modification_time: u64::from_be_bytes([
                    bytes[20], bytes[21], bytes[22], bytes[23], bytes[24], bytes[25], bytes[26],
                    bytes[27],
                ]),
                timescale: u32::from_be_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]),
                duration: u64::from_be_bytes([
                    bytes[32], bytes[33], bytes[34], bytes[35], bytes[36], bytes[37], bytes[38],
                    bytes[39],
                ]),
                rate: u32::from_be_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
                volume: u16::from_be_bytes([bytes[44], bytes[45]]),
                matrix: [
                    i32::from_be_bytes([bytes[56], bytes[57], bytes[58], bytes[59]]),
                    i32::from_be_bytes([bytes[60], bytes[61], bytes[62], bytes[63]]),
                    i32::from_be_bytes([bytes[64], bytes[65], bytes[66], bytes[67]]),
                    i32::from_be_bytes([bytes[68], bytes[69], bytes[70], bytes[71]]),
                    i32::from_be_bytes([bytes[72], bytes[73], bytes[74], bytes[75]]),
                    i32::from_be_bytes([bytes[76], bytes[77], bytes[78], bytes[79]]),
                    i32::from_be_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]),
                    i32::from_be_bytes([bytes[84], bytes[85], bytes[86], bytes[87]]),
                    i32::from_be_bytes([bytes[88], bytes[89], bytes[90], bytes[91]]),
                ],
                next_track_id: u32::from_be_bytes([bytes[116], bytes[117], bytes[118], bytes[119]]),
            })
        }
    }
}

impl Serialize for MovieHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        // v1 is 120 bytes, not 124 — see the parse-side comment above (#1015).
        if self.version == 0 { 108 } else { 120 }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"mvhd");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        let (ct_sz, mt_sz, dur_sz) = if self.version == 0 {
            (4u8, 4u8, 4u8)
        } else {
            (8u8, 8u8, 8u8)
        };
        let write_u64 = |buf: &mut [u8], off: usize, sz: u8, v: u64| {
            if sz == 4 {
                buf[off..off + 4].copy_from_slice(&(v as u32).to_be_bytes());
            } else {
                buf[off..off + 8].copy_from_slice(&v.to_be_bytes());
            }
        };
        write_u64(buf, c, ct_sz, self.creation_time);
        c += ct_sz as usize;
        write_u64(buf, c, mt_sz, self.modification_time);
        c += mt_sz as usize;
        buf[c..c + 4].copy_from_slice(&self.timescale.to_be_bytes());
        c += 4;
        write_u64(buf, c, dur_sz, self.duration);
        c += dur_sz as usize;
        buf[c..c + 4].copy_from_slice(&self.rate.to_be_bytes());
        c += 4;
        buf[c..c + 2].copy_from_slice(&self.volume.to_be_bytes());
        c += 2;
        buf[c..c + 10].fill(0);
        c += 10; // reserved
        for &m in &self.matrix {
            buf[c..c + 4].copy_from_slice(&m.to_be_bytes());
            c += 4;
        }
        buf[c..c + 24].fill(0);
        c += 24; // pre_defined
        buf[c..c + 4].copy_from_slice(&self.next_track_id.to_be_bytes());
        c += 4;
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// TrackHeaderBox — tkhd (ISO/IEC 14496-12:2015 §8.2.3)
// ---------------------------------------------------------------------------

/// Track Header Box (`tkhd`) — §8.2.3.
/// v0: 32-bit creation_time, modification_time, duration.
/// v1: 64-bit equivalents.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackHeaderBox {
    pub version: u8,
    pub flags: u32,
    pub creation_time: u64,
    pub modification_time: u64,
    pub track_id: u32,
    pub duration: u64,
    pub layer: i16,
    pub alternate_group: i16,
    pub volume: i16,
    pub matrix: [i32; 9],
    pub width: u32,
    pub height: u32,
}

impl<'a> Parse<'a> for TrackHeaderBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 12 {
            return Err(Error::BufferTooShort {
                need: 12,
                have: bytes.len(),
                what: "tkhd",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        if ver == 0 {
            let need = 92;
            if bytes.len() < need {
                return Err(Error::BufferTooShort {
                    need,
                    have: bytes.len(),
                    what: "tkhd v0",
                });
            }
            let ct = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as u64;
            let mt = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) as u64;
            let tid = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
            let dur = u32::from_be_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]) as u64;
            Ok(Self {
                version: 0,
                flags,
                creation_time: ct,
                modification_time: mt,
                track_id: tid,
                duration: dur,
                layer: i16::from_be_bytes([bytes[40], bytes[41]]),
                alternate_group: i16::from_be_bytes([bytes[42], bytes[43]]),
                volume: i16::from_be_bytes([bytes[44], bytes[45]]),
                matrix: matrix_from_bytes(&bytes[48..84]),
                width: u32::from_be_bytes([bytes[84], bytes[85], bytes[86], bytes[87]]),
                height: u32::from_be_bytes([bytes[88], bytes[89], bytes[90], bytes[91]]),
            })
        } else {
            let need = 104;
            if bytes.len() < need {
                return Err(Error::BufferTooShort {
                    need,
                    have: bytes.len(),
                    what: "tkhd v1",
                });
            }
            // v1 body (ISO/IEC 14496-12 §8.3.2): track_ID(32) + a single
            // 32-bit `reserved` (not 8) sit between modification_time and
            // duration, so duration starts at byte 36, not 40 — every field
            // from `duration` on was previously read 4 bytes early
            // (issue #1016).
            let ct = u64::from_be_bytes(bytes[12..20].try_into().unwrap());
            let mt = u64::from_be_bytes(bytes[20..28].try_into().unwrap());
            let tid = u32::from_be_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]);
            let dur = u64::from_be_bytes(bytes[36..44].try_into().unwrap());
            Ok(Self {
                version: 1,
                flags,
                creation_time: ct,
                modification_time: mt,
                track_id: tid,
                duration: dur,
                layer: i16::from_be_bytes([bytes[52], bytes[53]]),
                alternate_group: i16::from_be_bytes([bytes[54], bytes[55]]),
                volume: i16::from_be_bytes([bytes[56], bytes[57]]),
                matrix: matrix_from_bytes(&bytes[60..96]),
                width: u32::from_be_bytes([bytes[96], bytes[97], bytes[98], bytes[99]]),
                height: u32::from_be_bytes([bytes[100], bytes[101], bytes[102], bytes[103]]),
            })
        }
    }
}

fn matrix_from_bytes(b: &[u8]) -> [i32; 9] {
    let mut m = [0i32; 9];
    for i in 0..9 {
        m[i] = i32::from_be_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
    }
    m
}

impl Serialize for TrackHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        if self.version == 0 { 92 } else { 104 }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"tkhd");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        if self.version == 0 {
            buf[c..c + 4].copy_from_slice(&(self.creation_time as u32).to_be_bytes());
            c += 4;
            buf[c..c + 4].copy_from_slice(&(self.modification_time as u32).to_be_bytes());
            c += 4;
            buf[c..c + 4].copy_from_slice(&self.track_id.to_be_bytes());
            c += 4;
            buf[c..c + 4].fill(0);
            c += 4; // reserved
            buf[c..c + 4].copy_from_slice(&(self.duration as u32).to_be_bytes());
            c += 4;
            buf[c..c + 8].fill(0);
            c += 8; // reserved * 2
            buf[c..c + 2].copy_from_slice(&self.layer.to_be_bytes());
            c += 2;
            buf[c..c + 2].copy_from_slice(&self.alternate_group.to_be_bytes());
            c += 2;
            buf[c..c + 2].copy_from_slice(&self.volume.to_be_bytes());
            c += 2;
            buf[c..c + 2].fill(0);
            c += 2; // reserved
        } else {
            buf[c..c + 8].copy_from_slice(&self.creation_time.to_be_bytes());
            c += 8;
            buf[c..c + 8].copy_from_slice(&self.modification_time.to_be_bytes());
            c += 8;
            buf[c..c + 4].copy_from_slice(&self.track_id.to_be_bytes());
            c += 4;
            buf[c..c + 4].fill(0);
            c += 4; // reserved
            buf[c..c + 8].copy_from_slice(&self.duration.to_be_bytes());
            c += 8;
            buf[c..c + 8].fill(0);
            c += 8; // reserved * 2
            buf[c..c + 2].copy_from_slice(&self.layer.to_be_bytes());
            c += 2;
            buf[c..c + 2].copy_from_slice(&self.alternate_group.to_be_bytes());
            c += 2;
            buf[c..c + 2].copy_from_slice(&self.volume.to_be_bytes());
            c += 2;
            buf[c..c + 2].fill(0);
            c += 2; // reserved
        }
        for &m in &self.matrix {
            buf[c..c + 4].copy_from_slice(&m.to_be_bytes());
            c += 4;
        }
        buf[c..c + 4].copy_from_slice(&self.width.to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.height.to_be_bytes());
        c += 4;
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// MediaHeaderBox — mdhd (ISO/IEC 14496-12:2015 §8.4.2)
// ---------------------------------------------------------------------------

/// Media Header Box (`mdhd`) — §8.4.2.
/// v0: 32-bit creation/modification/duration; v1: 64-bit.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MediaHeaderBox {
    pub version: u8,
    pub flags: u32,
    pub creation_time: u64,
    pub modification_time: u64,
    pub timescale: u32,
    pub duration: u64,
    pub language: u16,
}

impl<'a> Parse<'a> for MediaHeaderBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 12 {
            return Err(Error::BufferTooShort {
                need: 12,
                have: bytes.len(),
                what: "mdhd",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        if ver == 0 {
            if bytes.len() < 32 {
                return Err(Error::BufferTooShort {
                    need: 32,
                    have: bytes.len(),
                    what: "mdhd v0",
                });
            }
            Ok(Self {
                version: 0,
                flags,
                creation_time: u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]])
                    as u64,
                modification_time: u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]])
                    as u64,
                timescale: u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]),
                duration: u32::from_be_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]) as u64,
                language: u16::from_be_bytes([bytes[28], bytes[29]]),
            })
        } else {
            if bytes.len() < 44 {
                return Err(Error::BufferTooShort {
                    need: 44,
                    have: bytes.len(),
                    what: "mdhd v1",
                });
            }
            Ok(Self {
                version: 1,
                flags,
                creation_time: u64::from_be_bytes(bytes[12..20].try_into().unwrap()),
                modification_time: u64::from_be_bytes(bytes[20..28].try_into().unwrap()),
                timescale: u32::from_be_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]),
                duration: u64::from_be_bytes(bytes[32..40].try_into().unwrap()),
                language: u16::from_be_bytes([bytes[40], bytes[41]]),
            })
        }
    }
}

impl Serialize for MediaHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        if self.version == 0 { 32 } else { 44 }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"mdhd");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        if self.version == 0 {
            buf[c..c + 4].copy_from_slice(&(self.creation_time as u32).to_be_bytes());
            c += 4;
            buf[c..c + 4].copy_from_slice(&(self.modification_time as u32).to_be_bytes());
            c += 4;
            buf[c..c + 4].copy_from_slice(&self.timescale.to_be_bytes());
            c += 4;
            buf[c..c + 4].copy_from_slice(&(self.duration as u32).to_be_bytes());
            c += 4;
            buf[c..c + 2].copy_from_slice(&self.language.to_be_bytes());
            c += 2;
        } else {
            buf[c..c + 8].copy_from_slice(&self.creation_time.to_be_bytes());
            c += 8;
            buf[c..c + 8].copy_from_slice(&self.modification_time.to_be_bytes());
            c += 8;
            buf[c..c + 4].copy_from_slice(&self.timescale.to_be_bytes());
            c += 4;
            buf[c..c + 8].copy_from_slice(&self.duration.to_be_bytes());
            c += 8;
            buf[c..c + 2].copy_from_slice(&self.language.to_be_bytes());
            c += 2;
        }
        // quality(16), reserved = 0 — written, not just counted, so a reused
        // caller buffer cannot keep garbage there (r04-W45).
        buf[c..c + 2].fill(0);
        Ok(c + 2)
    }
}

// ---------------------------------------------------------------------------
// HandlerBox — hdlr (ISO/IEC 14496-12:2015 §8.4.3)
// ---------------------------------------------------------------------------

/// Handler Box (`hdlr`) — §8.4.3.
/// Declares the media handler type (`vide`, `soun`, etc.) and an optional name.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HandlerBox {
    pub version: u8,
    pub flags: u32,
    pub handler_type: [u8; 4],
    pub name: Vec<u8>,
}

impl<'a> Parse<'a> for HandlerBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 24 {
            return Err(Error::BufferTooShort {
                need: 24,
                have: bytes.len(),
                what: "hdlr",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        let handler_type = [bytes[16], bytes[17], bytes[18], bytes[19]];
        let name = if bytes.len() > 32 {
            bytes[32..].to_vec()
        } else {
            Vec::new()
        };
        Ok(Self {
            version: ver,
            flags,
            handler_type,
            name,
        })
    }
}

impl Serialize for HandlerBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + FULL_HDR + 20 + self.name.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"hdlr");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        // pre_defined(32) = 0, written rather than skipped (r04-W45).
        buf[c..c + 4].fill(0);
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.handler_type);
        c += 4;
        // reserved[3] (12 bytes) = 0, written rather than skipped (r04-W45).
        buf[c..c + 12].fill(0);
        c += 12;
        if !self.name.is_empty() {
            buf[c..c + self.name.len()].copy_from_slice(&self.name);
        }
        Ok(c + self.name.len())
    }
}

// ---------------------------------------------------------------------------
// VideoMediaHeaderBox — vmhd (ISO/IEC 14496-12:2015 §8.4.5.2)
// ---------------------------------------------------------------------------

/// Video Media Header Box (`vmhd`) — §8.4.5.2.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct VideoMediaHeaderBox {
    pub version: u8,
    pub flags: u32,
    pub graphicsmode: u16,
    pub opcolor: [u16; 3],
}

impl<'a> Parse<'a> for VideoMediaHeaderBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 20 {
            return Err(Error::BufferTooShort {
                need: 20,
                have: bytes.len(),
                what: "vmhd",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        Ok(Self {
            version: ver,
            flags,
            graphicsmode: u16::from_be_bytes([bytes[12], bytes[13]]),
            opcolor: [
                u16::from_be_bytes([bytes[14], bytes[15]]),
                u16::from_be_bytes([bytes[16], bytes[17]]),
                u16::from_be_bytes([bytes[18], bytes[19]]),
            ],
        })
    }
}

impl Serialize for VideoMediaHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + FULL_HDR + 8
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"vmhd");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        buf[c..c + 2].copy_from_slice(&self.graphicsmode.to_be_bytes());
        c += 2;
        buf[c..c + 2].copy_from_slice(&self.opcolor[0].to_be_bytes());
        c += 2;
        buf[c..c + 2].copy_from_slice(&self.opcolor[1].to_be_bytes());
        c += 2;
        buf[c..c + 2].copy_from_slice(&self.opcolor[2].to_be_bytes());
        Ok(c + 2)
    }
}

// ---------------------------------------------------------------------------
// SoundMediaHeaderBox — smhd (ISO/IEC 14496-12:2015 §8.4.5.3)
// ---------------------------------------------------------------------------

/// Sound Media Header Box (`smhd`) — §8.4.5.3.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SoundMediaHeaderBox {
    pub version: u8,
    pub flags: u32,
    pub balance: i16,
}

impl<'a> Parse<'a> for SoundMediaHeaderBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 16 {
            return Err(Error::BufferTooShort {
                need: 16,
                have: bytes.len(),
                what: "smhd",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        Ok(Self {
            version: ver,
            flags,
            balance: i16::from_be_bytes([bytes[12], bytes[13]]),
        })
    }
}

impl Serialize for SoundMediaHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + FULL_HDR + 4
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"smhd");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        buf[c..c + 2].copy_from_slice(&self.balance.to_be_bytes());
        c += 2;
        // reserved(16) = 0, written rather than skipped (r04-W45).
        buf[c..c + 2].fill(0);
        c += 2;
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// DataReferenceBox — dref (ISO/IEC 14496-12:2015 §8.7.2)
// ---------------------------------------------------------------------------

/// Data Reference Box (`dref`) — §8.7.2.
/// Contains a list of DataEntryUrlBox entries.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataReferenceBox {
    pub version: u8,
    pub flags: u32,
    pub entries: Vec<DataEntryUrlBox>,
}

impl<'a> Parse<'a> for DataReferenceBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 16 {
            return Err(Error::BufferTooShort {
                need: 16,
                have: bytes.len(),
                what: "dref",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        let count = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        check_entry_count(bytes.len().saturating_sub(16), 8, count, "dref.entry_count")?;
        let mut entries = Vec::with_capacity(bounded_entry_count(
            bytes.len().saturating_sub(16),
            8,
            count,
        ));
        let mut off = 16usize;
        for _ in 0..count {
            if off + 8 > bytes.len() {
                break;
            }
            let sz =
                u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
                    as usize;
            if sz < 8 {
                break;
            }
            let end = (off + sz).min(bytes.len());
            entries.push(DataEntryUrlBox::parse(&bytes[off..end])?);
            off += sz;
        }
        Ok(Self {
            version: ver,
            flags,
            entries,
        })
    }
}

impl Serialize for DataReferenceBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR + FULL_HDR + 4;
        for e in &self.entries {
            n += e.serialized_len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"dref");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        let entry_count = broadcast_common::len::fit_u32(self.entries.len(), "entry_count")?;
        buf[c..c + 4].copy_from_slice(&entry_count.to_be_bytes());
        c += 4;
        for entry in &self.entries {
            c += entry.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// DataEntryUrlBox — url  (ISO/IEC 14496-12:2015 §8.7.2)
// ---------------------------------------------------------------------------

/// Data Entry URL Box (`url `) — §8.7.2.
/// When `flags & 1` is set, the media data is in this file (self-contained).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataEntryUrlBox {
    pub version: u8,
    pub flags: u32,
    pub location: Vec<u8>,
}

impl<'a> Parse<'a> for DataEntryUrlBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 12 {
            return Err(Error::BufferTooShort {
                need: 12,
                have: bytes.len(),
                what: "url",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        let location = if bytes.len() > 12 {
            bytes[12..].to_vec()
        } else {
            Vec::new()
        };
        Ok(Self {
            version: ver,
            flags,
            location,
        })
    }
}

impl Serialize for DataEntryUrlBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + FULL_HDR + self.location.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"url ");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        if !self.location.is_empty() {
            buf[c..c + self.location.len()].copy_from_slice(&self.location);
            c += self.location.len();
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SampleToChunkBox — stsc (ISO/IEC 14496-12:2015 §8.7.4)
// ---------------------------------------------------------------------------

/// Entry in the stsc chunk-to-sample table (§8.7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StscEntry {
    pub first_chunk: u32,
    pub samples_per_chunk: u32,
    pub sample_description_index: u32,
}

/// Sample To Chunk Box (`stsc`) — §8.7.4.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleToChunkBox {
    pub version: u8,
    pub flags: u32,
    pub entries: Vec<StscEntry>,
}

impl<'a> Parse<'a> for SampleToChunkBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 16 {
            return Err(Error::BufferTooShort {
                need: 16,
                have: bytes.len(),
                what: "stsc",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        let count = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        check_entry_count(
            bytes.len().saturating_sub(16),
            12,
            count,
            "SampleToChunkBox.entry_count",
        )?;
        let mut entries = Vec::with_capacity(bounded_entry_count(
            bytes.len().saturating_sub(16),
            12,
            count,
        ));
        let mut off = 16usize;
        for _ in 0..count {
            if off + 12 > bytes.len() {
                break;
            }
            entries.push(StscEntry {
                first_chunk: u32::from_be_bytes([
                    bytes[off],
                    bytes[off + 1],
                    bytes[off + 2],
                    bytes[off + 3],
                ]),
                samples_per_chunk: u32::from_be_bytes([
                    bytes[off + 4],
                    bytes[off + 5],
                    bytes[off + 6],
                    bytes[off + 7],
                ]),
                sample_description_index: u32::from_be_bytes([
                    bytes[off + 8],
                    bytes[off + 9],
                    bytes[off + 10],
                    bytes[off + 11],
                ]),
            });
            off += 12;
        }
        Ok(Self {
            version: ver,
            flags,
            entries,
        })
    }
}

impl Serialize for SampleToChunkBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + FULL_HDR + 4 + self.entries.len() * 12
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"stsc");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        let entry_count = broadcast_common::len::fit_u32(self.entries.len(), "entry_count")?;
        buf[c..c + 4].copy_from_slice(&entry_count.to_be_bytes());
        c += 4;
        for entry in &self.entries {
            buf[c..c + 4].copy_from_slice(&entry.first_chunk.to_be_bytes());
            buf[c + 4..c + 8].copy_from_slice(&entry.samples_per_chunk.to_be_bytes());
            buf[c + 8..c + 12].copy_from_slice(&entry.sample_description_index.to_be_bytes());
            c += 12;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SampleSizeBox — stsz (ISO/IEC 14496-12:2015 §8.7.3)
// ---------------------------------------------------------------------------

/// Sample Size Box (`stsz`) — §8.7.3.
/// If `sample_size > 0`, all samples have that uniform size and the entries vec
/// is empty. If `sample_size == 0`, entries contains per-sample sizes.
///
/// `sample_count` (wire `unsigned int(32)`) is carried explicitly for the
/// same reason as `SampleAuxInfoSizesBox::sample_count` (issue #1013): when
/// `sample_size != 0` the wire count is real but `entries` stays empty, so
/// `entries.len()` alone cannot stand in for it without collapsing every
/// uniform-size track to "0 samples" on re-serialize (issue #1018).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleSizeBox {
    pub version: u8,
    pub flags: u32,
    pub sample_size: u32,
    pub sample_count: u32,
    pub entries: Vec<u32>,
}

impl<'a> Parse<'a> for SampleSizeBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 20 {
            return Err(Error::BufferTooShort {
                need: 20,
                have: bytes.len(),
                what: "stsz",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        let sample_size = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let count = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) as usize;
        if sample_size == 0 {
            check_entry_count(
                bytes.len().saturating_sub(20),
                4,
                count,
                "SampleSizeBox sample_count",
            )?;
        }
        // `entries` is only ever populated below when `sample_size == 0`
        // (per-sample sizes) — a nonzero uniform `sample_size` means the loop
        // never runs, so a wire `count` in that branch mustn't drive any
        // allocation at all, not merely a bounded one.
        let capacity = if sample_size == 0 {
            bounded_entry_count(bytes.len().saturating_sub(20), 4, count)
        } else {
            0
        };
        let mut entries = Vec::with_capacity(capacity);
        if sample_size == 0 {
            let mut off = 20usize;
            for _ in 0..count {
                if off + 4 > bytes.len() {
                    break;
                }
                entries.push(u32::from_be_bytes([
                    bytes[off],
                    bytes[off + 1],
                    bytes[off + 2],
                    bytes[off + 3],
                ]));
                off += 4;
            }
        }
        Ok(Self {
            version: ver,
            flags,
            sample_size,
            sample_count: count as u32,
            entries,
        })
    }
}

impl Serialize for SampleSizeBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let count = if self.sample_size == 0 {
            self.entries.len()
        } else {
            0
        };
        BOX_HDR + FULL_HDR + 8 + count * 4
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.sample_size == 0 && self.entries.len() != self.sample_count as usize {
            return Err(Error::InvalidInput(
                "stsz: entries.len() must equal sample_count when sample_size == 0",
            ));
        }
        let count = if self.sample_size == 0 {
            self.entries.len()
        } else {
            0
        };
        let need = BOX_HDR + FULL_HDR + 8 + count * 4;
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"stsz");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        buf[c..c + 4].copy_from_slice(&self.sample_size.to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.sample_count.to_be_bytes());
        c += 4;
        for &sz in &self.entries {
            buf[c..c + 4].copy_from_slice(&sz.to_be_bytes());
            c += 4;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// ChunkOffsetBox — stco (ISO/IEC 14496-12:2015 §8.7.5)
// ---------------------------------------------------------------------------

/// Chunk Offset Box (`stco`) — §8.7.5 (32-bit offsets).
/// 64-bit offsets via `co64` are captured as an opaque box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ChunkOffsetBox {
    pub version: u8,
    pub flags: u32,
    pub entries: Vec<u32>,
}

impl<'a> Parse<'a> for ChunkOffsetBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 16 {
            return Err(Error::BufferTooShort {
                need: 16,
                have: bytes.len(),
                what: "stco",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        let count = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        check_entry_count(
            bytes.len().saturating_sub(16),
            4,
            count,
            "ChunkOffsetBox.entry_count",
        )?;
        let mut entries = Vec::with_capacity(bounded_entry_count(
            bytes.len().saturating_sub(16),
            4,
            count,
        ));
        let mut off = 16usize;
        for _ in 0..count {
            if off + 4 > bytes.len() {
                break;
            }
            entries.push(u32::from_be_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ]));
            off += 4;
        }
        Ok(Self {
            version: ver,
            flags,
            entries,
        })
    }
}

impl Serialize for ChunkOffsetBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + FULL_HDR + 4 + self.entries.len() * 4
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"stco");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        let entry_count = broadcast_common::len::fit_u32(self.entries.len(), "entry_count")?;
        buf[c..c + 4].copy_from_slice(&entry_count.to_be_bytes());
        c += 4;
        for entry in &self.entries {
            buf[c..c + 4].copy_from_slice(&entry.to_be_bytes());
            c += 4;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// ChunkLargeOffsetBox — co64 (ISO/IEC 14496-12:2015 §8.7.5)
// ---------------------------------------------------------------------------

/// Chunk Large Offset Box (`co64`) — §8.7.5 (64-bit chunk offsets).
///
/// The 64-bit sibling of [`ChunkOffsetBox`], used when any chunk offset exceeds
/// [`u32::MAX`]. Same semantics; each entry is an absolute byte offset into the
/// file of the first sample in a chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ChunkLargeOffsetBox {
    pub version: u8,
    pub flags: u32,
    pub entries: Vec<u64>,
}

impl<'a> Parse<'a> for ChunkLargeOffsetBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 16 {
            return Err(Error::BufferTooShort {
                need: 16,
                have: bytes.len(),
                what: "co64",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        let count = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        check_entry_count(
            bytes.len().saturating_sub(16),
            8,
            count,
            "ChunkLargeOffsetBox.entry_count",
        )?;
        let mut entries = Vec::with_capacity(bounded_entry_count(
            bytes.len().saturating_sub(16),
            8,
            count,
        ));
        let mut off = 16usize;
        for _ in 0..count {
            if off + 8 > bytes.len() {
                break;
            }
            entries.push(u64::from_be_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
                bytes[off + 4],
                bytes[off + 5],
                bytes[off + 6],
                bytes[off + 7],
            ]));
            off += 8;
        }
        Ok(Self {
            version: ver,
            flags,
            entries,
        })
    }
}

impl Serialize for ChunkLargeOffsetBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + FULL_HDR + 4 + self.entries.len() * 8
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"co64");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        let entry_count = broadcast_common::len::fit_u32(self.entries.len(), "entry_count")?;
        buf[c..c + 4].copy_from_slice(&entry_count.to_be_bytes());
        c += 4;
        for entry in &self.entries {
            buf[c..c + 8].copy_from_slice(&entry.to_be_bytes());
            c += 8;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SyncSampleBox — stss (ISO/IEC 14496-12:2015 §8.6.2)
// ---------------------------------------------------------------------------

/// Sync Sample Box (`stss`) — §8.6.2.
///
/// Lists the 1-based indices of the sync (random-access) samples. If the box is
/// absent every sample is a sync sample; when present it is the exhaustive list
/// of random-access points.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SyncSampleBox {
    pub version: u8,
    pub flags: u32,
    /// 1-based sample numbers that are sync samples.
    pub entries: Vec<u32>,
}

impl<'a> Parse<'a> for SyncSampleBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 16 {
            return Err(Error::BufferTooShort {
                need: 16,
                have: bytes.len(),
                what: "stss",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        let count = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        check_entry_count(
            bytes.len().saturating_sub(16),
            4,
            count,
            "SyncSampleBox.entry_count",
        )?;
        let mut entries = Vec::with_capacity(bounded_entry_count(
            bytes.len().saturating_sub(16),
            4,
            count,
        ));
        let mut off = 16usize;
        for _ in 0..count {
            if off + 4 > bytes.len() {
                break;
            }
            entries.push(u32::from_be_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ]));
            off += 4;
        }
        Ok(Self {
            version: ver,
            flags,
            entries,
        })
    }
}

impl Serialize for SyncSampleBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + FULL_HDR + 4 + self.entries.len() * 4
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"stss");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        let entry_count = broadcast_common::len::fit_u32(self.entries.len(), "entry_count")?;
        buf[c..c + 4].copy_from_slice(&entry_count.to_be_bytes());
        c += 4;
        for entry in &self.entries {
            buf[c..c + 4].copy_from_slice(&entry.to_be_bytes());
            c += 4;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SampleDescriptionBox — stsd (ISO/IEC 14496-12:2015 §8.5.2)
// ---------------------------------------------------------------------------

/// The `entry_version`-1 ("AudioSampleEntryV1") form of an audio sample entry —
/// ISO/IEC 14496-12:2015 §12.2.3.2, as amended by Amd 1:2017.
///
/// A plain `AudioSampleEntry` carries the sampling rate as a 16.16 fixed-point
/// `u32`, so its integer part is limited to 65535 Hz. A rate above that (88.2,
/// 96, 176.4, 192 kHz, …) cannot be represented and is silently truncated:
/// `192000 << 16` keeps only `0xEE000000`, whose integer part reads back as
/// 60928 Hz (`96000` gives `0x77000000` -> 30464 Hz). The spec's answer
/// is to use the v1 entry, whose `entry_version` field is 1, whose `samplerate`
/// field is the placeholder `1 << 16`, and which carries the real rate in a
/// [`SamplingRateBox`]. Such an entry must sit in an `stsd` with
/// `version == 1`.
///
/// This type exists so the constants have one home; the flag is carried per
/// sample entry as its `entry_version` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioSampleEntryV1;

impl AudioSampleEntryV1 {
    /// The `entry_version` value that marks an `AudioSampleEntryV1`
    /// (ISO/IEC 14496-12 §12.2.3.2: "must be 1").
    pub const ENTRY_VERSION: u16 = 1;
    /// The `stsd` `version` an `AudioSampleEntryV1` requires — "must be in an
    /// stsd with version == 1" (ISO/IEC 14496-12 §12.2.3.2).
    pub const STSD_VERSION: u8 = 1;
    /// The placeholder written into the v1 `samplerate` field:
    /// `template unsigned int(32) samplerate = 1<<16;` (§12.2.3.2).
    pub const SAMPLERATE_PLACEHOLDER: u32 = 1 << 16;

    /// Whether an `AudioSampleEntry`'s 16.16 fixed-point `samplerate` field can
    /// carry `rate` without losing its integer part.
    ///
    /// The integer part is the top 16 bits, so any rate above `u16::MAX` needs
    /// the v1 form (ISO/IEC 14496-12 §12.2.3 vs §12.2.3.2).
    pub fn rate_fits_v0(rate: u32) -> bool {
        rate <= u32::from(u16::MAX)
    }
}

/// Sampling Rate Box (`srat`) — ISO/IEC 14496-12:2015 §12.2.3.1 (as amended by
/// Amd 1:2017).
///
/// Carries the *actual* sampling rate of an audio track whose rate does not fit
/// the 16.16 fixed-point `samplerate` field of an AudioSampleEntry. It is valid
/// only inside an `AudioSampleEntryV1` (whose `entry_version` is 1 and which
/// must sit in an `stsd` with `version == 1`), where it overrides the
/// `samplerate` field — that field is then written as `1 << 16` (`0x00010000`).
///
/// ```text
/// aligned(8) class SamplingRateBox extends FullBox('srat') {
///     unsigned int(32) sampling_rate;
/// }
/// ```
///
/// 88.2, 96, 176.4 and 192 kHz tracks use this box: `96000 << 16` truncated to
/// the 16.16 field's integer part reads back as 30464 Hz, and `192000 << 16` as
/// 60928 Hz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SamplingRateBox {
    /// Version of the `FullBox` (`srat` is defined with version 0, flags 0).
    pub version: u8,
    pub flags: u32,
    /// Actual sampling rate in Hz.
    pub sampling_rate: u32,
}

impl SamplingRateBox {
    /// FourCC of this box.
    pub const FOURCC: [u8; 4] = *b"srat";
    /// Box size in bytes (8-byte header + FullBox header + one `u32`).
    pub const SIZE: usize = BOX_HDR + FULL_HDR + 4;

    /// Build a version-0/flags-0 `srat` for the given rate.
    pub fn new(sampling_rate: u32) -> Self {
        Self {
            version: 0,
            flags: 0,
            sampling_rate,
        }
    }
}

impl Serialize for SamplingRateBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        Self::SIZE
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = Self::SIZE;
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0..4].copy_from_slice(&(need as u32).to_be_bytes());
        buf[4..8].copy_from_slice(&Self::FOURCC);
        buf[8] = self.version;
        let fb = self.flags.to_be_bytes();
        buf[9..12].copy_from_slice(&fb[1..]);
        buf[12..16].copy_from_slice(&self.sampling_rate.to_be_bytes());
        Ok(need)
    }
}

impl<'a> Parse<'a> for SamplingRateBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < Self::SIZE {
            return Err(Error::BufferTooShort {
                need: Self::SIZE,
                have: bytes.len(),
                what: "srat",
            });
        }
        // The four-CC is validated, not assumed: this parser is also reached
        // from a raw child-box walk, where the bytes preceding it may belong to
        // any box at all (a wrong box parsed as an `srat` would silently supply
        // a bogus rate).
        if bytes[4..8] != Self::FOURCC {
            return Err(Error::InvalidInput("SamplingRateBox: not an 'srat' box"));
        }
        let version = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        let sampling_rate = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        Ok(Self {
            version,
            flags,
            sampling_rate,
        })
    }
}

/// Read the `srat` body out of an audio sample entry's config boxes, if any.
///
/// A `SamplingRateBox` may appear at most once and takes precedence over the
/// entry's 16.16 `samplerate` field (ISO/IEC 14496-12 §12.2.3.1).
///
/// Returns `None` — leaving the entry's own 16.16 field to stand — when the box
/// is absent, fails to parse, or declares `sampling_rate == 0`. A zero rate is
/// not a rate: honouring it would *replace* a perfectly good 16.16 field with
/// "0 Hz", which is strictly worse than ignoring the malformed box.
pub fn sampling_rate_override(config_boxes: &[OpaqueBox]) -> Option<u32> {
    let b = config_boxes
        .iter()
        .find(|b| b.box_type == SamplingRateBox::FOURCC)?;
    // The body was captured without its 8-byte header; rebuild the FullBox
    // bytes so the typed parser sees a complete box.
    let mut full = vec![0u8; 8 + b.data.len()];
    full[4..8].copy_from_slice(&b.box_type);
    full[8..].copy_from_slice(&b.data);
    let srat = SamplingRateBox::parse(&full).ok()?;
    if srat.sampling_rate == 0 {
        return None;
    }
    Some(srat.sampling_rate)
}

/// AAC audio sample entry (`mp4a`) — ISO/IEC 14496-12:2015 §12.2.3.
///
/// Wire layout (32 bytes before optional config children):
/// - SampleEntry: reserved(6) + data_reference_index(16) = 8 bytes
/// - AudioSampleEntry reserved `[2]`: 8 bytes
/// - channelcount(16) + samplesize(16) + predefined(16) + reserved(16) + samplerate(32) = 16 bytes
/// - then config boxes (esds, etc.)
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Mp4aSampleEntry {
    /// The FourCC of this sample entry — `mp4a`, or `enca` when the track is
    /// CENC-protected (the `sinf` child then carries the real codec under
    /// `frma`). Previously hard-coded to `mp4a` on serialize, which silently
    /// re-labelled a protected audio track as clear on any parse -> serialize
    /// round trip (issue #1017).
    pub codec_type: [u8; 4],
    /// `entry_version`: 0 for a plain `AudioSampleEntry` (§12.2.3), 1 for an
    /// [`AudioSampleEntryV1`] (§12.2.3.2) whose real rate lives in a
    /// [`SamplingRateBox`] among `config_boxes`, or any other value a
    /// QuickTime sound description carried.
    ///
    /// Stored and written back **verbatim** — the field used to be re-derived
    /// as 0-or-1 on serialize, which silently rewrote a QuickTime v2 (`02 00`)
    /// entry as v0 on any parse -> serialize round trip.
    pub entry_version: u16,
    /// The six bytes following `entry_version` in the fixed 8-byte block
    /// (QuickTime's `revision_level` + `vendor`, ISO/IEC 14496-12's
    /// `reserved[3]`). Written back verbatim so a real file round-trips.
    pub reserved_1: [u8; 6],
    pub data_reference_index: u16,
    pub channelcount: u16,
    pub samplesize: u16,
    /// The `pre_defined` + `reserved` pair (ISO/IEC 14496-12 §12.2.3) — or, in
    /// a QuickTime sound description, `compression_ID` + `packet_size`.
    /// QuickTime's VBR marker is `compression_ID = -2` (`ff fe`), so this is
    /// not always zero and must survive a round trip.
    pub compression_id_and_packet_size: [u8; 4],
    pub samplerate: u32,
    pub config_boxes: Vec<OpaqueBox>,
}

// ---------------------------------------------------------------------------
// Ac3SampleEntry (ac-3) — ETSI TS 102 366 §F.3
// ---------------------------------------------------------------------------

/// AC-3 audio sample entry (`ac-3`) — ETSI TS 102 366 §F.3.
///
/// Same AudioSampleEntry fixed fields as [`Mp4aSampleEntry`], then a `dac3`
/// config box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Ac3SampleEntry {
    /// `entry_version`: 0 for a plain `AudioSampleEntry` (§12.2.3), 1 for an
    /// [`AudioSampleEntryV1`] (§12.2.3.2) whose real rate lives in a
    /// [`SamplingRateBox`] among `config_boxes`, or any other value a
    /// QuickTime sound description carried.
    ///
    /// Stored and written back **verbatim** — the field used to be re-derived
    /// as 0-or-1 on serialize, which silently rewrote a QuickTime v2 (`02 00`)
    /// entry as v0 on any parse -> serialize round trip.
    pub entry_version: u16,
    /// The six bytes following `entry_version` in the fixed 8-byte block
    /// (QuickTime's `revision_level` + `vendor`, ISO/IEC 14496-12's
    /// `reserved[3]`). Written back verbatim so a real file round-trips.
    pub reserved_1: [u8; 6],
    pub data_reference_index: u16,
    pub channelcount: u16,
    pub samplesize: u16,
    /// The `pre_defined` + `reserved` pair (ISO/IEC 14496-12 §12.2.3) — or, in
    /// a QuickTime sound description, `compression_ID` + `packet_size`.
    /// QuickTime's VBR marker is `compression_ID = -2` (`ff fe`), so this is
    /// not always zero and must survive a round trip.
    pub compression_id_and_packet_size: [u8; 4],
    pub samplerate: u32,
    pub config_boxes: Vec<OpaqueBox>,
}

impl Serialize for Ac3SampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        audio_sample_entry_serialized_len(&self.config_boxes)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_audio_sample_entry(
            buf,
            AudioSampleEntryFields {
                fourcc: b"ac-3",
                entry_version: self.entry_version,
                reserved_1: self.reserved_1,
                data_reference_index: self.data_reference_index,
                channelcount: self.channelcount,
                samplesize: self.samplesize,
                compression_id_and_packet_size: self.compression_id_and_packet_size,
                samplerate: self.samplerate,
                config_boxes: &self.config_boxes,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Ec3SampleEntry (ec-3) — ETSI TS 102 366 §F.5
// ---------------------------------------------------------------------------

/// E-AC-3 audio sample entry (`ec-3`) — ETSI TS 102 366 §F.5.
///
/// Same AudioSampleEntry fixed fields as [`Mp4aSampleEntry`], then a `dec3`
/// config box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Ec3SampleEntry {
    /// `entry_version`: 0 for a plain `AudioSampleEntry` (§12.2.3), 1 for an
    /// [`AudioSampleEntryV1`] (§12.2.3.2) whose real rate lives in a
    /// [`SamplingRateBox`] among `config_boxes`, or any other value a
    /// QuickTime sound description carried.
    ///
    /// Stored and written back **verbatim** — the field used to be re-derived
    /// as 0-or-1 on serialize, which silently rewrote a QuickTime v2 (`02 00`)
    /// entry as v0 on any parse -> serialize round trip.
    pub entry_version: u16,
    /// The six bytes following `entry_version` in the fixed 8-byte block
    /// (QuickTime's `revision_level` + `vendor`, ISO/IEC 14496-12's
    /// `reserved[3]`). Written back verbatim so a real file round-trips.
    pub reserved_1: [u8; 6],
    pub data_reference_index: u16,
    pub channelcount: u16,
    pub samplesize: u16,
    /// The `pre_defined` + `reserved` pair (ISO/IEC 14496-12 §12.2.3) — or, in
    /// a QuickTime sound description, `compression_ID` + `packet_size`.
    /// QuickTime's VBR marker is `compression_ID = -2` (`ff fe`), so this is
    /// not always zero and must survive a round trip.
    pub compression_id_and_packet_size: [u8; 4],
    pub samplerate: u32,
    pub config_boxes: Vec<OpaqueBox>,
}

impl Serialize for Ec3SampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        audio_sample_entry_serialized_len(&self.config_boxes)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_audio_sample_entry(
            buf,
            AudioSampleEntryFields {
                fourcc: b"ec-3",
                entry_version: self.entry_version,
                reserved_1: self.reserved_1,
                data_reference_index: self.data_reference_index,
                channelcount: self.channelcount,
                samplesize: self.samplesize,
                compression_id_and_packet_size: self.compression_id_and_packet_size,
                samplerate: self.samplerate,
                config_boxes: &self.config_boxes,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// OpusSampleEntry (Opus) / FlacSampleEntry (fLaC) / Ac4SampleEntry (ac-4)
// ---------------------------------------------------------------------------

/// Opus audio sample entry (`Opus`) — Opus-in-ISOBMFF §4.3.2.
///
/// Same AudioSampleEntry fixed fields as [`Mp4aSampleEntry`], then a `dOps` box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OpusSampleEntry {
    /// `entry_version`: 0 for a plain `AudioSampleEntry` (§12.2.3), 1 for an
    /// [`AudioSampleEntryV1`] (§12.2.3.2) whose real rate lives in a
    /// [`SamplingRateBox`] among `config_boxes`, or any other value a
    /// QuickTime sound description carried.
    ///
    /// Stored and written back **verbatim** — the field used to be re-derived
    /// as 0-or-1 on serialize, which silently rewrote a QuickTime v2 (`02 00`)
    /// entry as v0 on any parse -> serialize round trip.
    pub entry_version: u16,
    /// The six bytes following `entry_version` in the fixed 8-byte block
    /// (QuickTime's `revision_level` + `vendor`, ISO/IEC 14496-12's
    /// `reserved[3]`). Written back verbatim so a real file round-trips.
    pub reserved_1: [u8; 6],
    pub data_reference_index: u16,
    pub channelcount: u16,
    pub samplesize: u16,
    /// The `pre_defined` + `reserved` pair (ISO/IEC 14496-12 §12.2.3) — or, in
    /// a QuickTime sound description, `compression_ID` + `packet_size`.
    /// QuickTime's VBR marker is `compression_ID = -2` (`ff fe`), so this is
    /// not always zero and must survive a round trip.
    pub compression_id_and_packet_size: [u8; 4],
    pub samplerate: u32,
    pub config_boxes: Vec<OpaqueBox>,
}

impl Serialize for OpusSampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        audio_sample_entry_serialized_len(&self.config_boxes)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_audio_sample_entry(
            buf,
            AudioSampleEntryFields {
                fourcc: b"Opus",
                entry_version: self.entry_version,
                reserved_1: self.reserved_1,
                data_reference_index: self.data_reference_index,
                channelcount: self.channelcount,
                samplesize: self.samplesize,
                compression_id_and_packet_size: self.compression_id_and_packet_size,
                samplerate: self.samplerate,
                config_boxes: &self.config_boxes,
            },
        )
    }
}

impl<'a> Parse<'a> for OpusSampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let e = parse_audio_sample_entry(bytes, "Opus")?;
        Ok(Self {
            entry_version: e.entry_version,
            reserved_1: e.reserved_1,
            data_reference_index: e.data_reference_index,
            channelcount: e.channelcount,
            samplesize: e.samplesize,
            compression_id_and_packet_size: e.compression_id_and_packet_size,
            samplerate: e.samplerate,
            config_boxes: e.config_boxes,
        })
    }
}

/// FLAC audio sample entry (`fLaC`) — FLAC-in-ISOBMFF.
///
/// Same AudioSampleEntry fixed fields as [`Mp4aSampleEntry`], then a `dfLa` box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FlacSampleEntry {
    /// `entry_version`: 0 for a plain `AudioSampleEntry` (§12.2.3), 1 for an
    /// [`AudioSampleEntryV1`] (§12.2.3.2) whose real rate lives in a
    /// [`SamplingRateBox`] among `config_boxes`, or any other value a
    /// QuickTime sound description carried.
    ///
    /// Stored and written back **verbatim** — the field used to be re-derived
    /// as 0-or-1 on serialize, which silently rewrote a QuickTime v2 (`02 00`)
    /// entry as v0 on any parse -> serialize round trip.
    pub entry_version: u16,
    /// The six bytes following `entry_version` in the fixed 8-byte block
    /// (QuickTime's `revision_level` + `vendor`, ISO/IEC 14496-12's
    /// `reserved[3]`). Written back verbatim so a real file round-trips.
    pub reserved_1: [u8; 6],
    pub data_reference_index: u16,
    pub channelcount: u16,
    pub samplesize: u16,
    /// The `pre_defined` + `reserved` pair (ISO/IEC 14496-12 §12.2.3) — or, in
    /// a QuickTime sound description, `compression_ID` + `packet_size`.
    /// QuickTime's VBR marker is `compression_ID = -2` (`ff fe`), so this is
    /// not always zero and must survive a round trip.
    pub compression_id_and_packet_size: [u8; 4],
    pub samplerate: u32,
    pub config_boxes: Vec<OpaqueBox>,
}

impl Serialize for FlacSampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        audio_sample_entry_serialized_len(&self.config_boxes)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_audio_sample_entry(
            buf,
            AudioSampleEntryFields {
                fourcc: b"fLaC",
                entry_version: self.entry_version,
                reserved_1: self.reserved_1,
                data_reference_index: self.data_reference_index,
                channelcount: self.channelcount,
                samplesize: self.samplesize,
                compression_id_and_packet_size: self.compression_id_and_packet_size,
                samplerate: self.samplerate,
                config_boxes: &self.config_boxes,
            },
        )
    }
}

impl<'a> Parse<'a> for FlacSampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let e = parse_audio_sample_entry(bytes, "fLaC")?;
        Ok(Self {
            entry_version: e.entry_version,
            reserved_1: e.reserved_1,
            data_reference_index: e.data_reference_index,
            channelcount: e.channelcount,
            samplesize: e.samplesize,
            compression_id_and_packet_size: e.compression_id_and_packet_size,
            samplerate: e.samplerate,
            config_boxes: e.config_boxes,
        })
    }
}

/// AC-4 audio sample entry (`ac-4`) — ETSI TS 103 190-2 §E.4.
///
/// Same AudioSampleEntry fixed fields as [`Mp4aSampleEntry`], then a `dac4` box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Ac4SampleEntry {
    /// `entry_version`: 0 for a plain `AudioSampleEntry` (§12.2.3), 1 for an
    /// [`AudioSampleEntryV1`] (§12.2.3.2) whose real rate lives in a
    /// [`SamplingRateBox`] among `config_boxes`, or any other value a
    /// QuickTime sound description carried.
    ///
    /// Stored and written back **verbatim** — the field used to be re-derived
    /// as 0-or-1 on serialize, which silently rewrote a QuickTime v2 (`02 00`)
    /// entry as v0 on any parse -> serialize round trip.
    pub entry_version: u16,
    /// The six bytes following `entry_version` in the fixed 8-byte block
    /// (QuickTime's `revision_level` + `vendor`, ISO/IEC 14496-12's
    /// `reserved[3]`). Written back verbatim so a real file round-trips.
    pub reserved_1: [u8; 6],
    pub data_reference_index: u16,
    pub channelcount: u16,
    pub samplesize: u16,
    /// The `pre_defined` + `reserved` pair (ISO/IEC 14496-12 §12.2.3) — or, in
    /// a QuickTime sound description, `compression_ID` + `packet_size`.
    /// QuickTime's VBR marker is `compression_ID = -2` (`ff fe`), so this is
    /// not always zero and must survive a round trip.
    pub compression_id_and_packet_size: [u8; 4],
    pub samplerate: u32,
    pub config_boxes: Vec<OpaqueBox>,
}

impl Serialize for Ac4SampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        audio_sample_entry_serialized_len(&self.config_boxes)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_audio_sample_entry(
            buf,
            AudioSampleEntryFields {
                fourcc: b"ac-4",
                entry_version: self.entry_version,
                reserved_1: self.reserved_1,
                data_reference_index: self.data_reference_index,
                channelcount: self.channelcount,
                samplesize: self.samplesize,
                compression_id_and_packet_size: self.compression_id_and_packet_size,
                samplerate: self.samplerate,
                config_boxes: &self.config_boxes,
            },
        )
    }
}

impl<'a> Parse<'a> for Ac4SampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let e = parse_audio_sample_entry(bytes, "ac-4")?;
        Ok(Self {
            entry_version: e.entry_version,
            reserved_1: e.reserved_1,
            data_reference_index: e.data_reference_index,
            channelcount: e.channelcount,
            samplesize: e.samplesize,
            compression_id_and_packet_size: e.compression_id_and_packet_size,
            samplerate: e.samplerate,
            config_boxes: e.config_boxes,
        })
    }
}

// ---------------------------------------------------------------------------
// DtsSampleEntry (dtsc / dtsh / dtsl / dtse) — ETSI TS 102 114 §E.2
// ---------------------------------------------------------------------------

/// DTS audio sample entry (`dtsc`, `dtsh`, `dtsl`, `dtse`) — ETSI TS 102 114 §E.2.
///
/// Same AudioSampleEntry fixed fields as [`Mp4aSampleEntry`], then a `ddts` box
/// carrying the [`crate::dts::DtsSpecificBox`].  The `codec_type` field records
/// which of the four DTS FourCCs (`dtsc`/`dtsh`/`dtsl`/`dtse`) was parsed or built.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DtsSampleEntry {
    /// The FourCC of this sample entry — one of `dtsc`, `dtsh`, `dtsl`, `dtse`.
    pub codec_type: [u8; 4],
    /// `entry_version`: 0 for a plain `AudioSampleEntry` (§12.2.3), 1 for an
    /// [`AudioSampleEntryV1`] (§12.2.3.2) whose real rate lives in a
    /// [`SamplingRateBox`] among `config_boxes`, or any other value a
    /// QuickTime sound description carried.
    ///
    /// Stored and written back **verbatim** — the field used to be re-derived
    /// as 0-or-1 on serialize, which silently rewrote a QuickTime v2 (`02 00`)
    /// entry as v0 on any parse -> serialize round trip.
    pub entry_version: u16,
    /// The six bytes following `entry_version` in the fixed 8-byte block
    /// (QuickTime's `revision_level` + `vendor`, ISO/IEC 14496-12's
    /// `reserved[3]`). Written back verbatim so a real file round-trips.
    pub reserved_1: [u8; 6],
    pub data_reference_index: u16,
    pub channelcount: u16,
    pub samplesize: u16,
    /// The `pre_defined` + `reserved` pair (ISO/IEC 14496-12 §12.2.3) — or, in
    /// a QuickTime sound description, `compression_ID` + `packet_size`.
    /// QuickTime's VBR marker is `compression_ID = -2` (`ff fe`), so this is
    /// not always zero and must survive a round trip.
    pub compression_id_and_packet_size: [u8; 4],
    pub samplerate: u32,
    /// Config and any extra child boxes (typically one `ddts`).
    pub config_boxes: Vec<OpaqueBox>,
}

impl Serialize for DtsSampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        audio_sample_entry_serialized_len(&self.config_boxes)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_audio_sample_entry(
            buf,
            AudioSampleEntryFields {
                fourcc: &self.codec_type,
                entry_version: self.entry_version,
                reserved_1: self.reserved_1,
                data_reference_index: self.data_reference_index,
                channelcount: self.channelcount,
                samplesize: self.samplesize,
                compression_id_and_packet_size: self.compression_id_and_packet_size,
                samplerate: self.samplerate,
                config_boxes: &self.config_boxes,
            },
        )
    }
}

impl<'a> Parse<'a> for DtsSampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 8 {
            return Err(Error::BufferTooShort {
                need: 8,
                have: bytes.len(),
                what: "DtsSampleEntry",
            });
        }
        let mut codec_type = [0u8; 4];
        codec_type.copy_from_slice(&bytes[4..8]);
        let e = parse_audio_sample_entry(bytes, "DtsSampleEntry")?;
        Ok(Self {
            codec_type,
            entry_version: e.entry_version,
            reserved_1: e.reserved_1,
            data_reference_index: e.data_reference_index,
            channelcount: e.channelcount,
            samplesize: e.samplesize,
            compression_id_and_packet_size: e.compression_id_and_packet_size,
            samplerate: e.samplerate,
            config_boxes: e.config_boxes,
        })
    }
}

// ---------------------------------------------------------------------------
// MhaSampleEntry (mha1 / mha2 / mhm1 / mhm2) — ISO/IEC 23008-3 §20
// ---------------------------------------------------------------------------

/// MPEG-H 3D Audio sample entry (`mha1`, `mha2`, `mhm1`, `mhm2`) — ISO/IEC 23008-3 §20.
///
/// Same AudioSampleEntry fixed fields as [`Mp4aSampleEntry`], then an `mhaC` box
/// (mandatory for `mha1`/`mha2`; optional for `mhm1`/`mhm2`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MhaSampleEntry {
    /// The FourCC of this sample entry — one of `mha1`, `mha2`, `mhm1`, `mhm2`.
    pub codec_type: [u8; 4],
    /// `entry_version`: 0 for a plain `AudioSampleEntry` (§12.2.3), 1 for an
    /// [`AudioSampleEntryV1`] (§12.2.3.2) whose real rate lives in a
    /// [`SamplingRateBox`] among `config_boxes`, or any other value a
    /// QuickTime sound description carried.
    ///
    /// Stored and written back **verbatim** — the field used to be re-derived
    /// as 0-or-1 on serialize, which silently rewrote a QuickTime v2 (`02 00`)
    /// entry as v0 on any parse -> serialize round trip.
    pub entry_version: u16,
    /// The six bytes following `entry_version` in the fixed 8-byte block
    /// (QuickTime's `revision_level` + `vendor`, ISO/IEC 14496-12's
    /// `reserved[3]`). Written back verbatim so a real file round-trips.
    pub reserved_1: [u8; 6],
    pub data_reference_index: u16,
    pub channelcount: u16,
    pub samplesize: u16,
    /// The `pre_defined` + `reserved` pair (ISO/IEC 14496-12 §12.2.3) — or, in
    /// a QuickTime sound description, `compression_ID` + `packet_size`.
    /// QuickTime's VBR marker is `compression_ID = -2` (`ff fe`), so this is
    /// not always zero and must survive a round trip.
    pub compression_id_and_packet_size: [u8; 4],
    pub samplerate: u32,
    /// Config and any extra child boxes (typically one `mhaC`).
    pub config_boxes: Vec<OpaqueBox>,
}

impl Serialize for MhaSampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        audio_sample_entry_serialized_len(&self.config_boxes)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_audio_sample_entry(
            buf,
            AudioSampleEntryFields {
                fourcc: &self.codec_type,
                entry_version: self.entry_version,
                reserved_1: self.reserved_1,
                data_reference_index: self.data_reference_index,
                channelcount: self.channelcount,
                samplesize: self.samplesize,
                compression_id_and_packet_size: self.compression_id_and_packet_size,
                samplerate: self.samplerate,
                config_boxes: &self.config_boxes,
            },
        )
    }
}

impl<'a> Parse<'a> for MhaSampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 8 {
            return Err(Error::BufferTooShort {
                need: 8,
                have: bytes.len(),
                what: "MhaSampleEntry",
            });
        }
        let mut codec_type = [0u8; 4];
        codec_type.copy_from_slice(&bytes[4..8]);
        let e = parse_audio_sample_entry(bytes, "MhaSampleEntry")?;
        Ok(Self {
            codec_type,
            entry_version: e.entry_version,
            reserved_1: e.reserved_1,
            data_reference_index: e.data_reference_index,
            channelcount: e.channelcount,
            samplesize: e.samplesize,
            compression_id_and_packet_size: e.compression_id_and_packet_size,
            samplerate: e.samplerate,
            config_boxes: e.config_boxes,
        })
    }
}

// ---------------------------------------------------------------------------
// Shared audio sample entry parse/serialize helpers
// ---------------------------------------------------------------------------

/// Parse an AudioSampleEntry-derived box (28-byte fixed prefix + config boxes).
///
/// The fixed fields parsed out of an audio sample entry, plus its config
/// children.
///
/// `entry_version` is 0 for a plain `AudioSampleEntry`, 1 for an
/// `AudioSampleEntryV1` (ISO/IEC 14496-12 §12.2.3.2 as amended by Amd 1:2017)
/// or any other value a QuickTime sound description carried; every form shares
/// the 28-byte fixed layout. Every field is preserved verbatim, including the
/// ones this crate does not interpret (QuickTime's `revision_level`/`vendor`
/// and `compression_ID`/`packet_size`), so a real file round-trips
/// byte-exactly.
struct ParsedAudioSampleEntry {
    entry_version: u16,
    reserved_1: [u8; 6],
    data_reference_index: u16,
    channelcount: u16,
    samplesize: u16,
    compression_id_and_packet_size: [u8; 4],
    samplerate: u32,
    config_boxes: Vec<OpaqueBox>,
}

fn parse_audio_sample_entry(bytes: &[u8], what: &'static str) -> Result<ParsedAudioSampleEntry> {
    if bytes.len() < 8 + 28 {
        return Err(Error::BufferTooShort {
            need: 8 + 28,
            have: bytes.len(),
            what,
        });
    }
    let body = &bytes[8..];
    let entry_version = u16::from_be_bytes([body[8], body[9]]);
    let mut reserved_1 = [0u8; 6];
    reserved_1.copy_from_slice(&body[10..16]);
    let dri = u16::from_be_bytes([body[6], body[7]]);
    let chan = u16::from_be_bytes([body[16], body[17]]);
    let samp_sz = u16::from_be_bytes([body[18], body[19]]);
    let mut compression_id_and_packet_size = [0u8; 4];
    compression_id_and_packet_size.copy_from_slice(&body[20..24]);
    let sr = u32::from_be_bytes([body[24], body[25], body[26], body[27]]);

    let mut config_boxes = Vec::new();
    let mut off = 28usize;
    while off + 8 <= body.len() {
        let sz =
            u32::from_be_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]]) as usize;
        if sz < 8 {
            break;
        }
        let end = (off + sz).min(body.len());
        let boxtype = [body[off + 4], body[off + 5], body[off + 6], body[off + 7]];
        let data = body[off + 8..end].to_vec();
        config_boxes.push(OpaqueBox {
            box_type: boxtype,
            data,
            to_end: false,
            largesize: false,
        });
        off += sz;
    }
    Ok(ParsedAudioSampleEntry {
        entry_version,
        reserved_1,
        data_reference_index: dri,
        channelcount: chan,
        samplesize: samp_sz,
        compression_id_and_packet_size,
        samplerate: sr,
        config_boxes,
    })
}

impl<'a> Parse<'a> for Ac3SampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let e = parse_audio_sample_entry(bytes, "ac-3")?;
        Ok(Self {
            entry_version: e.entry_version,
            reserved_1: e.reserved_1,
            data_reference_index: e.data_reference_index,
            channelcount: e.channelcount,
            samplesize: e.samplesize,
            compression_id_and_packet_size: e.compression_id_and_packet_size,
            samplerate: e.samplerate,
            config_boxes: e.config_boxes,
        })
    }
}

impl<'a> Parse<'a> for Ec3SampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let e = parse_audio_sample_entry(bytes, "ec-3")?;
        Ok(Self {
            entry_version: e.entry_version,
            reserved_1: e.reserved_1,
            data_reference_index: e.data_reference_index,
            channelcount: e.channelcount,
            samplesize: e.samplesize,
            compression_id_and_packet_size: e.compression_id_and_packet_size,
            samplerate: e.samplerate,
            config_boxes: e.config_boxes,
        })
    }
}

fn audio_sample_entry_serialized_len(config_boxes: &[OpaqueBox]) -> usize {
    let mut n = BOX_HDR + 28;
    for c in config_boxes {
        n += c.serialized_len();
    }
    n
}

/// The shared fixed fields of an audio sample entry, grouped so the writer
/// stays within the argument-count lint.
struct AudioSampleEntryFields<'a> {
    fourcc: &'a [u8; 4],
    entry_version: u16,
    reserved_1: [u8; 6],
    data_reference_index: u16,
    channelcount: u16,
    samplesize: u16,
    compression_id_and_packet_size: [u8; 4],
    samplerate: u32,
    config_boxes: &'a [OpaqueBox],
}

fn serialize_audio_sample_entry(buf: &mut [u8], f: AudioSampleEntryFields<'_>) -> Result<usize> {
    let config_boxes = f.config_boxes;
    let need = audio_sample_entry_serialized_len(config_boxes);
    if buf.len() < need {
        return Err(Error::OutputBufferTooSmall {
            need,
            have: buf.len(),
        });
    }
    let mut c = 0usize;
    buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
    c += 4;
    buf[c..c + 4].copy_from_slice(f.fourcc);
    c += 4;
    // SampleEntry: reserved(6) zeros + data_reference_index(2). Written, not
    // skipped, so a reused caller buffer cannot keep garbage (r04-W45).
    buf[c..c + 6].fill(0);
    c += 6;
    buf[c..c + 2].copy_from_slice(&f.data_reference_index.to_be_bytes());
    c += 2;
    // The 8-byte block: `entry_version` (0 for a plain AudioSampleEntry
    // §12.2.3, 1 for an AudioSampleEntryV1 §12.2.3.2, or whatever a QuickTime
    // sound description carried) followed by 6 bytes that are
    // `reserved[3]`/`revision_level`+`vendor`. Both are written back verbatim:
    // re-deriving them rewrote a QuickTime entry as v0 on re-serialize.
    buf[c..c + 2].copy_from_slice(&f.entry_version.to_be_bytes());
    buf[c + 2..c + 8].copy_from_slice(&f.reserved_1);
    c += 8;
    buf[c..c + 2].copy_from_slice(&f.channelcount.to_be_bytes());
    c += 2;
    buf[c..c + 2].copy_from_slice(&f.samplesize.to_be_bytes());
    c += 2;
    // `pre_defined`(16) + `reserved`(16) — QuickTime's `compression_ID` +
    // `packet_size`. Written from the parsed value, not zeroed: QuickTime marks
    // VBR audio with `compression_ID = -2`, so zeroing would corrupt it.
    buf[c..c + 4].copy_from_slice(&f.compression_id_and_packet_size);
    c += 4;
    buf[c..c + 4].copy_from_slice(&f.samplerate.to_be_bytes());
    c += 4;
    for cb in config_boxes {
        c += cb.serialize_into(&mut buf[c..])?;
    }
    Ok(c)
}

// ---------------------------------------------------------------------------
// Mp4aSampleEntry (existing — refactored to use helpers)
// ---------------------------------------------------------------------------

impl<'a> Parse<'a> for Mp4aSampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        // bytes is a full box with 8-byte header; fields start at bytes[8]
        if bytes.len() < 8 + 28 {
            return Err(Error::BufferTooShort {
                need: 8 + 28,
                have: bytes.len(),
                what: "mp4a",
            });
        }
        let mut codec_type = [0u8; 4];
        codec_type.copy_from_slice(&bytes[4..8]);
        let e = parse_audio_sample_entry(bytes, "mp4a")?;
        Ok(Self {
            codec_type,
            entry_version: e.entry_version,
            reserved_1: e.reserved_1,
            data_reference_index: e.data_reference_index,
            channelcount: e.channelcount,
            samplesize: e.samplesize,
            compression_id_and_packet_size: e.compression_id_and_packet_size,
            samplerate: e.samplerate,
            config_boxes: e.config_boxes,
        })
    }
}

impl Serialize for Mp4aSampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR + 28; // box header + AudioSampleEntry fixed fields
        for c in &self.config_boxes {
            n += c.serialized_len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        serialize_audio_sample_entry(
            buf,
            AudioSampleEntryFields {
                fourcc: &self.codec_type,
                entry_version: self.entry_version,
                reserved_1: self.reserved_1,
                data_reference_index: self.data_reference_index,
                channelcount: self.channelcount,
                samplesize: self.samplesize,
                compression_id_and_packet_size: self.compression_id_and_packet_size,
                samplerate: self.samplerate,
                config_boxes: &self.config_boxes,
            },
        )
    }
}

/// Describes one sample entry in an stsd box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SampleEntryVariant {
    Avc1(crate::sample_entries::AVCSampleEntry),
    Hevc1(crate::sample_entries::HEVCSampleEntry),
    /// H.266/VVC sample entry (`vvc1`/`vvi1`) — ISO/IEC 14496-15:2022 §11.3.3.
    /// The `codec_type` field on [`crate::sample_entries::VVCSampleEntry`]
    /// records which FourCC was parsed.
    Vvc(Box<crate::sample_entries::VVCSampleEntry>),
    /// MPEG visual sample entry (`mp4v`, esds-bearing) — used for MPEG-2 video
    /// (H.262). ISO/IEC 14496-14 §5.6.
    Mp4v(Box<crate::sample_entries::Mp4vSampleEntry>),
    Av01(Box<crate::av1::Av1SampleEntry>),
    Vp09(Box<crate::vp9::Vp9SampleEntry>),
    Mp4a(Box<Mp4aSampleEntry>),
    Ac3(Box<Ac3SampleEntry>),
    Ec3(Box<Ec3SampleEntry>),
    /// ISO/IEC 14496-30 TTML/IMSC XML subtitle sample entry (`stpp`).
    Stpp(Box<crate::subtitle_entries::XmlSubtitleSampleEntry>),
    /// ISO/IEC 14496-30 WebVTT subtitle sample entry (`wvtt`).
    Wvtt(Box<crate::subtitle_entries::WvttSampleEntry>),
    Ac4(Box<Ac4SampleEntry>),
    Opus(Box<OpusSampleEntry>),
    Flac(Box<FlacSampleEntry>),
    /// MPEG-H 3D Audio sample entry (`mha1`, `mha2`, `mhm1`, or `mhm2`) —
    /// ISO/IEC 23008-3 §20.  The `codec_type` field on [`MhaSampleEntry`]
    /// records which FourCC was parsed.
    Mha(Box<MhaSampleEntry>),
    /// DTS audio sample entry (`dtsc`, `dtsh`, `dtsl`, or `dtse`) —
    /// ETSI TS 102 114 §E.2.  The `codec_type` field on [`DtsSampleEntry`]
    /// records which FourCC was parsed.
    Dts(Box<DtsSampleEntry>),
    Unknown(OpaqueBox),
}

impl SampleEntryVariant {
    /// The `stsd` version this entry requires.
    ///
    /// An `AudioSampleEntryV1` (`entry_version == 1`) is valid only inside an
    /// `stsd` with `version == 1` (ISO/IEC 14496-12 §12.2.3.2 as amended by
    /// Amd 1:2017). Every other entry uses the version-0 form (§8.5.2).
    pub fn required_stsd_version(&self) -> u8 {
        let entry_version = match self {
            SampleEntryVariant::Mp4a(e) => e.entry_version,
            SampleEntryVariant::Ac3(e) => e.entry_version,
            SampleEntryVariant::Ec3(e) => e.entry_version,
            SampleEntryVariant::Opus(e) => e.entry_version,
            SampleEntryVariant::Flac(e) => e.entry_version,
            SampleEntryVariant::Ac4(e) => e.entry_version,
            SampleEntryVariant::Mha(e) => e.entry_version,
            SampleEntryVariant::Dts(e) => e.entry_version,
            _ => 0,
        };
        if entry_version == AudioSampleEntryV1::ENTRY_VERSION {
            AudioSampleEntryV1::STSD_VERSION
        } else {
            0
        }
    }
}

/// Sample Description Box (`stsd`) — §8.5.2.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleDescriptionBox {
    pub version: u8,
    pub flags: u32,
    pub entries: Vec<SampleEntryVariant>,
}

impl<'a> Parse<'a> for SampleDescriptionBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 16 {
            return Err(Error::BufferTooShort {
                need: 16,
                have: bytes.len(),
                what: "stsd",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        let count = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        check_entry_count(
            bytes.len().saturating_sub(16),
            8,
            count,
            "SampleDescriptionBox.entry_count",
        )?;
        let mut entries = Vec::with_capacity(bounded_entry_count(
            bytes.len().saturating_sub(16),
            8,
            count,
        ));
        let mut off = 16usize;
        for _ in 0..count {
            if off + 8 > bytes.len() {
                break;
            }
            let sz =
                u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
                    as usize;
            if sz < 8 {
                break;
            }
            let end = (off + sz).min(bytes.len());
            let box_bytes = &bytes[off..end];
            let codec = &box_bytes[4..8];
            let entry = match codec {
                b"avc1" | b"avc3" | b"avc2" | b"avc4" => SampleEntryVariant::Avc1(
                    crate::sample_entries::AVCSampleEntry::bare_parse(box_bytes)?,
                ),
                b"hvc1" | b"hev1" => SampleEntryVariant::Hevc1(
                    crate::sample_entries::HEVCSampleEntry::bare_parse(box_bytes)?,
                ),
                b"vvc1" | b"vvi1" => SampleEntryVariant::Vvc(Box::new(
                    crate::sample_entries::VVCSampleEntry::bare_parse(box_bytes)?,
                )),
                b"mp4v" => SampleEntryVariant::Mp4v(Box::new(
                    crate::sample_entries::Mp4vSampleEntry::bare_parse(box_bytes)?,
                )),
                b"mp4a" | b"enca" => {
                    SampleEntryVariant::Mp4a(Box::new(Mp4aSampleEntry::parse(box_bytes)?))
                }
                b"ac-3" => SampleEntryVariant::Ac3(Box::new(Ac3SampleEntry::parse(box_bytes)?)),
                b"ec-3" => SampleEntryVariant::Ec3(Box::new(Ec3SampleEntry::parse(box_bytes)?)),
                b"stpp" => SampleEntryVariant::Stpp(Box::new(
                    crate::subtitle_entries::XmlSubtitleSampleEntry::bare_parse(box_bytes)?,
                )),
                b"wvtt" => SampleEntryVariant::Wvtt(Box::new(
                    crate::subtitle_entries::WvttSampleEntry::bare_parse(box_bytes)?,
                )),
                b"av01" => SampleEntryVariant::Av01(Box::new(
                    crate::av1::Av1SampleEntry::parse_entry(box_bytes)?,
                )),
                b"vp09" => SampleEntryVariant::Vp09(Box::new(
                    crate::vp9::Vp9SampleEntry::parse_entry(box_bytes)?,
                )),
                b"ac-4" => SampleEntryVariant::Ac4(Box::new(Ac4SampleEntry::parse(box_bytes)?)),
                b"Opus" => SampleEntryVariant::Opus(Box::new(OpusSampleEntry::parse(box_bytes)?)),
                b"fLaC" => SampleEntryVariant::Flac(Box::new(FlacSampleEntry::parse(box_bytes)?)),
                b"mha1" | b"mha2" | b"mhm1" | b"mhm2" => {
                    SampleEntryVariant::Mha(Box::new(MhaSampleEntry::parse(box_bytes)?))
                }
                b"dtsc" | b"dtsh" | b"dtsl" | b"dtse" => {
                    SampleEntryVariant::Dts(Box::new(DtsSampleEntry::parse(box_bytes)?))
                }
                _ => {
                    let mut c4 = [0u8; 4];
                    c4.copy_from_slice(&codec[..4.min(codec.len())]);
                    SampleEntryVariant::Unknown(OpaqueBox::new(c4, box_bytes[8..].to_vec()))
                }
            };
            entries.push(entry);
            off += sz;
        }
        Ok(Self {
            version: ver,
            flags,
            entries,
        })
    }
}

impl Serialize for SampleDescriptionBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR + FULL_HDR + 4;
        for e in &self.entries {
            n += match e {
                SampleEntryVariant::Avc1(a) => a.serialized_len(),
                SampleEntryVariant::Hevc1(h) => h.serialized_len(),
                SampleEntryVariant::Vvc(v) => v.serialized_len(),
                SampleEntryVariant::Mp4v(m) => m.serialized_len(),
                SampleEntryVariant::Av01(a) => a.serialized_len(),
                SampleEntryVariant::Vp09(v) => v.serialized_len(),
                SampleEntryVariant::Mp4a(m) => m.serialized_len(),
                SampleEntryVariant::Ac3(a) => a.serialized_len(),
                SampleEntryVariant::Ec3(e) => e.serialized_len(),
                SampleEntryVariant::Stpp(s) => s.serialized_len(),
                SampleEntryVariant::Wvtt(w) => w.serialized_len(),
                SampleEntryVariant::Ac4(a) => a.serialized_len(),
                SampleEntryVariant::Opus(o) => o.serialized_len(),
                SampleEntryVariant::Flac(f) => f.serialized_len(),
                SampleEntryVariant::Mha(m) => m.serialized_len(),
                SampleEntryVariant::Dts(d) => d.serialized_len(),
                SampleEntryVariant::Unknown(u) => u.serialized_len(),
            };
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"stsd");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        let entry_count = broadcast_common::len::fit_u32(self.entries.len(), "entry_count")?;
        buf[c..c + 4].copy_from_slice(&entry_count.to_be_bytes());
        c += 4;
        for e in &self.entries {
            c += match e {
                SampleEntryVariant::Avc1(a) => a.serialize_into(&mut buf[c..])?,
                SampleEntryVariant::Hevc1(h) => h.serialize_into(&mut buf[c..])?,
                SampleEntryVariant::Vvc(v) => v.serialize_into(&mut buf[c..])?,
                SampleEntryVariant::Mp4v(m) => m.serialize_into(&mut buf[c..])?,
                SampleEntryVariant::Av01(a) => a.serialize_into(&mut buf[c..])?,
                SampleEntryVariant::Vp09(v) => v.serialize_into(&mut buf[c..])?,
                SampleEntryVariant::Mp4a(m) => m.serialize_into(&mut buf[c..])?,
                SampleEntryVariant::Ac3(a) => a.serialize_into(&mut buf[c..])?,
                SampleEntryVariant::Ec3(e) => e.serialize_into(&mut buf[c..])?,
                SampleEntryVariant::Stpp(s) => s.serialize_into(&mut buf[c..])?,
                SampleEntryVariant::Wvtt(w) => w.serialize_into(&mut buf[c..])?,
                SampleEntryVariant::Ac4(a) => a.serialize_into(&mut buf[c..])?,
                SampleEntryVariant::Opus(o) => o.serialize_into(&mut buf[c..])?,
                SampleEntryVariant::Flac(f) => f.serialize_into(&mut buf[c..])?,
                SampleEntryVariant::Mha(m) => m.serialize_into(&mut buf[c..])?,
                SampleEntryVariant::Dts(d) => d.serialize_into(&mut buf[c..])?,
                SampleEntryVariant::Unknown(u) => u.serialize_into(&mut buf[c..])?,
            };
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// stbl children that we preserve as opaque (stss, sgpd, sbgp)
// ---------------------------------------------------------------------------

/// Opaque stbl child box (stss, sgpd, sbgp, etc.)
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StblOpaque {
    /// The full box bytes including 8-byte header.
    pub data: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Helper: parse a list of child boxes from container body and return typed
// variants via an enum.  Used by the container types below.
// ---------------------------------------------------------------------------

/// A single child within an stbl container.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum StblChild {
    Stsd(SampleDescriptionBox),
    Stts(crate::timing::TimeToSampleBox),
    Ctts(crate::timing::CompositionOffsetBox),
    Stsc(SampleToChunkBox),
    Stsz(SampleSizeBox),
    Stco(ChunkOffsetBox),
    Co64(ChunkLargeOffsetBox),
    Stss(SyncSampleBox),
    Opaque(Vec<u8>),
}

fn parse_stbl_children(body: &[u8]) -> Result<Vec<StblChild>> {
    let mut children = Vec::new();
    walk_children(body, |child| {
        let box_bytes = child.bytes;
        let boxtype = child.four_cc;
        children.push(match &boxtype {
            // A `stsd` that fails to parse (e.g. an `avcC` with a malformed
            // trailer) is kept as raw bytes rather than defaulted to an empty
            // box — an empty `entries: Vec::new()` would later present to
            // `track_spec_from_trak` as "no stsd entry" and discard the real
            // parse error, hiding *why* the whole track is about to be
            // dropped (issue #952). The moov as a whole must still parse
            // (media-plane "lenient but loud": one broken optional field
            // costs that field, not the file), so `track_spec_from_trak`
            // re-parses these raw bytes to recover the real error for
            // `Media::skipped`.
            b"stsd" => match SampleDescriptionBox::parse(box_bytes) {
                Ok(stsd) => StblChild::Stsd(stsd),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            // Same treatment as the `stsd` arm above (issue #952): a box that
            // fails to parse is kept as raw bytes, not defaulted to an empty
            // typed box. `TimeToSampleBox { entries: Vec::new(), .. }` for a
            // malformed `stts` used to claim (falsely) that the track has
            // *zero* sample durations — every real duration silently
            // discarded, with nothing surfacing in `Media::skipped` — rather
            // than the truth, which is "this box didn't parse". Consumers
            // (`progressive_demux::find_stbl_child`) re-parse `Opaque` bytes
            // whose four-CC matches what they're looking for, recovering the
            // real error instead of silently treating a corrupt table as
            // absent or empty (audit finding #3).
            b"stts" => match crate::timing::TimeToSampleBox::parse(box_bytes) {
                Ok(b) => StblChild::Stts(b),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            b"ctts" => match crate::timing::CompositionOffsetBox::parse(box_bytes) {
                Ok(b) => StblChild::Ctts(b),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            b"stsc" => match SampleToChunkBox::parse(box_bytes) {
                Ok(b) => StblChild::Stsc(b),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            b"stsz" => match SampleSizeBox::parse(box_bytes) {
                Ok(b) => StblChild::Stsz(b),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            b"stco" => match ChunkOffsetBox::parse(box_bytes) {
                Ok(b) => StblChild::Stco(b),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            b"co64" => match ChunkLargeOffsetBox::parse(box_bytes) {
                Ok(b) => StblChild::Co64(b),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            b"stss" => match SyncSampleBox::parse(box_bytes) {
                Ok(b) => StblChild::Stss(b),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            _ => StblChild::Opaque(box_bytes.to_vec()),
        });
        Ok(())
    })?;
    Ok(children)
}

fn serialize_stbl_children(children: &[StblChild], buf: &mut [u8], off: &mut usize) -> Result<()> {
    for child in children {
        match child {
            StblChild::Stsd(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Stts(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Ctts(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Stsc(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Stsz(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Stco(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Co64(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Stss(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Opaque(d) => {
                let len = d.len();
                buf[*off..*off + len].copy_from_slice(d);
                *off += len;
            }
        }
    }
    Ok(())
}

fn stbl_children_len(children: &[StblChild]) -> usize {
    let mut n = 0;
    for child in children {
        n += match child {
            StblChild::Stsd(b) => b.serialized_len(),
            StblChild::Stts(b) => b.serialized_len(),
            StblChild::Ctts(b) => b.serialized_len(),
            StblChild::Stsc(b) => b.serialized_len(),
            StblChild::Stsz(b) => b.serialized_len(),
            StblChild::Stco(b) => b.serialized_len(),
            StblChild::Co64(b) => b.serialized_len(),
            StblChild::Stss(b) => b.serialized_len(),
            StblChild::Opaque(d) => d.len(),
        };
    }
    n
}

// ---------------------------------------------------------------------------
// SampleTableBox — stbl (container)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleTableBox {
    pub children: Vec<StblChild>,
}

impl<'a> Parse<'a> for SampleTableBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        // Expect full box bytes (size+type header then body)

        Ok(Self {
            children: parse_stbl_children(box_body(bytes)?)?,
        })
    }
}

impl Serialize for SampleTableBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + stbl_children_len(&self.children)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"stbl");
        c += 4;
        serialize_stbl_children(&self.children, buf, &mut c)?;
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// DataInformationBox — dinf (container: dref)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataInformationBox {
    pub dref: Option<DataReferenceBox>,
    pub opaque: Vec<OpaqueBox>,
    /// The wire order of this container's children (audit r05-W12).
    pub order: ChildOrder,
}

impl DataInformationBox {
    /// Build a `dinf` from its typed parts, children in the conventional
    /// `dref`-then-opaque order.
    pub fn new(dref: Option<DataReferenceBox>, opaque: Vec<OpaqueBox>) -> Self {
        Self {
            dref,
            opaque,
            order: ChildOrder::default(),
        }
    }

    /// The container's typed children, in declaration order — the input
    /// [`ChildOrder::resolve`] maps a wire order onto.
    fn typed_children(&self) -> Vec<&dyn SerializeBox> {
        let mut out: Vec<&dyn SerializeBox> = Vec::with_capacity(1);
        if let Some(ref d) = self.dref {
            out.push(d);
        }
        out
    }

    /// This container's typed children in declaration order, as four-CCs.
    fn typed_four_ccs(&self) -> Vec<&'static [u8; 4]> {
        let mut out = Vec::with_capacity(1);
        if self.dref.is_some() {
            out.push(b"dref" as &'static [u8; 4]);
        }
        out
    }

    /// The children to serialize, in wire order.
    fn child_order(&self) -> Vec<Option<usize>> {
        self.order.resolve(&self.typed_four_ccs())
    }
}

impl<'a> Parse<'a> for DataInformationBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut dref = None;
        let mut opaque = Vec::new();
        let mut order = ChildOrder::default();
        walk_children(box_body(bytes)?, |child| {
            if is_four_cc(&child.four_cc, b"dref") {
                order.push(child.four_cc);
                dref = Some(DataReferenceBox::parse(child.bytes)?);
            } else {
                order.push_opaque(child.four_cc);
                opaque.push(OpaqueBox {
                    box_type: child.four_cc,
                    data: child.payload.to_vec(),
                    to_end: child.to_end,
                    largesize: child.largesize,
                });
            }
            Ok(())
        })?;
        Ok(Self {
            dref,
            opaque,
            order,
        })
    }
}

impl Serialize for DataInformationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR;
        for b in self.typed_children() {
            n += b.box_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let len = broadcast_common::len::fit_u32(need, "dinf size")?;
        buf[..4].copy_from_slice(&len.to_be_bytes());
        buf[4..8].copy_from_slice(b"dinf");
        let typed = typed_child_bytes(&self.typed_children())?;
        let mut c = BOX_HDR;
        serialize_children(&self.child_order(), &typed, &self.opaque, buf, &mut c)?;
        if c != need {
            return Err(Error::InvalidInput(
                "dinf child order does not account for the whole box",
            ));
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// MediaInformationBox — minf (container: vmhd/smhd, dinf, stbl)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MediaInformationBox {
    pub vmhd: Option<VideoMediaHeaderBox>,
    pub smhd: Option<SoundMediaHeaderBox>,
    pub dinf: Option<DataInformationBox>,
    pub stbl: Option<SampleTableBox>,
    pub opaque: Vec<OpaqueBox>,
    /// The wire order of this container's children (audit r05-W12).
    pub order: ChildOrder,
}

impl MediaInformationBox {
    /// Build a `minf` from its typed parts, children in the conventional
    /// sequence (media header, `dinf`, `stbl`, then opaque).
    pub fn new(
        vmhd: Option<VideoMediaHeaderBox>,
        smhd: Option<SoundMediaHeaderBox>,
        dinf: Option<DataInformationBox>,
        stbl: Option<SampleTableBox>,
        opaque: Vec<OpaqueBox>,
    ) -> Self {
        Self {
            vmhd,
            smhd,
            dinf,
            stbl,
            opaque,
            order: ChildOrder::default(),
        }
    }
}

impl MediaInformationBox {
    /// The container's typed children, in declaration order — the input
    /// [`ChildOrder::resolve`] maps a wire order onto.
    fn typed_children(&self) -> Vec<&dyn SerializeBox> {
        let mut out: Vec<&dyn SerializeBox> = Vec::with_capacity(4);
        if let Some(ref b) = self.vmhd {
            out.push(b);
        }
        if let Some(ref b) = self.smhd {
            out.push(b);
        }
        if let Some(ref b) = self.dinf {
            out.push(b);
        }
        if let Some(ref b) = self.stbl {
            out.push(b);
        }
        out
    }

    /// This container's typed children in declaration order, as four-CCs.
    fn typed_four_ccs(&self) -> Vec<&'static [u8; 4]> {
        let mut out = Vec::with_capacity(4);
        if self.vmhd.is_some() {
            out.push(b"vmhd" as &'static [u8; 4]);
        }
        if self.smhd.is_some() {
            out.push(b"smhd" as &'static [u8; 4]);
        }
        if self.dinf.is_some() {
            out.push(b"dinf" as &'static [u8; 4]);
        }
        if self.stbl.is_some() {
            out.push(b"stbl" as &'static [u8; 4]);
        }
        out
    }

    fn child_order(&self) -> Vec<Option<usize>> {
        self.order.resolve(&self.typed_four_ccs())
    }
}

impl<'a> Parse<'a> for MediaInformationBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut vmhd = None;
        let mut smhd = None;
        let mut dinf = None;
        let mut stbl = None;
        let mut opaque = Vec::new();
        let mut order = ChildOrder::default();
        walk_children(box_body(bytes)?, |child| {
            match &child.four_cc {
                b"vmhd" => {
                    order.push(child.four_cc);
                    vmhd = Some(VideoMediaHeaderBox::parse(child.bytes)?);
                }
                b"smhd" => {
                    order.push(child.four_cc);
                    smhd = Some(SoundMediaHeaderBox::parse(child.bytes)?);
                }
                b"dinf" => {
                    order.push(child.four_cc);
                    dinf = Some(DataInformationBox::parse(child.bytes)?);
                }
                b"stbl" => {
                    order.push(child.four_cc);
                    stbl = Some(SampleTableBox::parse(child.bytes)?);
                }
                // A subtitle track's `sthd` and a data track's `nmhd` are not
                // modelled (no fields to carry — §12.6.2/§8.4.5.2 are both
                // headerless FullBoxes), but they are preserved *in place*:
                // §6.2.3 puts a media header first in `minf`, and the old
                // "typed first, opaque last" serializer moved them after
                // `stbl`, where strict readers reject them (audit r05-W12).
                _ => {
                    order.push_opaque(child.four_cc);
                    opaque.push(OpaqueBox {
                        box_type: child.four_cc,
                        data: child.payload.to_vec(),
                        to_end: child.to_end,
                        largesize: child.largesize,
                    });
                }
            }
            Ok(())
        })?;
        Ok(Self {
            vmhd,
            smhd,
            dinf,
            stbl,
            opaque,
            order,
        })
    }
}

impl Serialize for MediaInformationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR;
        for b in self.typed_children() {
            n += b.box_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let len = broadcast_common::len::fit_u32(need, "minf size")?;
        buf[..4].copy_from_slice(&len.to_be_bytes());
        buf[4..8].copy_from_slice(b"minf");
        let typed = typed_child_bytes(&self.typed_children())?;
        let mut c = BOX_HDR;
        serialize_children(&self.child_order(), &typed, &self.opaque, buf, &mut c)?;
        if c != need {
            return Err(Error::InvalidInput(
                "minf child order does not account for the whole box",
            ));
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// MediaBox — mdia (container: mdhd, hdlr, minf)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MediaBox {
    pub mdhd: Option<MediaHeaderBox>,
    pub hdlr: Option<HandlerBox>,
    pub minf: Option<MediaInformationBox>,
    pub opaque: Vec<OpaqueBox>,
    /// The wire order of this container's children (audit r05-W12).
    pub order: ChildOrder,
}

impl MediaBox {
    /// Build an `mdia` from its typed parts, children in the conventional
    /// `mdhd`, `hdlr`, `minf` sequence.
    pub fn new(
        mdhd: Option<MediaHeaderBox>,
        hdlr: Option<HandlerBox>,
        minf: Option<MediaInformationBox>,
        opaque: Vec<OpaqueBox>,
    ) -> Self {
        Self {
            mdhd,
            hdlr,
            minf,
            opaque,
            order: ChildOrder::default(),
        }
    }
}

impl MediaBox {
    /// The container's typed children, in declaration order — the input
    /// [`ChildOrder::resolve`] maps a wire order onto.
    fn typed_children(&self) -> Vec<&dyn SerializeBox> {
        let mut out: Vec<&dyn SerializeBox> = Vec::with_capacity(3);
        if let Some(ref b) = self.mdhd {
            out.push(b);
        }
        if let Some(ref b) = self.hdlr {
            out.push(b);
        }
        if let Some(ref b) = self.minf {
            out.push(b);
        }
        out
    }

    /// This container's typed children in declaration order, as four-CCs.
    fn typed_four_ccs(&self) -> Vec<&'static [u8; 4]> {
        let mut out = Vec::with_capacity(3);
        if self.mdhd.is_some() {
            out.push(b"mdhd" as &'static [u8; 4]);
        }
        if self.hdlr.is_some() {
            out.push(b"hdlr" as &'static [u8; 4]);
        }
        if self.minf.is_some() {
            out.push(b"minf" as &'static [u8; 4]);
        }
        out
    }

    fn child_order(&self) -> Vec<Option<usize>> {
        self.order.resolve(&self.typed_four_ccs())
    }
}

impl<'a> Parse<'a> for MediaBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut mdhd = None;
        let mut hdlr = None;
        let mut minf = None;
        let mut opaque = Vec::new();
        let mut order = ChildOrder::default();
        walk_children(box_body(bytes)?, |child| {
            match &child.four_cc {
                b"mdhd" => {
                    order.push(child.four_cc);
                    mdhd = Some(MediaHeaderBox::parse(child.bytes)?);
                }
                b"hdlr" => {
                    order.push(child.four_cc);
                    hdlr = Some(HandlerBox::parse(child.bytes)?);
                }
                b"minf" => {
                    order.push(child.four_cc);
                    minf = Some(MediaInformationBox::parse(child.bytes)?);
                }
                // `elng` (§8.4.6) is not modelled and stays opaque, in place.
                _ => {
                    order.push_opaque(child.four_cc);
                    opaque.push(OpaqueBox {
                        box_type: child.four_cc,
                        data: child.payload.to_vec(),
                        to_end: child.to_end,
                        largesize: child.largesize,
                    });
                }
            }
            Ok(())
        })?;
        Ok(Self {
            mdhd,
            hdlr,
            minf,
            opaque,
            order,
        })
    }
}

impl Serialize for MediaBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR;
        for b in self.typed_children() {
            n += b.box_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let len = broadcast_common::len::fit_u32(need, "mdia size")?;
        buf[..4].copy_from_slice(&len.to_be_bytes());
        buf[4..8].copy_from_slice(b"mdia");
        let typed = typed_child_bytes(&self.typed_children())?;
        let mut c = BOX_HDR;
        serialize_children(&self.child_order(), &typed, &self.opaque, buf, &mut c)?;
        if c != need {
            return Err(Error::InvalidInput(
                "mdia child order does not account for the whole box",
            ));
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// EditBox — edts (container: elst)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EditBox {
    pub elst: Option<crate::timing::EditListBox>,
    pub opaque: Vec<OpaqueBox>,
    /// The wire order of this container's children (audit r05-W12).
    pub order: ChildOrder,
}

impl EditBox {
    /// Build an `edts` from its typed parts, children in the conventional
    /// `elst`-then-opaque order.
    pub fn new(elst: Option<crate::timing::EditListBox>, opaque: Vec<OpaqueBox>) -> Self {
        Self {
            elst,
            opaque,
            order: ChildOrder::default(),
        }
    }

    /// The container's typed children, in declaration order — the input
    /// [`ChildOrder::resolve`] maps a wire order onto.
    fn typed_children(&self) -> Vec<&dyn SerializeBox> {
        let mut out: Vec<&dyn SerializeBox> = Vec::with_capacity(1);
        if let Some(ref b) = self.elst {
            out.push(b);
        }
        out
    }

    /// This container's typed children in declaration order, as four-CCs.
    fn typed_four_ccs(&self) -> Vec<&'static [u8; 4]> {
        let mut out = Vec::with_capacity(1);
        if self.elst.is_some() {
            out.push(b"elst" as &'static [u8; 4]);
        }
        out
    }

    fn child_order(&self) -> Vec<Option<usize>> {
        self.order.resolve(&self.typed_four_ccs())
    }
}

impl<'a> Parse<'a> for EditBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut elst = None;
        let mut opaque = Vec::new();
        let mut order = ChildOrder::default();
        walk_children(box_body(bytes)?, |child| {
            if is_four_cc(&child.four_cc, b"elst") {
                order.push(child.four_cc);
                elst = Some(crate::timing::EditListBox::parse(child.bytes)?);
            } else {
                order.push_opaque(child.four_cc);
                opaque.push(OpaqueBox {
                    box_type: child.four_cc,
                    data: child.payload.to_vec(),
                    to_end: child.to_end,
                    largesize: child.largesize,
                });
            }
            Ok(())
        })?;
        Ok(Self {
            elst,
            opaque,
            order,
        })
    }
}

impl Serialize for EditBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR;
        for b in self.typed_children() {
            n += b.box_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let len = broadcast_common::len::fit_u32(need, "edts size")?;
        buf[..4].copy_from_slice(&len.to_be_bytes());
        buf[4..8].copy_from_slice(b"edts");
        let typed = typed_child_bytes(&self.typed_children())?;
        let mut c = BOX_HDR;
        serialize_children(&self.child_order(), &typed, &self.opaque, buf, &mut c)?;
        if c != need {
            return Err(Error::InvalidInput(
                "edts child order does not account for the whole box",
            ));
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// TrackBox — trak (container: tkhd, edts?, mdia, …)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackBox {
    pub tkhd: TrackHeaderBox,
    pub edts: Option<EditBox>,
    pub mdia: Option<MediaBox>,
    pub opaque: Vec<OpaqueBox>,
    /// The wire order of this container's children (audit r05-W12).
    pub order: ChildOrder,
}

impl TrackBox {
    /// Build a `trak` from its typed parts, children in the conventional
    /// `tkhd`, `edts`, `mdia` sequence.
    pub fn new(
        tkhd: TrackHeaderBox,
        edts: Option<EditBox>,
        mdia: Option<MediaBox>,
        opaque: Vec<OpaqueBox>,
    ) -> Self {
        Self {
            tkhd,
            edts,
            mdia,
            opaque,
            order: ChildOrder::default(),
        }
    }

    /// The container's typed children, in declaration order — the input
    /// [`ChildOrder::resolve`] maps a wire order onto.
    fn typed_children(&self) -> Vec<&dyn SerializeBox> {
        let mut out: Vec<&dyn SerializeBox> = Vec::with_capacity(3);
        out.push(&self.tkhd);
        if let Some(ref b) = self.edts {
            out.push(b);
        }
        if let Some(ref b) = self.mdia {
            out.push(b);
        }
        out
    }

    /// This container's typed children in declaration order, as four-CCs.
    fn typed_four_ccs(&self) -> Vec<&'static [u8; 4]> {
        let mut out = Vec::with_capacity(3);
        out.push(b"tkhd" as &'static [u8; 4]);
        if self.edts.is_some() {
            out.push(b"edts");
        }
        if self.mdia.is_some() {
            out.push(b"mdia");
        }
        out
    }

    fn child_order(&self) -> Vec<Option<usize>> {
        self.order.resolve(&self.typed_four_ccs())
    }
}

impl<'a> Parse<'a> for TrackBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut tkhd = None;
        let mut edts = None;
        let mut mdia = None;
        let mut opaque = Vec::new();
        let mut order = ChildOrder::default();
        walk_children(box_body(bytes)?, |child| {
            match &child.four_cc {
                b"tkhd" => {
                    order.push(child.four_cc);
                    tkhd = Some(TrackHeaderBox::parse(child.bytes)?);
                }
                b"edts" => {
                    order.push(child.four_cc);
                    edts = Some(EditBox::parse(child.bytes)?);
                }
                b"mdia" => {
                    order.push(child.four_cc);
                    mdia = Some(MediaBox::parse(child.bytes)?);
                }
                // `tref`/`trgr`/`udta`/`meta` stay opaque, in place.
                _ => {
                    order.push_opaque(child.four_cc);
                    opaque.push(OpaqueBox {
                        box_type: child.four_cc,
                        data: child.payload.to_vec(),
                        to_end: child.to_end,
                        largesize: child.largesize,
                    });
                }
            }
            Ok(())
        })?;
        Ok(Self {
            tkhd: tkhd.ok_or(Error::BufferTooShort {
                need: 0,
                have: 0,
                what: "trak missing tkhd",
            })?,
            edts,
            mdia,
            opaque,
            order,
        })
    }
}

impl Serialize for TrackBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR;
        for b in self.typed_children() {
            n += b.box_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let len = broadcast_common::len::fit_u32(need, "trak size")?;
        buf[..4].copy_from_slice(&len.to_be_bytes());
        buf[4..8].copy_from_slice(b"trak");
        let typed = typed_child_bytes(&self.typed_children())?;
        let mut c = BOX_HDR;
        serialize_children(&self.child_order(), &typed, &self.opaque, buf, &mut c)?;
        if c != need {
            return Err(Error::InvalidInput(
                "trak child order does not account for the whole box",
            ));
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// MovieBox — moov (container: mvhd, trak*, …) — THE TOP-LEVEL TYPE
// ---------------------------------------------------------------------------

/// Track Extends Box (`trex`) — ISO/IEC 14496-12:2015 §8.8.3.
///
/// Declares per-track defaults for the samples carried in movie fragments. A
/// fragmented-init `moov` carries one `trex` per track inside [`MovieExtendsBox`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackExtendsBox {
    /// FullBox version (0).
    pub version: u8,
    /// FullBox flags (0).
    pub flags: u32,
    /// The track these defaults apply to.
    pub track_id: u32,
    /// Default `stsd` entry index (1-based).
    pub default_sample_description_index: u32,
    /// Default sample duration (movie timescale units).
    pub default_sample_duration: u32,
    /// Default sample size in bytes.
    pub default_sample_size: u32,
    /// Default per-sample flags (§8.8.3 sample flags layout).
    pub default_sample_flags: u32,
}

impl<'a> Parse<'a> for TrackExtendsBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 32 {
            return Err(Error::BufferTooShort {
                need: 32,
                have: bytes.len(),
                what: "trex",
            });
        }
        let body = &bytes[8..];
        let version = body[0];
        let flags = u32::from_be_bytes([0, body[1], body[2], body[3]]);
        Ok(Self {
            version,
            flags,
            track_id: u32::from_be_bytes([body[4], body[5], body[6], body[7]]),
            default_sample_description_index: u32::from_be_bytes([
                body[8], body[9], body[10], body[11],
            ]),
            default_sample_duration: u32::from_be_bytes([body[12], body[13], body[14], body[15]]),
            default_sample_size: u32::from_be_bytes([body[16], body[17], body[18], body[19]]),
            default_sample_flags: u32::from_be_bytes([body[20], body[21], body[22], body[23]]),
        })
    }
}

impl Serialize for TrackExtendsBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        32
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < 32 {
            return Err(Error::OutputBufferTooSmall {
                need: 32,
                have: buf.len(),
            });
        }
        buf[0..4].copy_from_slice(&32u32.to_be_bytes());
        buf[4..8].copy_from_slice(b"trex");
        buf[8] = self.version;
        let fb = self.flags.to_be_bytes();
        buf[9..12].copy_from_slice(&fb[1..]);
        buf[12..16].copy_from_slice(&self.track_id.to_be_bytes());
        buf[16..20].copy_from_slice(&self.default_sample_description_index.to_be_bytes());
        buf[20..24].copy_from_slice(&self.default_sample_duration.to_be_bytes());
        buf[24..28].copy_from_slice(&self.default_sample_size.to_be_bytes());
        buf[28..32].copy_from_slice(&self.default_sample_flags.to_be_bytes());
        Ok(32)
    }
}

/// Movie Extends Box (`mvex`) — ISO/IEC 14496-12:2015 §8.8.1.
///
/// Signals that the movie is fragmented and carries the per-track [`TrackExtendsBox`]
/// defaults. Any other children (e.g. `mehd`, which §6.2.3 orders *before* the
/// `trex` list) are preserved verbatim in `opaque`, at the position they
/// occupied on the wire — see [`MovieExtendsBox::order`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MovieExtendsBox {
    /// One `trex` per track.
    pub trex: Vec<TrackExtendsBox>,
    /// Other `mvex` children preserved verbatim (e.g. `mehd`).
    pub opaque: Vec<OpaqueBox>,
    /// The wire order of this container's children (audit r05-W12).
    pub order: ChildOrder,
}

impl MovieExtendsBox {
    /// Build an `mvex` from its typed parts, children in the conventional
    /// `trex`…-then-opaque order.
    pub fn new(trex: Vec<TrackExtendsBox>, opaque: Vec<OpaqueBox>) -> Self {
        Self {
            trex,
            opaque,
            order: ChildOrder::default(),
        }
    }

    /// The container's typed children, in declaration order — the input
    /// [`ChildOrder::resolve`] maps a wire order onto.
    fn typed_children(&self) -> Vec<&dyn SerializeBox> {
        self.trex.iter().map(|t| t as &dyn SerializeBox).collect()
    }

    /// This container's typed children in declaration order, as four-CCs.
    fn typed_four_ccs(&self) -> Vec<&'static [u8; 4]> {
        alloc::vec![b"trex" as &'static [u8; 4]; self.trex.len()]
    }

    fn child_order(&self) -> Vec<Option<usize>> {
        self.order.resolve(&self.typed_four_ccs())
    }
}

impl<'a> Parse<'a> for MovieExtendsBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut trex = Vec::new();
        let mut opaque = Vec::new();
        let mut order = ChildOrder::default();
        walk_children(box_body(bytes)?, |child| {
            match &child.four_cc {
                b"trex" => {
                    order.push(child.four_cc);
                    trex.push(TrackExtendsBox::parse(child.bytes)?);
                }
                // `mehd`/`leva`/`trep` stay opaque, in place.
                _ => {
                    order.push_opaque(child.four_cc);
                    opaque.push(OpaqueBox {
                        box_type: child.four_cc,
                        data: child.payload.to_vec(),
                        to_end: child.to_end,
                        largesize: child.largesize,
                    });
                }
            }
            Ok(())
        })?;
        Ok(Self {
            trex,
            opaque,
            order,
        })
    }
}

impl Serialize for MovieExtendsBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR;
        for b in self.typed_children() {
            n += b.box_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let len = broadcast_common::len::fit_u32(need, "mvex size")?;
        buf[..4].copy_from_slice(&len.to_be_bytes());
        buf[4..8].copy_from_slice(b"mvex");
        let typed = typed_child_bytes(&self.typed_children())?;
        let mut c = BOX_HDR;
        serialize_children(&self.child_order(), &typed, &self.opaque, buf, &mut c)?;
        if c != need {
            return Err(Error::InvalidInput(
                "mvex child order does not account for the whole box",
            ));
        }
        Ok(c)
    }
}

/// Movie Box (`moov`) — §8.2.1.  The top-level init-segment container.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MovieBox {
    pub mvhd: MovieHeaderBox,
    pub tracks: Vec<TrackBox>,
    /// Movie-extends box (`mvex`) present in fragmented-init movies.
    pub mvex: Option<MovieExtendsBox>,
    pub opaque: Vec<OpaqueBox>,
    /// The wire order of this container's children (audit r05-W12).
    pub order: ChildOrder,
}

impl MovieBox {
    /// Build a `moov` from its typed parts, children in the conventional
    /// `mvhd`, `trak`…, `mvex` sequence.
    pub fn new(
        mvhd: MovieHeaderBox,
        tracks: Vec<TrackBox>,
        mvex: Option<MovieExtendsBox>,
        opaque: Vec<OpaqueBox>,
    ) -> Self {
        Self {
            mvhd,
            tracks,
            mvex,
            opaque,
            order: ChildOrder::default(),
        }
    }

    /// The container's typed children, in declaration order — the input
    /// [`ChildOrder::resolve`] maps a wire order onto.
    fn typed_children(&self) -> Vec<&dyn SerializeBox> {
        let mut out: Vec<&dyn SerializeBox> = Vec::with_capacity(2 + self.tracks.len());
        out.push(&self.mvhd);
        for t in &self.tracks {
            out.push(t);
        }
        if let Some(ref m) = self.mvex {
            out.push(m);
        }
        out
    }

    /// This container's typed children in declaration order, as four-CCs.
    fn typed_four_ccs(&self) -> Vec<&'static [u8; 4]> {
        let mut out: Vec<&'static [u8; 4]> = alloc::vec![b"mvhd"; 1];
        out.resize(1 + self.tracks.len(), b"trak");
        if self.mvex.is_some() {
            out.push(b"mvex");
        }
        out
    }

    /// The children to serialize, in wire order — the `pssh` a movie may carry
    /// keeps its own position instead of moving after the `trak`s (audit
    /// r05-W12).
    pub fn child_order(&self) -> Vec<Option<usize>> {
        self.order.resolve(&self.typed_four_ccs())
    }
}

impl<'a> Parse<'a> for MovieBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut mvhd = None;
        let mut tracks = Vec::new();
        let mut mvex = None;
        let mut opaque = Vec::new();
        let mut order = ChildOrder::default();
        walk_children(box_body(bytes)?, |child| {
            match &child.four_cc {
                b"mvhd" => {
                    order.push(child.four_cc);
                    mvhd = Some(MovieHeaderBox::parse(child.bytes)?);
                }
                b"trak" => {
                    order.push(child.four_cc);
                    tracks.push(TrackBox::parse(child.bytes)?);
                }
                b"mvex" => {
                    order.push(child.four_cc);
                    mvex = Some(MovieExtendsBox::parse(child.bytes)?);
                }
                // `pssh`/`udta`/`meta`/`iods` stay opaque, in place: §8.16.1
                // places `pssh` ahead of the `trak`s in the conventional
                // order, and the old serializer always emitted it last.
                _ => {
                    order.push_opaque(child.four_cc);
                    opaque.push(OpaqueBox {
                        box_type: child.four_cc,
                        data: child.payload.to_vec(),
                        to_end: child.to_end,
                        largesize: child.largesize,
                    });
                }
            }
            Ok(())
        })?;
        Ok(Self {
            mvhd: mvhd.ok_or(Error::BufferTooShort {
                need: 0,
                have: 0,
                what: "moov missing mvhd",
            })?,
            tracks,
            mvex,
            opaque,
            order,
        })
    }
}

impl Serialize for MovieBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR;
        for b in self.typed_children() {
            n += b.box_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let len = broadcast_common::len::fit_u32(need, "moov size")?;
        buf[..4].copy_from_slice(&len.to_be_bytes());
        buf[4..8].copy_from_slice(b"moov");
        let typed = typed_child_bytes(&self.typed_children())?;
        let mut c = BOX_HDR;
        serialize_children(&self.child_order(), &typed, &self.opaque, buf, &mut c)?;
        if c != need {
            return Err(Error::InvalidInput(
                "moov child order does not account for the whole box",
            ));
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// CENC init-segment protection (`encv`/`enca` + `sinf`) — ISO/IEC
// 14496-12:2015 §8.12 (sinf/frma/schm/schi), ISO/IEC 23001-7 §12.2 (tenc) —
// issue #564 Task 3 (muxer emission).
// ---------------------------------------------------------------------------

/// `schm.scheme_version` written for every CENC-protected sample entry:
/// version 1.0, packed as a 16.16 major.minor pair per ISO/IEC
/// 14496-12:2015 §8.12.5.
const CENC_SCHEME_VERSION: u32 = 0x0001_0000;

/// Which media kind a [`SampleEntryVariant`] carries, to choose the
/// CENC-protected wrapper four-CC (`encv` for video, `enca` for audio —
/// ISO/IEC 14496-12:2015 §8.12.1). Subtitle (`stpp`/`wvtt`) and unrecognised
/// entries have no protected-wrapper form defined in this crate.
enum SampleEntryMediaKind {
    Video,
    Audio,
    Unsupported,
}

fn sample_entry_media_kind(entry: &SampleEntryVariant) -> SampleEntryMediaKind {
    match entry {
        SampleEntryVariant::Avc1(_)
        | SampleEntryVariant::Hevc1(_)
        | SampleEntryVariant::Vvc(_)
        | SampleEntryVariant::Mp4v(_)
        | SampleEntryVariant::Av01(_)
        | SampleEntryVariant::Vp09(_) => SampleEntryMediaKind::Video,
        SampleEntryVariant::Mp4a(_)
        | SampleEntryVariant::Ac3(_)
        | SampleEntryVariant::Ec3(_)
        | SampleEntryVariant::Opus(_)
        | SampleEntryVariant::Flac(_)
        | SampleEntryVariant::Mha(_)
        | SampleEntryVariant::Dts(_)
        | SampleEntryVariant::Ac4(_) => SampleEntryMediaKind::Audio,
        SampleEntryVariant::Stpp(_)
        | SampleEntryVariant::Wvtt(_)
        | SampleEntryVariant::Unknown(_) => SampleEntryMediaKind::Unsupported,
    }
}

/// Serialize a [`SampleEntryVariant`] to its own box bytes (header + body).
/// Mirrors the dispatch [`SampleDescriptionBox::serialize_into`] uses,
/// exposed standalone so [`protect_sample_entry`] can recover the original
/// entry's bytes to wrap (rather than hand-rolling a duplicate encoder).
fn sample_entry_bytes(entry: &SampleEntryVariant) -> Result<Vec<u8>> {
    let len = match entry {
        SampleEntryVariant::Avc1(a) => a.serialized_len(),
        SampleEntryVariant::Hevc1(h) => h.serialized_len(),
        SampleEntryVariant::Vvc(v) => v.serialized_len(),
        SampleEntryVariant::Mp4v(m) => m.serialized_len(),
        SampleEntryVariant::Av01(a) => a.serialized_len(),
        SampleEntryVariant::Vp09(v) => v.serialized_len(),
        SampleEntryVariant::Mp4a(m) => m.serialized_len(),
        SampleEntryVariant::Ac3(a) => a.serialized_len(),
        SampleEntryVariant::Ec3(e) => e.serialized_len(),
        SampleEntryVariant::Stpp(s) => s.serialized_len(),
        SampleEntryVariant::Wvtt(w) => w.serialized_len(),
        SampleEntryVariant::Ac4(a) => a.serialized_len(),
        SampleEntryVariant::Opus(o) => o.serialized_len(),
        SampleEntryVariant::Flac(f) => f.serialized_len(),
        SampleEntryVariant::Mha(m) => m.serialized_len(),
        SampleEntryVariant::Dts(d) => d.serialized_len(),
        SampleEntryVariant::Unknown(u) => u.serialized_len(),
    };
    let mut buf = alloc::vec![0u8; len];
    let n = match entry {
        SampleEntryVariant::Avc1(a) => a.serialize_into(&mut buf)?,
        SampleEntryVariant::Hevc1(h) => h.serialize_into(&mut buf)?,
        SampleEntryVariant::Vvc(v) => v.serialize_into(&mut buf)?,
        SampleEntryVariant::Mp4v(m) => m.serialize_into(&mut buf)?,
        SampleEntryVariant::Av01(a) => a.serialize_into(&mut buf)?,
        SampleEntryVariant::Vp09(v) => v.serialize_into(&mut buf)?,
        SampleEntryVariant::Mp4a(m) => m.serialize_into(&mut buf)?,
        SampleEntryVariant::Ac3(a) => a.serialize_into(&mut buf)?,
        SampleEntryVariant::Ec3(e) => e.serialize_into(&mut buf)?,
        SampleEntryVariant::Stpp(s) => s.serialize_into(&mut buf)?,
        SampleEntryVariant::Wvtt(w) => w.serialize_into(&mut buf)?,
        SampleEntryVariant::Ac4(a) => a.serialize_into(&mut buf)?,
        SampleEntryVariant::Opus(o) => o.serialize_into(&mut buf)?,
        SampleEntryVariant::Flac(f) => f.serialize_into(&mut buf)?,
        SampleEntryVariant::Mha(m) => m.serialize_into(&mut buf)?,
        SampleEntryVariant::Dts(d) => d.serialize_into(&mut buf)?,
        SampleEntryVariant::Unknown(u) => u.serialize_into(&mut buf)?,
    };
    buf.truncate(n);
    Ok(buf)
}

/// Wrap a clear sample entry as a CENC-protected `encv`/`enca` entry —
/// ISO/IEC 14496-12:2015 §8.12.1: rename the codec four-CC to `encv`/`enca`,
/// keep the original four-CC in a child `sinf`>`frma`, and add `sinf`>`schm`
/// (`scheme_type`/`scheme_version`) + `sinf`>`schi`>`tenc` (ISO/IEC
/// 23001-7 §12.2).
///
/// Returned as [`SampleEntryVariant::Unknown`] — this crate's generic
/// passthrough case — rather than as a new enum variant: the decrypt side
/// (`crate::cenc_decrypt`) locates `sinf` by walking the raw sample-entry
/// bytes directly, not through this enum, so a generic body box is
/// sufficient, and it keeps every existing exhaustive match over
/// [`SampleEntryVariant`] (e.g. `crate::media::codec_config_from_entry`)
/// unchanged (issue #564).
pub fn protect_sample_entry(
    original: &SampleEntryVariant,
    scheme: crate::cenc::CencScheme,
    tenc: &crate::cenc::TrackEncryptionBox,
) -> Result<SampleEntryVariant> {
    let wrapper_fourcc: [u8; 4] = match sample_entry_media_kind(original) {
        SampleEntryMediaKind::Video => *b"encv",
        SampleEntryMediaKind::Audio => *b"enca",
        SampleEntryMediaKind::Unsupported => {
            return Err(Error::InvalidInput(
                "protect_sample_entry: CENC protection is only defined for audio/video sample entries",
            ));
        }
    };

    let original_bytes = sample_entry_bytes(original)?;
    if original_bytes.len() < BOX_HDR {
        return Err(Error::BufferTooShort {
            need: BOX_HDR,
            have: original_bytes.len(),
            what: "sample entry",
        });
    }
    let mut original_format = [0u8; 4];
    original_format.copy_from_slice(&original_bytes[4..8]);

    let scheme_name = scheme.name().as_bytes();
    let mut scheme_type = [0u8; 4];
    scheme_type.copy_from_slice(&scheme_name[..4]);

    let sinf = crate::cenc::ProtectionSchemeInfoBox {
        original_format: crate::cenc::OriginalFormatBox {
            data_format: original_format,
        },
        scheme_type: Some(crate::cenc::SchemeTypeBox {
            version: 0,
            flags: 0,
            scheme_type,
            scheme_version: CENC_SCHEME_VERSION,
            scheme_uri: None,
        }),
        scheme_info: Some(crate::cenc::SchemeInformationBox {
            tenc: Some(tenc.clone()),
            extra_boxes: Vec::new(),
        }),
        extra_boxes: Vec::new(),
    };
    let mut sinf_bytes = alloc::vec![0u8; sinf.serialized_len()];
    let n = sinf.serialize_into(&mut sinf_bytes)?;
    sinf_bytes.truncate(n);

    let mut body = Vec::with_capacity(original_bytes.len() - BOX_HDR + sinf_bytes.len());
    body.extend_from_slice(&original_bytes[BOX_HDR..]);
    body.extend_from_slice(&sinf_bytes);

    Ok(SampleEntryVariant::Unknown(OpaqueBox::new(
        wrapper_fourcc,
        body,
    )))
}

/// Rewrite an already-built CMAF/fMP4 init segment (`ftyp` + `moov`) so the
/// given track's sample entry becomes CENC-protected (issue #564 Task 3):
/// [`protect_sample_entry`] renames the entry to `encv`/`enca` and adds
/// `sinf`; every ancestor box (`stsd`/`stbl`/`minf`/`mdia`/`trak`/`moov`) is
/// **recomputed** from its typed children by [`MovieBox`]'s own `Serialize`
/// impl (no manual size patching) — only the target track's sample entry and
/// its ancestors' size fields differ from the input; every other byte, and
/// every other track, round-trips unchanged.
///
/// Operates as a *post-processing* pass over an already-muxed init segment
/// rather than being wired into `pipeline::build_init_segment` itself, so it
/// composes with any caller that already has a
/// [`crate::media::TrackEncryption`] in hand (e.g. from
/// `CencEncryptor::encrypt`'s `Track::encryption`) without that crypto
/// metadata needing to flow through the lower-level `TrackSpec`/pipeline
/// plumbing. `init_segment` may be the bare `ftyp`+`moov` pair or a larger
/// buffer with more boxes following (e.g. a whole `CmafMux` output including
/// `styp`/`moof`/`mdat`) — only the `moov` span is touched; everything
/// before and after it is copied through verbatim.
pub fn protect_init_segment(
    init_segment: &[u8],
    track_id: u32,
    encryption: &TrackEncryption,
) -> Result<Vec<u8>> {
    let mut prefix_len = 0usize;
    let mut moov_len = None;
    for step in box_iter(init_segment) {
        let (box_ref, consumed) = step?;
        if box_ref.header.box_type.is(b"moov") {
            moov_len = Some(consumed);
            break;
        }
        prefix_len += consumed;
    }
    let moov_len = moov_len.ok_or(Error::UnexpectedBox { expected: "moov" })?;
    let moov_bytes = &init_segment[prefix_len..prefix_len + moov_len];
    let suffix = &init_segment[prefix_len + moov_len..];

    let mut moov = MovieBox::parse(moov_bytes)?;
    {
        let track = moov
            .tracks
            .iter_mut()
            .find(|t| t.tkhd.track_id == track_id)
            .ok_or(Error::InvalidInput(
                "protect_init_segment: track_id not found in moov",
            ))?;
        let stbl = track
            .mdia
            .as_mut()
            .and_then(|m| m.minf.as_mut())
            .and_then(|m| m.stbl.as_mut())
            .ok_or(Error::UnexpectedBox {
                expected: "trak/mdia/minf/stbl",
            })?;
        let stsd = stbl
            .children
            .iter_mut()
            .find_map(|c| match c {
                StblChild::Stsd(s) => Some(s),
                _ => None,
            })
            .ok_or(Error::UnexpectedBox { expected: "stsd" })?;
        let original = stsd
            .entries
            .first()
            .ok_or(Error::UnexpectedBox {
                expected: "stsd sample entry",
            })?
            .clone();
        stsd.entries[0] = protect_sample_entry(&original, encryption.scheme, &encryption.tenc)?;
    }

    let mut new_moov = alloc::vec![0u8; moov.serialized_len()];
    let n = moov.serialize_into(&mut new_moov)?;
    new_moov.truncate(n);

    let mut out = Vec::with_capacity(prefix_len + new_moov.len() + suffix.len());
    out.extend_from_slice(&init_segment[..prefix_len]);
    out.extend_from_slice(&new_moov);
    out.extend_from_slice(suffix);
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Serialize;

    /// Build a small mvhd v0 for unit testing.
    fn sample_mvhd_v0() -> MovieHeaderBox {
        MovieHeaderBox {
            version: 0,
            flags: 0,
            creation_time: 0,
            modification_time: 0,
            timescale: 1000,
            duration: 2000,
            rate: 0x00010000,
            volume: 0x0100,
            matrix: [0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000],
            next_track_id: 3,
        }
    }

    #[test]
    fn mvhd_v0_round_trip() {
        let m = sample_mvhd_v0();
        let bytes = m.to_bytes();
        let parsed = MovieHeaderBox::parse(&bytes).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn mvhd_v0_mutation_changes_bytes() {
        let m = sample_mvhd_v0();
        let orig = m.to_bytes();
        let mut m2 = m.clone();
        m2.timescale = 30000;
        let mutated = m2.to_bytes();
        assert_ne!(orig, mutated);
        // Verify the right field changed
        assert_ne!(orig[20..24], mutated[20..24]);
    }

    #[test]
    fn tkhd_v0_round_trip() {
        let t = TrackHeaderBox {
            version: 0,
            flags: 0x000003, // track_enabled | track_in_movie
            creation_time: 0,
            modification_time: 0,
            track_id: 1,
            duration: 2000,
            layer: 0,
            alternate_group: 0,
            volume: 0,
            matrix: [0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000],
            width: 0,
            height: 0,
        };
        let bytes = t.to_bytes();
        let parsed = TrackHeaderBox::parse(&bytes).unwrap();
        assert_eq!(parsed, t);
    }

    #[test]
    fn stsc_round_trip() {
        let s = SampleToChunkBox {
            version: 0,
            flags: 0,
            entries: alloc::vec![StscEntry {
                first_chunk: 1,
                samples_per_chunk: 10,
                sample_description_index: 1
            },],
        };
        let bytes = s.to_bytes();
        let parsed = SampleToChunkBox::parse(&bytes).unwrap();
        assert_eq!(parsed, s);
    }

    #[test]
    fn stsz_uniform_round_trip() {
        // Uniform sample_size (constant-size samples): the wire still
        // carries a real sample_count even though `entries` stays empty
        // (issue #1018) — a real MP4Box/ffmpeg-produced fixture asserting
        // this against an independent oracle lives in
        // `tests/cenc_saiz_stsz_v1_boxes.rs`.
        let s = SampleSizeBox {
            version: 0,
            flags: 0,
            sample_size: 512,
            sample_count: 88,
            entries: alloc::vec![],
        };
        let bytes = s.to_bytes();
        let parsed = SampleSizeBox::parse(&bytes).unwrap();
        assert_eq!(parsed, s);
        assert_eq!(
            parsed.sample_count, 88,
            "uniform stsz must keep its real sample_count"
        );
    }

    #[test]
    fn stco_round_trip() {
        let s = ChunkOffsetBox {
            version: 0,
            flags: 0,
            entries: alloc::vec![0, 1024, 2048, 4096],
        };
        let bytes = s.to_bytes();
        let parsed = ChunkOffsetBox::parse(&bytes).unwrap();
        assert_eq!(parsed, s);
    }

    #[test]
    fn dref_url_round_trip() {
        let url = DataEntryUrlBox {
            version: 0,
            flags: 1,
            location: alloc::vec![],
        };
        let dref = DataReferenceBox {
            version: 0,
            flags: 0,
            entries: alloc::vec![url],
        };
        let bytes = dref.to_bytes();
        let parsed = DataReferenceBox::parse(&bytes).unwrap();
        assert_eq!(parsed, dref);
    }

    /// Audit finding #3: a `stbl` child that fails to parse must survive as
    /// [`StblChild::Opaque`] (its real bytes, so the real error is
    /// recoverable at the point of use — see
    /// `progressive_demux::find_stbl_child`), never as a defaulted-empty
    /// typed box that falsely claims the table has zero entries.
    ///
    /// One self-contained `stbl` body: a bare 8-byte `stsc` header (well
    /// below `SampleToChunkBox::parse`'s own 16-byte minimum, so it is
    /// guaranteed to fail to parse) followed by nothing else — no sibling
    /// boxes to keep aligned, so this pins the `parse_stbl_children` behaviour
    /// in isolation from any other box's layout.
    #[test]
    fn parse_stbl_children_keeps_malformed_box_as_opaque_not_defaulted_empty() {
        let mut body = alloc::vec![0u8; 8];
        body[0..4].copy_from_slice(&8u32.to_be_bytes());
        body[4..8].copy_from_slice(b"stsc");

        let children = parse_stbl_children(&body).unwrap();
        assert_eq!(children.len(), 1);
        match &children[0] {
            StblChild::Opaque(raw) => assert_eq!(raw.as_slice(), body.as_slice()),
            StblChild::Stsc(b) => panic!(
                "a malformed stsc must survive as Opaque, not a defaulted-empty typed box \
                 (got StblChild::Stsc with {} entries)",
                b.entries.len()
            ),
            other => panic!("expected StblChild::Opaque or StblChild::Stsc, got {other:?}"),
        }
    }

    /// The same walk over a well-formed `stsc` (entry_count = 0, the smallest
    /// box that still meets the 16-byte minimum) must still produce the typed
    /// variant — the fix must not have over-tightened parsing of genuinely
    /// valid boxes.
    #[test]
    fn parse_stbl_children_well_formed_stsc_stays_typed() {
        let mut body = alloc::vec![0u8; 16];
        body[0..4].copy_from_slice(&16u32.to_be_bytes());
        body[4..8].copy_from_slice(b"stsc");
        // version/flags already zero; entry_count (bytes 12..16) already zero.

        let children = parse_stbl_children(&body).unwrap();
        assert_eq!(children.len(), 1);
        match &children[0] {
            StblChild::Stsc(b) => assert!(b.entries.is_empty()),
            other => panic!("expected a well-formed empty StblChild::Stsc, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // bounded_entry_count / Vec::with_capacity DoS (audit finding #4)
    // -----------------------------------------------------------------------

    /// The audit's own scenario: a 16-byte `co64` (`BOX_HDR` + `FULL_HDR` +
    /// the 4-byte count field — no room left for even one 8-byte entry)
    /// declaring `count = 0xFFFFFFFF`. The *naive* `Vec::with_capacity(count)`
    /// would ask the allocator for ~32 GB; the bound must refuse that
    /// request outright, not merely complete it.
    #[test]
    fn bounded_entry_count_refuses_the_audits_co64_scenario() {
        let hostile_count = 0xFFFF_FFFFusize;
        let remaining = 0usize; // nothing left in a 16-byte co64 body
        let entry_len = 8usize; // co64 entries are 8-byte u64 offsets

        let bounded = bounded_entry_count(remaining, entry_len, hostile_count);

        let naive_bytes = hostile_count as u64 * entry_len as u64;
        assert!(
            naive_bytes > 30_000_000_000,
            "sanity check on the scenario itself: the naive request really is ~32 GB \
             ({naive_bytes} bytes)"
        );
        assert_eq!(bounded, 0, "no bytes remain for even one entry");
        assert!(
            (bounded as u64) * (entry_len as u64) < 1024,
            "the fix must request a bounded, sane allocation instead of {naive_bytes} bytes"
        );
    }

    /// The bound must not be so aggressive that it under-serves a
    /// legitimately-sized body: exactly as many entries fit as the remaining
    /// bytes can hold, and a genuinely smaller count is passed through
    /// untouched.
    #[test]
    fn bounded_entry_count_allows_exactly_what_fits() {
        assert_eq!(bounded_entry_count(40, 8, 0xFFFF_FFFF), 5);
        assert_eq!(
            bounded_entry_count(40, 8, 3),
            3,
            "a smaller, wire-verifiable count must pass through untouched"
        );
    }

    /// End-to-end: `ChunkLargeOffsetBox::parse` on the audit's exact `co64`
    /// scenario must reject the box rather than attempt to preallocate ~32 GB
    /// — `entry_count = 0xFFFFFFFF` cannot fit a 16-byte body (audit
    /// r05-W13: the old loop `break`ed and returned `Ok` with no entries).
    #[test]
    fn co64_hostile_count_is_rejected() {
        let mut body = alloc::vec![0u8; 16];
        body[0..4].copy_from_slice(&16u32.to_be_bytes());
        body[4..8].copy_from_slice(b"co64");
        body[12..16].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());

        let err = ChunkLargeOffsetBox::parse(&body)
            .expect_err("a count the body cannot hold is malformed");
        assert!(
            matches!(err, Error::BufferTooShort { .. }),
            "expected BufferTooShort, got {err:?}"
        );
    }

    /// `stsz` with a nonzero uniform `sample_size` never populates `entries`
    /// at all (§8.7.3) — a hostile `count` in that branch must drive *no*
    /// allocation whatsoever, not merely a bounded one.
    #[test]
    fn stsz_uniform_sample_size_ignores_hostile_count() {
        let mut body = alloc::vec![0u8; 20];
        body[0..4].copy_from_slice(&20u32.to_be_bytes());
        body[4..8].copy_from_slice(b"stsz");
        body[12..16].copy_from_slice(&64u32.to_be_bytes()); // uniform sample_size
        body[16..20].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // hostile count

        let parsed = SampleSizeBox::parse(&body).expect("a well-formed-enough header parses");
        assert_eq!(parsed.sample_size, 64);
        assert!(parsed.entries.is_empty());
    }

    /// r04-W45: every serializer must write its reserved/`pre_defined` bytes as
    /// zeros rather than skipping them, because `Serialize` accepts any
    /// `&mut [u8]` — a reused or dirty caller buffer would otherwise emit
    /// garbage that a conformance validator rejects. Serializing into a
    /// 0xFF-filled buffer must be byte-identical to `to_bytes()` (which
    /// zero-fills). Unfixed, the hdlr `pre_defined`/`reserved`, the smhd
    /// `reserved`, and the `mdhd` quality field all differed.
    #[test]
    fn reserved_bytes_written_as_zeros_into_a_dirty_buffer() {
        fn dirty_matches<T: Serialize>(value: &T, what: &str)
        where
            <T as Serialize>::Error: core::fmt::Debug,
        {
            let clean = value.to_bytes();
            let mut dirty = alloc::vec![0xFFu8; clean.len()];
            let written = value
                .serialize_into(&mut dirty)
                .expect("serialize into a correctly-sized buffer");
            assert_eq!(written, clean.len(), "{what}: serializer length");
            assert_eq!(
                dirty, clean,
                "{what}: reserializing into a 0xFF-filled buffer must equal to_bytes()"
            );
        }

        dirty_matches(&sample_mvhd_v0(), "mvhd");
        // mvhd v1 (the 64-bit form, issue #1015's 120-byte layout) has its own
        // reserved(10) + pre_defined(24) regions.
        let mut mvhd_v1 = sample_mvhd_v0();
        mvhd_v1.version = 1;
        dirty_matches(&mvhd_v1, "mvhd v1");
        dirty_matches(
            &MediaHeaderBox {
                version: 0,
                flags: 0,
                creation_time: 1,
                modification_time: 2,
                timescale: 90_000,
                duration: 180_000,
                language: 0x55C4,
            },
            "mdhd",
        );
        dirty_matches(
            &HandlerBox {
                version: 0,
                flags: 0,
                handler_type: *b"vide",
                name: alloc::vec![b'v', b'i', b'd', 0],
            },
            "hdlr",
        );
        dirty_matches(
            &SoundMediaHeaderBox {
                version: 0,
                flags: 0,
                balance: 0,
            },
            "smhd",
        );
        dirty_matches(
            &VideoMediaHeaderBox {
                version: 0,
                flags: 0,
                graphicsmode: 0,
                opcolor: [0, 0, 0],
            },
            "vmhd",
        );
        // tkhd v0 and v1 both carry reserved(4)+reserved[2](8)+reserved(2).
        for version in [0u8, 1] {
            dirty_matches(
                &TrackHeaderBox {
                    version,
                    flags: 0x000007,
                    creation_time: 1,
                    modification_time: 2,
                    track_id: 1,
                    duration: 1000,
                    layer: 0,
                    alternate_group: 0,
                    volume: 0x0100,
                    matrix: [0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000],
                    width: 640 << 16,
                    height: 360 << 16,
                },
                "tkhd",
            );
        }
    }

    /// The shared audio-sample-entry writer (`mp4a`/`ac-3`/`ec-3`/`Opus`/`fLaC`)
    /// skips 6 + 8 + 4 reserved/`pre_defined` bytes; each must be written as
    /// zeros (r04-W45). Serializing into a 0xFF-filled buffer must equal
    /// serializing into a fresh zeroed one.
    #[test]
    fn audio_sample_entry_reserved_bytes_written_as_zeros_into_a_dirty_buffer() {
        const NEED: usize = 36;
        let config_boxes: [OpaqueBox; 0] = [];
        for fourcc in [b"mp4a", b"ac-3", b"ec-3", b"Opus", b"fLaC"] {
            let mut clean = alloc::vec![0u8; NEED];
            let clean_len = serialize_audio_sample_entry(
                &mut clean,
                AudioSampleEntryFields {
                    fourcc,
                    entry_version: 0,
                    reserved_1: [0u8; 6],
                    data_reference_index: 1,
                    channelcount: 2,
                    samplesize: 16,
                    compression_id_and_packet_size: [0u8; 4],
                    samplerate: 48_000 << 16,
                    config_boxes: &config_boxes,
                },
            )
            .expect("serialize audio sample entry");
            assert_eq!(clean_len, NEED);

            let mut dirty = alloc::vec![0xFFu8; NEED];
            let dirty_len = serialize_audio_sample_entry(
                &mut dirty,
                AudioSampleEntryFields {
                    fourcc,
                    entry_version: 0,
                    reserved_1: [0u8; 6],
                    data_reference_index: 1,
                    channelcount: 2,
                    samplesize: 16,
                    compression_id_and_packet_size: [0u8; 4],
                    samplerate: 48_000 << 16,
                    config_boxes: &config_boxes,
                },
            )
            .expect("serialize into a dirty buffer");
            assert_eq!(dirty_len, NEED);
            assert_eq!(
                dirty, clean,
                "{fourcc:?}: a 0xFF-filled buffer must yield the same bytes"
            );
            // The reserved ranges are literally zero in the output.
            assert_eq!(&clean[8..14], &[0u8; 6], "{fourcc:?}: 6 reserved bytes");
            assert_eq!(
                &clean[16..24],
                &[0u8; 8],
                "{fourcc:?}: AudioSampleEntry reserved[2]"
            );
            assert_eq!(&clean[28..32], &[0u8; 4], "{fourcc:?}: predefined+reserved");
        }
    }
}
