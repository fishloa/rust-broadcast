//! Sans-IO Program Stream scanner — ISO/IEC 13818-1 §2.5.3.1–2.5.3.4
//! (Tables 2-37…2-41).
//!
//! [`PsScanner`] is fed the bytes of one Program Stream as they arrive and
//! yields the stream's structural units one at a time: a pack header, a system
//! header, a Program Stream Map, or a complete PES packet (with its
//! `stream_id`, the first byte of which selects the PES syntax, Table 2-22).
//!
//! It is the streaming sibling of [`parse_pack`](crate::program_stream::parse_pack):
//! that walker needs the whole pack (and, through
//! [`scan_packs`](crate::program_stream::scan_packs), the whole stream) in one
//! slice, while this one holds at most one incomplete unit. Every unit is
//! parsed by the crate's existing parser — no second parser lives here.
//!
//! # Bounds and recovery
//!
//! A PES packet is bounded by its 16-bit `PES_packet_length`
//! (ISO/IEC 13818-1 §2.4.3.7, Table 2-21), so a complete one is at most
//! `6 + 65535` bytes. [`PsLimits::max_pes_bytes`] is the caller's cap on that
//! hold; a packet that would exceed it is [`crate::Error::PesTooLarge`], never a
//! silent drop. A `PES_packet_length` of 0 (the unbounded form) is accepted
//! only up to the same cap and ends at the next `pack_start_code`.
//!
//! A stream that loses sync (a truncated capture, a corrupted pack) is
//! resynchronised on the next `pack_start_code` (`0x000001BA`), and the skipped
//! byte count is reported as [`PsEvent::Resync`] rather than swallowed. An
//! MPEG-1 pack header (`crate::Error::Mpeg1NotSupported`) is a typed error, not a
//! resync: it is a different stream layout, not damage.
//!
//! # Spec
//!
//! - Pack header: ISO/IEC 13818-1 §2.5.3.3, Table 2-39.
//! - System header: §2.5.3.4, Table 2-40.
//! - Program Stream Map: §2.5.4.3, Table 2-41.
//! - `stream_id` assignment and PES syntax: §2.4.3.6, Table 2-22.
//!
//! [`parse_pack`]: crate::program_stream::parse_pack
//! [`scan_packs`]: crate::program_stream::scan_packs

use alloc::vec::Vec;

use crate::PREFIX_LEN;
use crate::pack_header::{PACK_START_CODE, PackHeader};
use crate::program_stream_map::{MAP_STREAM_ID, ProgramStreamMap, program_stream_map_wire_len};
use crate::system_header::{SYSTEM_HEADER_START_CODE, SystemHeader};
use crate::wire::{read_u16_at, read_u32_at};
use broadcast_core::Parse;

use crate::{PROGRAM_END_CODE, Result};

/// A start code: the `00 00 01` prefix plus the `stream_id` byte.
const START_CODE_LEN: usize = core::mem::size_of::<u32>();
/// Byte offset of `stream_id` within a start code.
const STREAM_ID_OFFSET: usize = START_CODE_LEN - 1;
/// Byte offset of a system header's / PSM's 16-bit length field.
const LENGTH_OFFSET: usize = START_CODE_LEN;
/// A complete PES packet is at most `6 + u16::MAX` bytes: the 16-bit
/// `PES_packet_length` field (ISO/IEC 13818-1 §2.4.3.7, Table 2-21).
pub const MAX_PES_BYTES: usize = PREFIX_LEN + u16::MAX as usize;
/// Pack header bytes before the `pack_stuffing_length` byte.
const PACK_HEADER_FIXED_LEN: usize = 14;
/// Mask of the 3-bit `pack_stuffing_length` in byte 13 (Table 2-39).
const PACK_STUFFING_LENGTH_MASK: u8 = 0x07;
/// Trailing bytes retained when searching for a start code across feed
/// boundaries: a `00 00 01` prefix is three bytes.
const START_CODE_PREFIX_LEN: usize = crate::PACKET_START_CODE_PREFIX.len();

/// The caller's hostile-input bounds for [`PsScanner`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PsLimits {
    /// Largest PES packet the scanner will hold before reporting
    /// [`crate::Error::PesTooLarge`]. Defaults to [`MAX_PES_BYTES`], the largest a
    /// conformant 16-bit `PES_packet_length` can describe.
    pub max_pes_bytes: usize,
}

impl PsLimits {
    /// Bounds with the conformant maximum.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_pes_bytes: MAX_PES_BYTES,
        }
    }

    /// Set [`Self::max_pes_bytes`].
    #[must_use]
    pub const fn with_max_pes_bytes(mut self, max_pes_bytes: usize) -> Self {
        self.max_pes_bytes = max_pes_bytes;
        self
    }
}

impl Default for PsLimits {
    fn default() -> Self {
        Self::new()
    }
}

/// One structural unit of a Program Stream, as [`PsScanner::poll`] yields it.
///
/// Every borrowed variant borrows from the scanner's own buffer; the caller
/// must consume it before calling [`PsScanner::poll`] or
/// [`PsScanner::feed`] again.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PsEvent<'a> {
    /// A pack header (ISO/IEC 13818-1 §2.5.3.3, Table 2-39).
    Pack(PackHeader<'a>),
    /// A system header (§2.5.3.4, Table 2-40).
    SystemHeader(SystemHeader),
    /// A Program Stream Map (§2.5.4.3, Table 2-41).
    ProgramStreamMap(ProgramStreamMap<'a>),
    /// One complete PES packet: its `stream_id` (Table 2-22) and the packet
    /// bytes from the start-code prefix through its declared
    /// `PES_packet_length` (§2.4.3.7, Table 2-21).
    Pes {
        /// The `stream_id` byte (the fourth byte of the start code).
        stream_id: u8,
        /// The complete packet, start code included.
        packet: &'a [u8],
    },
    /// Bytes skipped while re-acquiring sync, up to the next
    /// `pack_start_code` (or the next unexamined byte when the prefix has not
    /// arrived yet).
    Resync {
        /// Count of bytes discarded since the last unit.
        skipped: u64,
    },
    /// The `MPEG_program_end_code` (`0x000001B9`) or the end of input was
    /// reached; nothing follows.
    End,
}

/// A sans-IO Program Stream scanner.
#[derive(Debug)]
pub struct PsScanner {
    buf: Vec<u8>,
    /// Offset of the first unconsumed byte in `buf`.
    start: usize,
    limits: PsLimits,
    finished: bool,
    ended: bool,
    /// The first parse failure the scanner met, for a caller that must report
    /// a stream with no recoverable pack as an error rather than an empty one.
    first_error: Option<crate::Error>,
}

impl PsScanner {
    /// A scanner bounded by `limits`.
    #[must_use]
    pub fn new(limits: PsLimits) -> Self {
        Self {
            buf: Vec::new(),
            start: 0,
            limits,
            finished: false,
            ended: false,
            first_error: None,
        }
    }

    /// The bytes currently held (unconsumed input plus one partial unit).
    #[must_use]
    pub fn buffered_len(&self) -> usize {
        self.buf.len().saturating_sub(self.start)
    }

    /// The first parse failure the scanner met, if any. A stream with no
    /// recoverable pack at all is reported from this; a recovered stream is not.
    #[must_use]
    pub fn first_error(&self) -> Option<&crate::Error> {
        self.first_error.as_ref()
    }

    /// Append `bytes` to the scanner.
    ///
    /// Never fails on its own: a bound is checked by [`poll`](Self::poll), once
    /// a unit's declared size is known.
    pub fn feed(&mut self, bytes: &[u8]) {
        if self.start > 0 {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// Signal that no more input will arrive.
    pub fn finish(&mut self) {
        self.finished = true;
    }

    /// The next structural unit, or `None` when the buffered bytes do not yet
    /// hold a complete one.
    ///
    /// # Errors
    ///
    /// [`crate::Error::PesTooLarge`] when a PES packet exceeds
    /// [`PsLimits::max_pes_bytes`], and [`crate::Error::Mpeg1NotSupported`] for an
    /// MPEG-1 pack header.
    pub fn poll(&mut self) -> Result<Option<PsEvent<'_>>> {
        if self.ended {
            return Ok(None);
        }
        let avail = &self.buf[self.start..];
        let avail_len = avail.len();
        if avail.is_empty() {
            if self.finished {
                self.ended = true;
                return Ok(Some(PsEvent::End));
            }
            return Ok(None);
        }

        // Re-acquire sync: every unit begins with a start-code prefix.
        if !avail.starts_with(&crate::PACKET_START_CODE_PREFIX[..]) {
            if self.first_error.is_none() && avail.len() >= START_CODE_LEN {
                self.first_error = Some(crate::Error::BadPackStartCode(read_u32_at(avail, 0)));
            }
            return match resync_skip(avail, self.limits.max_pes_bytes) {
                Some(skipped) => {
                    self.start += skipped;
                    Ok(Some(PsEvent::Resync {
                        skipped: skipped as u64,
                    }))
                }
                None => {
                    if let Some(skipped) = tail_report(self.finished, avail_len) {
                        self.start += avail_len;
                        return Ok(Some(PsEvent::Resync { skipped }));
                    }
                    return Ok(None);
                }
            };
        }
        if avail.len() < START_CODE_LEN {
            {
                if let Some(skipped) = tail_report(self.finished, avail_len) {
                    self.start += avail_len;
                    return Ok(Some(PsEvent::Resync { skipped }));
                }
                return Ok(None);
            }
        }
        let word = read_u32_at(avail, 0);
        if word == PROGRAM_END_CODE {
            self.start += START_CODE_LEN;
            self.ended = true;
            return Ok(Some(PsEvent::End));
        }
        if word == PACK_START_CODE {
            if avail.len() < PACK_HEADER_FIXED_LEN {
                {
                    if let Some(skipped) = tail_report(self.finished, avail_len) {
                        self.start += avail_len;
                        return Ok(Some(PsEvent::Resync { skipped }));
                    }
                    return Ok(None);
                }
            }
            let total = PACK_HEADER_FIXED_LEN
                + usize::from(avail[PACK_HEADER_FIXED_LEN - 1] & PACK_STUFFING_LENGTH_MASK);
            if avail.len() < total {
                {
                    if let Some(skipped) = tail_report(self.finished, avail_len) {
                        self.start += avail_len;
                        return Ok(Some(PsEvent::Resync { skipped }));
                    }
                    return Ok(None);
                }
            }
            return match PackHeader::parse(&avail[..total]) {
                Ok(header) => {
                    self.start += total;
                    Ok(Some(PsEvent::Pack(header)))
                }
                Err(crate::Error::Mpeg1NotSupported) => Err(crate::Error::Mpeg1NotSupported),
                Err(error) => {
                    if self.first_error.is_none() {
                        self.first_error = Some(error);
                    }
                    if let Some(skipped) = resync_skip(avail, self.limits.max_pes_bytes) {
                        self.start += skipped;
                        return Ok(Some(PsEvent::Resync {
                            skipped: skipped as u64,
                        }));
                    }
                    {
                        if let Some(skipped) = tail_report(self.finished, avail_len) {
                            self.start += avail_len;
                            return Ok(Some(PsEvent::Resync { skipped }));
                        }
                        return Ok(None);
                    }
                }
            };
        }
        if word == SYSTEM_HEADER_START_CODE {
            let Some(total) = declared_unit_len(avail) else {
                {
                    if let Some(skipped) = tail_report(self.finished, avail_len) {
                        self.start += avail_len;
                        return Ok(Some(PsEvent::Resync { skipped }));
                    }
                    return Ok(None);
                }
            };
            if avail.len() < total {
                {
                    if let Some(skipped) = tail_report(self.finished, avail_len) {
                        self.start += avail_len;
                        return Ok(Some(PsEvent::Resync { skipped }));
                    }
                    return Ok(None);
                }
            }
            return match SystemHeader::parse(&avail[..total]) {
                Ok(header) => {
                    self.start += total;
                    Ok(Some(PsEvent::SystemHeader(header)))
                }
                Err(_) => {
                    if let Some(skipped) = resync_skip(avail, self.limits.max_pes_bytes) {
                        self.start += skipped;
                        return Ok(Some(PsEvent::Resync {
                            skipped: skipped as u64,
                        }));
                    }
                    {
                        if let Some(skipped) = tail_report(self.finished, avail_len) {
                            self.start += avail_len;
                            return Ok(Some(PsEvent::Resync { skipped }));
                        }
                        return Ok(None);
                    }
                }
            };
        }
        if avail[STREAM_ID_OFFSET] == MAP_STREAM_ID {
            let Some(total) = declared_unit_len(avail) else {
                {
                    if let Some(skipped) = tail_report(self.finished, avail_len) {
                        self.start += avail_len;
                        return Ok(Some(PsEvent::Resync { skipped }));
                    }
                    return Ok(None);
                }
            };
            if avail.len() < total {
                {
                    if let Some(skipped) = tail_report(self.finished, avail_len) {
                        self.start += avail_len;
                        return Ok(Some(PsEvent::Resync { skipped }));
                    }
                    return Ok(None);
                }
            }
            return match ProgramStreamMap::parse(&avail[..total]) {
                Ok(map) => {
                    let wire = program_stream_map_wire_len(&avail[..total])?;
                    self.start += wire;
                    Ok(Some(PsEvent::ProgramStreamMap(map)))
                }
                Err(_) => {
                    if let Some(skipped) = resync_skip(avail, self.limits.max_pes_bytes) {
                        self.start += skipped;
                        return Ok(Some(PsEvent::Resync {
                            skipped: skipped as u64,
                        }));
                    }
                    {
                        if let Some(skipped) = tail_report(self.finished, avail_len) {
                            self.start += avail_len;
                            return Ok(Some(PsEvent::Resync { skipped }));
                        }
                        return Ok(None);
                    }
                }
            };
        }
        match pes_step(avail, &self.limits, self.finished)? {
            PesStep::Event {
                stream_id,
                packet,
                consumed,
            } => {
                self.start += consumed;
                Ok(Some(PsEvent::Pes { stream_id, packet }))
            }
            PesStep::Wait => {
                if let Some(skipped) = tail_report(self.finished, avail_len) {
                    self.start += avail_len;
                    return Ok(Some(PsEvent::Resync { skipped }));
                }
                Ok(None)
            }
            PesStep::Truncated => match resync_skip(avail, self.limits.max_pes_bytes) {
                Some(skipped) => {
                    self.start += skipped;
                    Ok(Some(PsEvent::Resync {
                        skipped: skipped as u64,
                    }))
                }
                None => {
                    if let Some(skipped) = tail_report(self.finished, avail_len) {
                        self.start += avail_len;
                        return Ok(Some(PsEvent::Resync { skipped }));
                    }
                    Ok(None)
                }
            },
        }
    }
}

/// The skipped-byte count to report for an unconsumed tail, once input has
/// ended; `None` means "wait for more bytes".
fn tail_report(finished: bool, len: usize) -> Option<u64> {
    finished.then_some(len as u64)
}

/// One step of PES parsing: a complete packet (borrowed from the input) or a
/// request for more bytes.
enum PesStep<'a> {
    Event {
        stream_id: u8,
        packet: &'a [u8],
        consumed: usize,
    },
    Wait,
    /// The packet's declared length lands somewhere that is not a start code:
    /// its bytes are missing, so its length must not swallow them.
    Truncated,
}

fn pes_step<'a>(avail: &'a [u8], limits: &PsLimits, finished: bool) -> Result<PesStep<'a>> {
    if avail.len() < PREFIX_LEN {
        return Ok(PesStep::Wait);
    }
    let declared = usize::from(read_u16_at(avail, LENGTH_OFFSET));
    let total = if declared == 0 {
        // The unbounded form: it ends where the next pack starts. Hold at
        // most `max_pes_bytes` while waiting for that boundary.
        match find_pack_start(&avail[PREFIX_LEN..]) {
            Some(offset) => PREFIX_LEN + offset,
            None => {
                if avail.len() >= limits.max_pes_bytes {
                    return Err(crate::Error::PesTooLarge {
                        declared: 0,
                        max: limits.max_pes_bytes,
                    });
                }
                return Ok(PesStep::Wait);
            }
        }
    } else {
        let total = PREFIX_LEN + declared;
        if total > limits.max_pes_bytes {
            return Err(crate::Error::PesTooLarge {
                declared,
                max: limits.max_pes_bytes,
            });
        }
        if avail.len() < total {
            // A declared length that runs past the bytes present *and* past a
            // pack boundary in them is a broken length field: the rest of the
            // pack would be swallowed. Resynchronise from this unit's own
            // first byte instead of waiting for bytes that will never come.
            if find_pack_start(&avail[PREFIX_LEN..]).is_some() {
                return Ok(PesStep::Truncated);
            }
            return Ok(PesStep::Wait);
        }
        // A complete PES packet is always followed by another start code
        // (a pack, a system header, a PSM, another PES, or the end code) -
        // that is how `parse_pes_loop` bounds a pack. A declared length
        // that lands anywhere else means the packet's bytes are missing
        // (a truncated capture), so its declared length must not swallow
        // the following pack: resync from its own first byte instead.
        if avail.len() >= total + START_CODE_LEN
            && !avail[total..].starts_with(&crate::PACKET_START_CODE_PREFIX[..])
        {
            return Ok(PesStep::Truncated);
        }
        if avail.len() < total + START_CODE_LEN && !finished {
            return Ok(PesStep::Wait);
        }
        total
    };
    Ok(PesStep::Event {
        stream_id: avail[STREAM_ID_OFFSET],
        packet: &avail[..total],
        consumed: total,
    })
}

/// Bytes to discard to reach the next `pack_start_code`, or `None` when the
/// buffered tail may still become one.
fn resync_skip(avail: &[u8], limit: usize) -> Option<usize> {
    match find_pack_start(&avail[1..]) {
        Some(offset) => Some(1 + offset),
        // Wait for the next pack start while the buffered tail could still
        // hold one; only a tail past the caller's cap is reported as lost.
        None if avail.len() > limit.max(START_CODE_PREFIX_LEN) => {
            Some(avail.len() - START_CODE_PREFIX_LEN)
        }
        None => None,
    }
}

/// The wire length a system header or PSM declares: `6 + length`
/// (ISO/IEC 13818-1 Tables 2-40 and 2-41), or `None` when the length field has
/// not arrived yet.
fn declared_unit_len(avail: &[u8]) -> Option<usize> {
    if avail.len() < PREFIX_LEN {
        return None;
    }
    Some(PREFIX_LEN + usize::from(read_u16_at(avail, LENGTH_OFFSET)))
}

/// Offset of the first `pack_start_code` in `data`, if any.
fn find_pack_start(data: &[u8]) -> Option<usize> {
    data.windows(START_CODE_LEN)
        .position(|window| read_u32_at(window, 0) == PACK_START_CODE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use broadcast_core::Serialize;

    /// A pack header with no stuffing (14 bytes), byte-identical to the
    /// crate's own round-trip fixture pattern.
    const PACK: [u8; 14] = [
        0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x43, 0x36, 0x3B, 0xF8,
    ];

    fn pes(stream_id: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0x00, 0x00, 0x01, stream_id];
        let body = 3 + 5 + payload.len(); // flags(2) + header_length(1) + PTS(5)
        out.extend_from_slice(&(body as u16).to_be_bytes());
        out.extend_from_slice(&[0x80, 0x80, 0x05, 0x21, 0x00, 0x01, 0x00, 0x01]);
        out.extend_from_slice(payload);
        out
    }

    fn drain(scanner: &mut PsScanner) -> Result<Vec<EventKind>> {
        let mut out = Vec::new();
        while let Some(event) = scanner.poll()? {
            match event {
                PsEvent::Pack(_) => out.push(EventKind::Pack),
                PsEvent::SystemHeader(_) => out.push(EventKind::SystemHeader),
                PsEvent::ProgramStreamMap(_) => out.push(EventKind::ProgramStreamMap),
                PsEvent::Pes { stream_id, packet } => {
                    out.push(EventKind::Pes(stream_id, packet.len()))
                }
                PsEvent::Resync { skipped } => out.push(EventKind::Resync(skipped)),
                PsEvent::End => out.push(EventKind::End),
            }
        }
        Ok(out)
    }

    #[derive(Debug, PartialEq, Eq)]
    enum EventKind {
        Pack,
        SystemHeader,
        ProgramStreamMap,
        Pes(u8, usize),
        Resync(u64),
        End,
    }

    #[test]
    fn a_pack_and_pes_split_across_feed_boundaries_are_recovered() {
        let mut stream = PACK.to_vec();
        stream.extend_from_slice(&pes(0xE0, &[0xAA, 0xBB, 0xCC]));
        let mut scanner = PsScanner::new(PsLimits::new());
        let mut events = Vec::new();
        for byte in &stream {
            scanner.feed(core::slice::from_ref(byte));
            events.extend(drain(&mut scanner).unwrap());
        }
        scanner.finish();
        events.extend(drain(&mut scanner).unwrap());
        assert_eq!(
            events,
            vec![
                EventKind::Pack,
                EventKind::Pes(0xE0, 6 + 3 + 5 + 3),
                EventKind::End
            ]
        );
    }

    #[test]
    fn a_truncated_pes_resyncs_on_the_next_pack_and_keeps_it() {
        let mut stream = PACK.to_vec();
        let first = pes(0xE0, &[0x11; 40]);
        // Drop the last 10 bytes of the first PES and append a whole pack.
        // Its declared length now lands inside that pack, so the scanner must
        // refuse the declared length and resync from its own first byte.
        let truncated_len = first.len() - 10;
        stream.extend_from_slice(&first[..truncated_len]);
        stream.extend_from_slice(&PACK);
        let mut scanner = PsScanner::new(PsLimits::new());
        scanner.feed(&stream);
        scanner.finish();
        let events = drain(&mut scanner).unwrap();
        assert_eq!(
            events,
            vec![
                EventKind::Pack,
                EventKind::Resync(truncated_len as u64),
                EventKind::Pack,
                EventKind::End
            ]
        );
    }

    /// A `PES_packet_length` declaring the 16-bit maximum with only its own
    /// header present must not make the scanner wait for (or reserve) the
    /// declared body: a pack boundary already in the supplied bytes is the
    /// evidence that the length is broken, so the unit resyncs immediately
    /// and the buffer stays the size of what was fed.
    #[test]
    fn a_maximum_pes_length_resyncs_at_the_pack_already_present() {
        let mut stream = PACK.to_vec();
        let mut pes_header = vec![0x00, 0x00, 0x01, 0xE0, 0xFF, 0xFF];
        pes_header.extend_from_slice(&[0x80, 0x80, 0x05, 0x21, 0x00, 0x01, 0x00, 0x01]);
        stream.extend_from_slice(&pes_header);
        stream.extend_from_slice(&PACK);
        let fed = stream.len();

        let mut scanner = PsScanner::new(PsLimits::new());
        scanner.feed(&stream);
        let events = drain(&mut scanner).unwrap();
        assert_eq!(
            events,
            vec![
                EventKind::Pack,
                EventKind::Resync(pes_header.len() as u64),
                EventKind::Pack,
            ],
            "the declared body is not waited for"
        );
        assert!(
            scanner.buffered_len() <= fed,
            "the buffer is bounded by the bytes supplied, not the declared length"
        );
    }

    /// Review focus (Phase 5 slice 1): 1000 bytes lost inside a PES must not
    /// swallow the packs that follow. The declared length lands in the middle
    /// of the next PES, so the scanner refuses it, resyncs on the first
    /// following pack, and that pack and its PES are delivered intact.
    #[test]
    fn a_pes_with_1000_bytes_missing_resyncs_and_keeps_the_following_pack() {
        let mut stream = PACK.to_vec();
        stream.extend_from_slice(&pes(0xE0, &[0x11; 2000]));
        stream.extend_from_slice(&PACK);
        stream.extend_from_slice(&pes(0xE0, &[0x22; 2000]));
        stream.extend_from_slice(&PACK);
        // Drop 1000 bytes from the middle of the first PES payload.
        let first_pes_at = PACK.len();
        stream.drain(first_pes_at + 6 + 500..first_pes_at + 6 + 1500);

        let mut scanner = PsScanner::new(PsLimits::new());
        scanner.feed(&stream);
        scanner.finish();
        let events = drain(&mut scanner).unwrap();
        assert_eq!(
            events,
            vec![
                EventKind::Pack,
                EventKind::Resync(1014),
                EventKind::Pack,
                EventKind::Pes(0xE0, 6 + 3 + 5 + 2000),
                EventKind::Pack,
                EventKind::End,
            ]
        );
    }

    #[test]
    fn an_oversized_pes_length_is_a_typed_error() {
        let mut pes_bytes = pes(0xE0, &[0x00; 4]);
        // Rewrite PES_packet_length far past the caller's cap.
        pes_bytes[4..6].copy_from_slice(&0xFFFF_u16.to_be_bytes());
        let mut scanner = PsScanner::new(PsLimits::new().with_max_pes_bytes(64));
        scanner.feed(&pes_bytes);
        assert!(matches!(
            scanner.poll(),
            Err(crate::Error::PesTooLarge { .. })
        ));
    }

    #[test]
    fn an_unbounded_pes_ends_at_the_next_pack() {
        let mut pes_bytes = pes(0xE0, &[0xAA; 8]);
        pes_bytes[4..6].copy_from_slice(&0x0000_u16.to_be_bytes());
        let mut stream = pes_bytes;
        stream.extend_from_slice(&PACK);
        let mut scanner = PsScanner::new(PsLimits::new());
        scanner.feed(&stream);
        let events = drain(&mut scanner).unwrap();
        assert_eq!(
            events,
            vec![EventKind::Pes(0xE0, 6 + 3 + 5 + 8), EventKind::Pack]
        );
    }

    #[test]
    fn a_program_stream_map_is_surfaced_and_skipped_whole() {
        let map = ProgramStreamMap {
            current_next_indicator: true,
            single_extension_stream_flag: false,
            version: 1,
            program_stream_info: crate::DescriptorLoop::new(&[]),
            elementary_stream_map: vec![crate::EsMapEntry {
                stream_type: 0x02,
                elementary_stream_id: 0xE0,
                stream_id_extension: None,
                pseudo_descriptor_tag: None,
                pseudo_descriptor_length: None,
                descriptors: crate::DescriptorLoop::new(&[]),
            }],
            crc: 0,
        };
        let mut psm_bytes = vec![0u8; map.serialized_len()];
        map.serialize_into(&mut psm_bytes).unwrap();
        let mut stream = psm_bytes.clone();
        stream.extend_from_slice(&PACK);
        let mut scanner = PsScanner::new(PsLimits::new());
        scanner.feed(&stream);
        scanner.finish();
        let events = drain(&mut scanner).unwrap();
        assert_eq!(
            events,
            vec![EventKind::ProgramStreamMap, EventKind::Pack, EventKind::End]
        );
    }

    #[test]
    fn a_garbage_prefix_is_reported_then_the_next_pack_is_delivered() {
        let mut stream = vec![0xAA; 1000];
        stream.extend_from_slice(&PACK);
        let mut scanner = PsScanner::new(PsLimits::new());
        scanner.feed(&stream);
        scanner.finish();
        let events = drain(&mut scanner).unwrap();
        assert_eq!(
            events,
            vec![EventKind::Resync(1000), EventKind::Pack, EventKind::End]
        );
    }

    #[test]
    fn an_mpeg1_pack_is_a_typed_error() {
        // `0010` SCR prefix: the ISO/IEC 11172-1 pack layout.
        let mut mpeg1 = PACK;
        mpeg1[4] = 0x20;
        let mut scanner = PsScanner::new(PsLimits::new());
        scanner.feed(&mpeg1);
        assert_eq!(scanner.poll(), Err(crate::Error::Mpeg1NotSupported));
    }

    #[test]
    fn the_buffer_never_grows_past_one_declared_unit() {
        let mut scanner = PsScanner::new(PsLimits::new().with_max_pes_bytes(32));
        scanner.feed(&[0x00; 4096]);
        // All of it is junk: the scanner reports sync loss instead of holding it.
        let mut total = 0usize;
        while let Some(event) = scanner.poll().unwrap() {
            match event {
                PsEvent::Resync { skipped } => total += skipped as usize,
                PsEvent::End => break,
                _ => {}
            }
        }
        assert!(total > 0);
        assert!(scanner.buffered_len() <= START_CODE_PREFIX_LEN);
    }

    #[test]
    fn a_zero_length_pack_stuffing_byte_means_no_stuffing() {
        let mut scanner = PsScanner::new(PsLimits::new());
        scanner.feed(&PACK);
        match scanner.poll().unwrap() {
            Some(PsEvent::Pack(header)) => {
                assert_eq!(header.header_len(), PACK.len());
                assert_eq!(header.stuffing_length, 0);
            }
            other => panic!("expected a pack, got {other:?}"),
        }
    }

    #[test]
    fn a_system_header_wire_length_is_honoured() {
        let mut stream = PACK.to_vec();
        stream.extend_from_slice(&[
            0x00, 0x00, 0x01, 0xBB, // system_header_start_code
            0x00, 0x08, // header_length = 8
            0x80, 0x00, 0x01, 0x00, 0x20, 0x00, // fixed body, 0 stream bounds
            0x00, 0x00, // reserved padding
        ]);
        stream.extend_from_slice(&pes(0xE0, &[0xAA, 0xBB]));
        let mut scanner = PsScanner::new(PsLimits::new());
        scanner.feed(&stream);
        scanner.finish();
        let events = drain(&mut scanner).unwrap();
        assert_eq!(
            events,
            vec![
                EventKind::Pack,
                EventKind::SystemHeader,
                EventKind::Pes(0xE0, 6 + 3 + 5 + 2),
                EventKind::End
            ]
        );
    }

    #[test]
    fn the_program_end_code_ends_the_scan_and_ignores_a_tail() {
        let mut stream = PACK.to_vec();
        stream.extend_from_slice(&[0x00, 0x00, 0x01, 0xB9, 0xDE, 0xAD]);
        let mut scanner = PsScanner::new(PsLimits::new());
        scanner.feed(&stream);
        let events = drain(&mut scanner).unwrap();
        assert_eq!(events, vec![EventKind::Pack, EventKind::End]);
        // The ended scanner never yields again, even after more input.
        scanner.feed(&PACK);
        assert_eq!(scanner.poll().unwrap(), None);
    }

    #[test]
    fn a_finished_scanner_reports_a_partial_tail_as_a_resync() {
        let mut scanner = PsScanner::new(PsLimits::new());
        scanner.feed(&[0x00, 0x00, 0x01, 0xE0, 0x00]);
        scanner.finish();
        let events = drain(&mut scanner).unwrap();
        assert_eq!(events, vec![EventKind::Resync(5), EventKind::End]);
    }
}
