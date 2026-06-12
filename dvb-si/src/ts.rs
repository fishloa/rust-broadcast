//! MPEG-TS packet parser + section reassembler. Feature-gated under `ts`.

use crate::error::{Error, Result};

/// Size of one MPEG-TS packet (ETSI EN 300 468 §3.2, ISO/IEC 13818-1 §2.4.3.2).
pub const TS_PACKET_SIZE: usize = 188;
/// Sync byte that every TS packet starts with (ISO/IEC 13818-1 §2.4.3.2).
pub const TS_SYNC_BYTE: u8 = 0x47;
/// Upper bound on a single section: `section_length` is 12 bits (max 4095)
/// plus the 3-byte header = 4098. (Long-form SI caps `section_length` at
/// 4093 → total 4096, but maximal short-form private sections may reach
/// 4098; the reassembler accepts the absolute ceiling.)
const MAX_SECTION_SIZE: usize = 4098;
/// Bytes before the `section_length` payload: `table_id` (1) + the two bytes
/// carrying the syntax/RFU flags and the 12-bit `section_length`
/// (ISO/IEC 13818-1 §2.4.4.1).
const SECTION_HEADER_LEN: usize = 3;
/// Mask for the 4 most-significant `section_length` bits in a section's second
/// byte (ISO/IEC 13818-1 §2.4.4.1 — `section_length` is 12 bits). Shared with
/// the packetizer in `mux.rs`.
pub(crate) const SECTION_LENGTH_HI_MASK: u8 = 0x0F;

/// ISO/IEC 13818-1 §2.4.3.3: transport header byte 1 bit 7 = tei (Transport Error Indicator).
const TEI_MASK: u8 = 0x80;
/// ISO/IEC 13818-1 §2.4.3.3: byte 1 bit 6 = pusi (Payload Unit Start Indicator).
const PUSI_MASK: u8 = 0x40;
/// ISO/IEC 13818-1 §2.4.3.3: byte 1 bits `[4:0]` = 13-bit PID (upper 5 bits).
pub const PID_MASK_HI: u8 = 0x1F;
/// ISO/IEC 13818-1 §2.4.3.3: byte 3 bits `[7:6]` = 2-bit scrambling control.
pub const SCRAMBLING_MASK: u8 = 0xC0;
/// ISO/IEC 13818-1 §2.4.3.3: byte 3 bit 5 = adaptation_field_control (bit 5 = 1 means adaptation present).
pub const ADAPTATION_FLAG: u8 = 0x20;
/// ISO/IEC 13818-1 §2.4.3.3: byte 3 bit 4 = adaptation_field_control (bit 4 = 1 means payload present).
pub const PAYLOAD_FLAG: u8 = 0x10;
/// ISO/IEC 13818-1 §2.4.3.3: byte 3 bits `[3:0]` = 4-bit continuity_counter.
pub const CC_MASK: u8 = 0x0F;

/// Parsed TS header — the 4-byte transport header fields.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TsHeader {
    /// Transport Error Indicator — set by the demodulator when an
    /// uncorrectable error is present in the packet.
    pub tei: bool,
    /// Payload Unit Start Indicator — first byte of the payload is a new
    /// PES packet or PSI section header when set.
    pub pusi: bool,
    /// 13-bit Packet Identifier.
    pub pid: u16,
    /// 2-bit transport_scrambling_control (0 = not scrambled).
    pub scrambling: u8,
    /// Adaptation field present flag (adaptation_field_control bit 1).
    pub has_adaptation: bool,
    /// Payload present flag (adaptation_field_control bit 0).
    pub has_payload: bool,
    /// 4-bit continuity_counter (wraps 0..=15 per PID).
    pub continuity_counter: u8,
}

/// Borrowed view into one 188-byte TS packet.
///
/// Serde: Serialize-only (re-parse from wire bytes to reconstruct). `raw` is
/// excluded from the serialized form because it is redundant once the header
/// has been parsed.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TsPacket<'a> {
    /// Parsed header fields.
    pub header: TsHeader,
    /// Slice into the packet's payload, or `None` when `has_payload == false`
    /// or the adaptation field consumed the whole packet body.
    pub payload: Option<&'a [u8]>,
    /// The adaptation-field bytes (after the length byte). Internal capture
    /// feeding [`adaptation_field`](Self::adaptation_field); not public.
    #[cfg_attr(feature = "serde", serde(skip))]
    adaptation: Option<&'a [u8]>,
    /// The raw 188 bytes of the packet — kept for cheap forwarding.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub raw: &'a [u8; TS_PACKET_SIZE],
}

impl TsHeader {
    /// Parse a 4-byte TS transport header.
    pub fn parse(raw4: &[u8]) -> Result<Self> {
        if raw4.len() < 4 {
            return Err(Error::BufferTooShort {
                need: 4,
                have: raw4.len(),
                what: "TsHeader",
            });
        }
        let b1 = raw4[1];
        let b2 = raw4[2];
        let b3 = raw4[3];

        let tei = (b1 & TEI_MASK) != 0;
        let pusi = (b1 & PUSI_MASK) != 0;
        let pid = (((b1 & PID_MASK_HI) as u16) << 8) | (b2 as u16);
        let scrambling = (b3 & SCRAMBLING_MASK) >> 6;
        let has_adaptation = (b3 & ADAPTATION_FLAG) != 0;
        let has_payload = (b3 & PAYLOAD_FLAG) != 0;
        let continuity_counter = b3 & CC_MASK;

        Ok(Self {
            tei,
            pusi,
            pid,
            scrambling,
            has_adaptation,
            has_payload,
            continuity_counter,
        })
    }

    /// Number of bytes written by [`serialize_into`](Self::serialize_into).
    pub const fn serialized_len() -> usize {
        4
    }

    /// Serialize this header into the first 4 bytes of `buf`.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < 4 {
            return Err(Error::OutputBufferTooSmall {
                need: 4,
                have: buf.len(),
            });
        }
        buf[0] = TS_SYNC_BYTE;
        buf[1] = 0;
        if self.tei {
            buf[1] |= TEI_MASK;
        }
        if self.pusi {
            buf[1] |= PUSI_MASK;
        }
        buf[1] |= ((self.pid >> 8) as u8) & PID_MASK_HI;
        buf[2] = (self.pid & 0xFF) as u8;
        buf[3] = (self.scrambling << 6) & SCRAMBLING_MASK;
        if self.has_adaptation {
            buf[3] |= ADAPTATION_FLAG;
        }
        if self.has_payload {
            buf[3] |= PAYLOAD_FLAG;
        }
        buf[3] |= self.continuity_counter & CC_MASK;
        Ok(4)
    }
}

impl<'a> dvb_common::Parse<'a> for TsHeader {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        TsHeader::parse(bytes)
    }
}

impl dvb_common::Serialize for TsHeader {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        TsHeader::serialized_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        TsHeader::serialize_into(self, buf)
    }
}

impl<'a> TsPacket<'a> {
    /// Parse a single 188-byte TS packet from a buffer.
    ///
    /// Returns `Err(Error::InvalidSyncByte)` if the first byte is not `0x47`,
    /// `Err(Error::BufferTooShort)` if fewer than 188 bytes, or `Ok` with
    /// the parsed packet otherwise.
    pub fn parse(buf: &'a [u8]) -> Result<Self> {
        if buf.len() < TS_PACKET_SIZE {
            return Err(Error::BufferTooShort {
                need: TS_PACKET_SIZE,
                have: buf.len(),
                what: "TsPacket",
            });
        }
        if buf[0] != TS_SYNC_BYTE {
            return Err(Error::InvalidSyncByte { found: buf[0] });
        }

        let raw: &[u8; TS_PACKET_SIZE] =
            buf[..TS_PACKET_SIZE]
                .try_into()
                .map_err(|_| Error::BufferTooShort {
                    need: TS_PACKET_SIZE,
                    have: buf.len(),
                    what: "TsPacket::parse (array conversion)",
                })?;

        let header = TsHeader::parse(&raw[..4])?;

        let mut cursor = 4usize;
        let mut payload = None;
        let mut adaptation = None;

        // Capture the adaptation field if present, then skip it (the section
        // path does not need it; decode lazily via `adaptation_field`).
        if header.has_adaptation && cursor < TS_PACKET_SIZE {
            let af_len = raw[cursor] as usize;
            let af_start = cursor + 1;
            if af_len > 0 && af_start < TS_PACKET_SIZE {
                let af_end = (af_start + af_len).min(TS_PACKET_SIZE);
                adaptation = Some(&raw[af_start..af_end]);
            }
            cursor += 1 + af_len;
        }

        if header.has_payload && cursor < TS_PACKET_SIZE {
            payload = Some(&raw[cursor..]);
        }

        Ok(TsPacket {
            header,
            payload,
            adaptation,
            raw,
        })
    }

    /// Decode the adaptation field, if present.
    ///
    /// Returns `None` when the packet carries no adaptation field, and
    /// `Some(Err(..))` when a present field is truncated. Layout per
    /// ISO/IEC 13818-1:2007 §2.4.3.4 (`docs/iso_13818_1_systems.md`).
    pub fn adaptation_field(&self) -> Option<crate::Result<AdaptationField>> {
        self.adaptation.map(AdaptationField::parse)
    }
}

// Adaptation-field flag bits, byte 0 (ISO/IEC 13818-1:2007 §2.4.3.4).
const AF_DISCONTINUITY: u8 = 0x80;
const AF_RANDOM_ACCESS: u8 = 0x40;
const AF_ES_PRIORITY: u8 = 0x20;
const AF_PCR_FLAG: u8 = 0x10;
const AF_OPCR_FLAG: u8 = 0x08;
const AF_SPLICING_FLAG: u8 = 0x04;
/// Encoded PCR / OPCR field width: 33-bit base + 6 reserved + 9-bit extension.
const PCR_FIELD_LEN: usize = 6;

/// Program Clock Reference (ISO/IEC 13818-1:2007 §2.4.3.5): a 33-bit base on a
/// 90 kHz clock plus a 9-bit extension on a 27 MHz clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Pcr {
    /// 33-bit base (90 kHz units).
    pub base: u64,
    /// 9-bit extension (27 MHz units).
    pub extension: u16,
}

impl Pcr {
    /// Full PCR value on the 27 MHz clock: `base * 300 + extension`.
    #[must_use]
    pub fn as_27mhz(self) -> u64 {
        self.base * 300 + self.extension as u64
    }

    /// Decode the 6-byte PCR/OPCR field starting at `at` within `af`.
    fn parse(af: &[u8], at: usize) -> Result<Self> {
        let b: &[u8; PCR_FIELD_LEN] = af
            .get(at..at + PCR_FIELD_LEN)
            .and_then(|s| s.try_into().ok())
            .ok_or(Error::BufferTooShort {
                need: at + PCR_FIELD_LEN,
                have: af.len(),
                what: "adaptation_field PCR",
            })?;
        let base = ((b[0] as u64) << 25)
            | ((b[1] as u64) << 17)
            | ((b[2] as u64) << 9)
            | ((b[3] as u64) << 1)
            | ((b[4] as u64) >> 7);
        let extension = (((b[4] & 0x01) as u16) << 8) | (b[5] as u16);
        Ok(Self { base, extension })
    }
}

/// Decoded adaptation field — flags plus PCR/OPCR and splice point per
/// ISO/IEC 13818-1:2007 §2.4.3.4. Transport-private data and the
/// adaptation-field extension are not yet surfaced; more fields may be
/// added in future releases.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AdaptationField {
    /// A timing/continuity discontinuity starts at this packet.
    pub discontinuity_indicator: bool,
    /// This packet is a random-access point.
    pub random_access_indicator: bool,
    /// Elementary-stream priority hint.
    pub elementary_stream_priority_indicator: bool,
    /// Program Clock Reference, present iff the PCR flag is set.
    pub pcr: Option<Pcr>,
    /// Original PCR, present iff the OPCR flag is set.
    pub opcr: Option<Pcr>,
    /// Splice countdown (packets until the splice point), iff the flag is set.
    pub splice_countdown: Option<i8>,
}

impl AdaptationField {
    /// Parse the adaptation-field bytes (those following the length byte).
    fn parse(af: &[u8]) -> Result<Self> {
        let flags = *af.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "adaptation_field flags",
        })?;
        let mut cursor = 1usize;

        let pcr = if flags & AF_PCR_FLAG != 0 {
            let p = Pcr::parse(af, cursor)?;
            cursor += PCR_FIELD_LEN;
            Some(p)
        } else {
            None
        };
        let opcr = if flags & AF_OPCR_FLAG != 0 {
            let p = Pcr::parse(af, cursor)?;
            cursor += PCR_FIELD_LEN;
            Some(p)
        } else {
            None
        };
        let splice_countdown = if flags & AF_SPLICING_FLAG != 0 {
            let b = *af.get(cursor).ok_or(Error::BufferTooShort {
                need: cursor + 1,
                have: af.len(),
                what: "adaptation_field splice_countdown",
            })?;
            Some(b as i8)
        } else {
            None
        };

        Ok(AdaptationField {
            discontinuity_indicator: flags & AF_DISCONTINUITY != 0,
            random_access_indicator: flags & AF_RANDOM_ACCESS != 0,
            elementary_stream_priority_indicator: flags & AF_ES_PRIORITY != 0,
            pcr,
            opcr,
            splice_countdown,
        })
    }
}

/// Reassembles PSI/SI sections from TS packets on a single PID.
///
/// Feed each TS packet's payload with `feed`. Complete sections are
/// appended to an internal queue; drain them with `pop_section`.
#[derive(Default)]
pub struct SectionReassembler {
    buf: bytes::BytesMut,
    ready: std::collections::VecDeque<bytes::Bytes>,
}

impl SectionReassembler {
    /// Feed a TS payload and whether its packet had PUSI set.
    ///
    /// Extracts complete SI sections into the internal queue. A single call
    /// can produce zero, one, or **several** sections — a payload may
    /// concatenate multiple complete sections after the `pointer_field`
    /// (EN 300 468 §5.1.4; common on EMM PIDs). Drain with a
    /// `while let Some(s) = r.pop_section()` loop, not a single `if let`.
    pub fn feed(&mut self, payload: &[u8], pusi: bool) {
        if pusi {
            // A PUSI packet whose adaptation field consumed the whole body is
            // malformed but constructible — drop sync rather than panic.
            if payload.is_empty() {
                self.buf.clear();
                return;
            }
            let pointer = payload[0] as usize;

            // The `pointer_field` counts bytes that belong to a section still
            // in progress from a previous packet (ISO/IEC 13818-1 §2.4.4): the
            // `pointer` bytes immediately after it are that section's tail and
            // must complete it BEFORE new sections begin at `1 + pointer`.
            // Skipping them (or clearing `buf` first) drops any section that
            // spans into a PUSI packet — silent loss biased toward whichever
            // section happens to straddle a packet boundary.
            if !self.buf.is_empty() && pointer > 0 {
                let avail = payload.len() - 1;
                let tail_len = pointer.min(avail);
                if self.buf.len() + tail_len > MAX_SECTION_SIZE {
                    self.buf.clear();
                } else {
                    self.buf.extend_from_slice(&payload[1..1 + tail_len]);
                    self.drain_complete_sections();
                }
            }

            // New sections start at `1 + pointer`; anything still buffered is
            // an incomplete (corrupt / lost-packet) section — discard it.
            self.buf.clear();

            let start = 1 + pointer;
            if start >= payload.len() {
                // Pointer spans to (or past) the end — no new section here.
                return;
            }
            let new_data = &payload[start..];
            if new_data.len() > MAX_SECTION_SIZE {
                return;
            }
            self.buf.extend_from_slice(new_data);
        } else {
            if self.buf.is_empty() {
                return;
            }
            // Append only the bytes the in-progress section still needs. A new
            // section cannot start in a continuation (non-PUSI) packet
            // (ISO/IEC 13818-1 §2.4.4), so once the section's declared length
            // is satisfied the remaining payload bytes are 0xFF stuffing and
            // are ignored. Counting that stuffing toward `MAX_SECTION_SIZE`
            // previously dropped valid near-maximal sections (#148). Because
            // the 12-bit `section_length` caps a section at `MAX_SECTION_SIZE`,
            // `take` is inherently bounded and the buffer cannot grow without
            // limit.
            let take = if self.buf.len() >= SECTION_HEADER_LEN {
                let exp = SECTION_HEADER_LEN
                    + (((self.buf[1] & SECTION_LENGTH_HI_MASK) as usize) << 8
                        | self.buf[2] as usize);
                exp.saturating_sub(self.buf.len()).min(payload.len())
            } else {
                // Header not yet complete (split across the packet boundary) —
                // take enough to read `section_length` on the next drain,
                // bounded by the maximum possible section size.
                payload.len().min(MAX_SECTION_SIZE - self.buf.len())
            };
            self.buf.extend_from_slice(&payload[..take]);
        }

        self.drain_complete_sections();
    }

    /// Queue every complete section the buffer currently holds.
    ///
    /// A single TS payload may concatenate multiple complete sections after
    /// the `pointer_field` (legal per ETSI EN 300 468 §5.1.4 and common on
    /// EMM PIDs, which pack several short messages into one payload). We must
    /// keep extracting until the buffer holds only a partial (multi-packet
    /// spanning) section, whose bytes stay buffered for the next packet to
    /// continue (the expected length is recomputed from the section header on
    /// each drain). A `0xFF` where a `table_id` is expected marks the rest of
    /// the payload as stuffing.
    fn drain_complete_sections(&mut self) {
        loop {
            if self.buf.len() < SECTION_HEADER_LEN {
                // Not enough for a section header yet; keep the partial bytes
                // and wait for the next packet to complete the header.
                break;
            }
            if self.buf[0] == 0xFF {
                // Stuffing where a table_id is expected — payload tail is fill.
                self.buf.clear();
                break;
            }
            let exp = SECTION_HEADER_LEN
                + (((self.buf[1] & SECTION_LENGTH_HI_MASK) as usize) << 8 | self.buf[2] as usize);
            if self.buf.len() >= exp {
                // split_to returns the first `exp` bytes as an owned BytesMut,
                // leaving the remainder in self.buf — cheap (shifts pointers).
                let section = self.buf.split_to(exp).freeze();
                self.ready.push_back(section);
            } else {
                // Partial section spanning into later packets.
                break;
            }
        }
    }

    /// Pop one complete section. Returns `None` when the queue is empty.
    pub fn pop_section(&mut self) -> Option<bytes::Bytes> {
        self.ready.pop_front()
    }

    /// Number of bytes currently buffered (incomplete section).
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// True if no bytes are currently buffered.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: construct a minimal 188-byte TS packet buffer with given header flags and payload.
    fn make_packet(b1: u8, b2: u8, b3: u8, payload_data: &[u8]) -> [u8; TS_PACKET_SIZE] {
        let mut pkt = [0u8; TS_PACKET_SIZE];
        pkt[0] = TS_SYNC_BYTE;
        pkt[1] = b1;
        pkt[2] = b2;
        pkt[3] = b3;
        let payload_start = 4;
        let end = (payload_start + payload_data.len()).min(TS_PACKET_SIZE);
        let len = (end - payload_start).min(payload_data.len());
        pkt[payload_start..payload_start + len].copy_from_slice(&payload_data[..len]);
        pkt
    }

    #[test]
    fn parse_rejects_non_0x47_sync_byte() {
        let mut pkt = [0u8; TS_PACKET_SIZE];
        pkt[0] = 0x46; // wrong sync byte
        let err = TsPacket::parse(&pkt).unwrap_err();
        match err {
            Error::InvalidSyncByte { found } => assert_eq!(found, 0x46),
            other => panic!("expected InvalidSyncByte, got {other:?}"),
        }
    }

    #[test]
    fn ts_header_round_trip() {
        // struct → serialize → parse must reproduce the header (the project's
        // symmetric Parse/Serialize invariant) across flag/field combinations.
        let cases = [
            TsHeader {
                tei: false,
                pusi: true,
                pid: 0x0000,
                scrambling: 0,
                has_adaptation: false,
                has_payload: true,
                continuity_counter: 0,
            },
            TsHeader {
                tei: true,
                pusi: false,
                pid: 0x1FFF,
                scrambling: 0b11,
                has_adaptation: true,
                has_payload: true,
                continuity_counter: 0x0F,
            },
            TsHeader {
                tei: false,
                pusi: false,
                pid: 0x0100,
                scrambling: 0b10,
                has_adaptation: true,
                has_payload: false,
                continuity_counter: 7,
            },
        ];
        for h in cases {
            let mut buf = [0u8; 4];
            assert_eq!(h.serialize_into(&mut buf).unwrap(), 4);
            assert_eq!(TsHeader::parse(&buf).unwrap(), h, "round-trip mismatch");
        }
    }

    #[test]
    fn parse_extracts_pid_and_continuity_counter() {
        // PID = 0x1234 → upper 5 bits = 0x12, lower 8 bits = 0x34
        // CC = 5 → 0x05
        // b1 bits: [tei:1][pusi:1][pid_hi:5]
        // pid_hi = 0x12 = 0b00100_10 → bits 5..=1 = 0x12
        // b1 = 0b00_010010 = 0x12 (no tei, no pusi)
        let pkt = make_packet(0x12, 0x34, 0x05, &[]);
        let pkt = TsPacket::parse(&pkt).unwrap();
        assert_eq!(pkt.header.pid, 0x1234);
        assert_eq!(pkt.header.continuity_counter, 5);
    }

    #[test]
    fn payload_unit_start_indicator_flag_extracted() {
        // b1 = 0x40 → pusi = true (bit 6 set, no tei, no pid bits)
        let pkt1 = make_packet(0x40, 0x00, 0x00, &[]);
        let pkt1 = TsPacket::parse(&pkt1).unwrap();
        assert!(pkt1.header.pusi);

        // b1 = 0x00 → pusi = false
        let pkt2 = make_packet(0x00, 0x00, 0x00, &[]);
        let pkt2 = TsPacket::parse(&pkt2).unwrap();
        assert!(!pkt2.header.pusi);
    }

    /// Build a PSI-carrying TS payload: `pointer_field` byte followed by
    /// (optionally) some tail of a previous section, followed by a fresh
    /// section. `pointer_field` is the number of bytes of the previous
    /// section that precede the new one (per ETSI EN 300 468 §5.1.4).
    fn build_pusi_payload(pointer_field: u8, previous_tail: &[u8], section: &[u8]) -> Vec<u8> {
        assert_eq!(pointer_field as usize, previous_tail.len());
        let mut v = Vec::with_capacity(1 + previous_tail.len() + section.len());
        v.push(pointer_field);
        v.extend_from_slice(previous_tail);
        v.extend_from_slice(section);
        v
    }

    /// Build a long-form section with the given table_id and body bytes.
    /// Returns the full section including its 3-byte + 5-byte header and a
    /// placeholder CRC — for reassembler testing we don't validate the CRC.
    fn build_section(table_id: u8, body_after_length: &[u8]) -> Vec<u8> {
        let section_length = body_after_length.len() as u16;
        let mut v = Vec::with_capacity(3 + section_length as usize);
        v.push(table_id);
        // ssi=1, pi=0, reserved=11, length hi 4 bits
        v.push(0xB0 | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(body_after_length);
        v
    }

    // The reassembler tests below feed raw payload slices directly to
    // `feed()` rather than wrapping them in 188-byte TS packets. This avoids
    // the TS stuffing-byte tail (0xFF padding) bleeding into the reassembled
    // section and keeps the assertions exact.

    #[test]
    fn reassembler_accumulates_multi_packet_section() {
        // 200-byte section that spans two payload slices.
        let body = vec![0xAAu8; 197];
        let section = build_section(0x02, &body);
        assert_eq!(section.len(), 200);

        let first_chunk = 100;
        let payload1 = build_pusi_payload(0, &[], &section[..first_chunk]);
        let payload2 = section[first_chunk..].to_vec();

        let mut reasm = SectionReassembler::default();
        reasm.feed(&payload1, true);
        reasm.feed(&payload2, false);

        let out = reasm.pop_section().expect("section should be ready");
        assert_eq!(out.len(), 200);
        assert_eq!(out.as_ref(), &section[..]);
    }

    #[test]
    fn reassembler_yields_complete_section_once_length_satisfied() {
        // 1-byte-body section: table_id=0x42, section_length=1, total=4 bytes.
        let section = build_section(0x42, &[0xAA]);
        assert_eq!(section.len(), 4);
        let payload = build_pusi_payload(0, &[], &section);

        let mut reasm = SectionReassembler::default();
        reasm.feed(&payload, true);

        let out = reasm
            .pop_section()
            .expect("single-packet section should pop");
        assert_eq!(out.as_ref(), &section[..]);
    }

    #[test]
    fn reassembler_extracts_all_concatenated_sections_in_one_payload() {
        // Issue #29: a single PUSI payload packing three complete short
        // sections after the pointer_field. All three must be queued — the
        // old `feed` stopped after the first and the rest were silently lost
        // (the CAS/EMM data-loss bug: SHARED EMMs landing as the 2nd+ section).
        let s1 = build_section(0x42, &[0x11, 0x22]); // 5 bytes
        let s2 = build_section(0x46, &[0x33]); // 4 bytes
        let s3 = build_section(0x4A, &[0x44, 0x55, 0x66]); // 6 bytes

        let mut concat = Vec::new();
        concat.extend_from_slice(&s1);
        concat.extend_from_slice(&s2);
        concat.extend_from_slice(&s3);
        let payload = build_pusi_payload(0, &[], &concat);

        let mut reasm = SectionReassembler::default();
        reasm.feed(&payload, true);

        // Consumers must drain with a loop, not a single `if let`.
        let got: Vec<_> = std::iter::from_fn(|| reasm.pop_section()).collect();
        assert_eq!(got.len(), 3, "all three concatenated sections must pop");
        assert_eq!(got[0].as_ref(), &s1[..]);
        assert_eq!(got[1].as_ref(), &s2[..]);
        assert_eq!(got[2].as_ref(), &s3[..]);
    }

    #[test]
    fn reassembler_stops_at_stuffing_after_concatenated_sections() {
        // Two sections then 0xFF stuffing fill — the stuffing must not be
        // mistaken for a section header (0xFF table_id) nor leak into a
        // section; both real sections still pop.
        let s1 = build_section(0x42, &[0xAA]); // 4 bytes
        let s2 = build_section(0x46, &[0xBB, 0xCC]); // 5 bytes
        let mut concat = Vec::new();
        concat.extend_from_slice(&s1);
        concat.extend_from_slice(&s2);
        concat.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]); // stuffing tail
        let payload = build_pusi_payload(0, &[], &concat);

        let mut reasm = SectionReassembler::default();
        reasm.feed(&payload, true);

        let got: Vec<_> = std::iter::from_fn(|| reasm.pop_section()).collect();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].as_ref(), &s1[..]);
        assert_eq!(got[1].as_ref(), &s2[..]);
        assert!(
            reasm.is_empty(),
            "stuffing tail must be discarded, not buffered"
        );
    }

    #[test]
    fn reassembler_concatenated_then_spanning_tail() {
        // One complete section followed by the head of a second that spans
        // into a continuation packet: first pops immediately, second pops
        // once the continuation arrives.
        let s1 = build_section(0x42, &[0x01, 0x02]); // 5 bytes
        let s2 = build_section(0x46, &[0x09u8; 60]); // 63 bytes
        let split = 30;

        let mut head = Vec::new();
        head.extend_from_slice(&s1);
        head.extend_from_slice(&s2[..split]);
        let payload1 = build_pusi_payload(0, &[], &head);
        let payload2 = s2[split..].to_vec();

        let mut reasm = SectionReassembler::default();
        reasm.feed(&payload1, true);
        let first = reasm.pop_section().expect("first section pops at once");
        assert_eq!(first.as_ref(), &s1[..]);
        assert!(reasm.pop_section().is_none(), "second is still partial");

        reasm.feed(&payload2, false);
        let second = reasm.pop_section().expect("second pops after continuation");
        assert_eq!(second.as_ref(), &s2[..]);
    }

    #[test]
    fn reassembler_completes_section_spanning_into_pusi_packet() {
        // Issue #29 (second case): a section starts late in packet A and spills
        // into packet B, but B is itself PUSI=1 because new sections begin in it.
        // B's pointer_field = the count of leading tail bytes belonging to the
        // section from A. Those bytes MUST complete A's section before new
        // sections start. 3.1.1 cleared buf + skipped them → the spanning
        // section was lost (the SHARED EMM the smartcard needed).
        let spanning = build_section(0x42, &[0x5Au8; 62]); // 65 bytes
        let head = 41;
        let tail = &spanning[head..]; // 24 bytes — lands in packet B
        assert_eq!(tail.len(), 24);

        // New section that begins in packet B after the spanning tail.
        let next = build_section(0x46, &[0x77, 0x88]); // 5 bytes

        // Packet A (PUSI): pointer 0, then the 41-byte head (incomplete).
        let payload_a = build_pusi_payload(0, &[], &spanning[..head]);
        // Packet B (PUSI): pointer = 24 (tail of A's section), then `next`.
        let payload_b = build_pusi_payload(24, tail, &next);

        let mut reasm = SectionReassembler::default();
        reasm.feed(&payload_a, true);
        assert!(reasm.pop_section().is_none(), "head alone is incomplete");

        reasm.feed(&payload_b, true);
        let got: Vec<_> = std::iter::from_fn(|| reasm.pop_section()).collect();
        assert_eq!(got.len(), 2, "spanning section + new section must both pop");
        assert_eq!(
            got[0].as_ref(),
            &spanning[..],
            "spanning section completed from B's pointer tail"
        );
        assert_eq!(got[1].as_ref(), &next[..]);
    }

    #[test]
    fn reassembler_pusi_pointer_spans_whole_payload() {
        // A section spans into a PUSI packet whose pointer covers the ENTIRE
        // remaining payload (no new section starts here) — the tail must be
        // appended and the section completed once the count is satisfied.
        let spanning = build_section(0x42, &[0x33u8; 40]); // 43 bytes
        let head = 20;
        let payload_a = build_pusi_payload(0, &[], &spanning[..head]);
        let tail = &spanning[head..]; // 23 bytes — exactly the rest of payload B

        let mut reasm = SectionReassembler::default();
        reasm.feed(&payload_a, true);
        // Packet B: pointer = 23 = all remaining bytes; no new section follows.
        reasm.feed(&build_pusi_payload_pointer_spanning_all(tail), true);

        let out = reasm.pop_section().expect("spanning section completes");
        assert_eq!(out.as_ref(), &spanning[..]);
        assert!(reasm.pop_section().is_none());
    }

    /// Build a PUSI payload whose `pointer_field` equals the whole tail (so the
    /// pointer spans to the end of the payload and no new section starts).
    fn build_pusi_payload_pointer_spanning_all(tail: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(1 + tail.len());
        v.push(tail.len() as u8);
        v.extend_from_slice(tail);
        v
    }

    #[test]
    fn reassembler_completes_max_length_section_and_stays_usable() {
        // A section declaring the maximum `section_length` (0xFFF → 4098 bytes
        // total). The 12-bit length structurally caps the buffer at
        // MAX_SECTION_SIZE, so there is no unbounded growth — and (unlike the
        // pre-#148 guard, which discarded once buf+payload crossed the cap) a
        // valid max-length section completes at its declared length.
        let mut section = Vec::with_capacity(MAX_SECTION_SIZE);
        section.push(0x00); // table_id
        section.push(0xB0 | ((4095u16 >> 8) as u8 & 0x0F));
        section.push(0xFF); // section_length = 0xFFF
        section.resize(MAX_SECTION_SIZE, 0u8);
        assert_eq!(section.len(), MAX_SECTION_SIZE);

        let mut reasm = SectionReassembler::default();
        let mut first = vec![0x00u8]; // pointer_field 0
        first.extend_from_slice(&section[..183]);
        reasm.feed(&first, true);
        assert!(
            reasm.pop_section().is_none(),
            "incomplete until the declared length arrives"
        );

        for chunk in section[183..].chunks(184) {
            reasm.feed(chunk, false);
        }
        let out = reasm
            .pop_section()
            .expect("max-length section completes at its declared length");
        assert_eq!(out.len(), MAX_SECTION_SIZE);
        assert_eq!(out.as_ref(), &section[..]);
        assert!(reasm.is_empty());

        // Extra trailing continuation data after completion is ignored (the
        // buffer is empty, so a non-PUSI payload is dropped) — no panic, no
        // spurious section.
        reasm.feed(&[0u8; 184], false);
        assert!(reasm.pop_section().is_none());

        // State must be resettable — a fresh valid PUSI section works.
        let valid_section = build_section(0x00, &[0xAA]);
        let payload2 = build_pusi_payload(0, &[], &valid_section);
        reasm.feed(&payload2, true);
        let out = reasm
            .pop_section()
            .expect("fresh section should pop after reset");
        assert_eq!(out.as_ref(), &valid_section[..]);
    }

    #[test]
    fn reassembler_handles_pusi_with_nonzero_pointer_field() {
        // payload = pointer_field=3, 3 bytes of prior-section tail, then new section.
        let prior_tail = vec![0x11, 0x22, 0x33];
        let new_section = build_section(0x02, &[0xBB]);
        assert_eq!(new_section.len(), 4);
        let payload = build_pusi_payload(3, &prior_tail, &new_section);

        let mut reasm = SectionReassembler::default();
        reasm.feed(&payload, true);

        let out = reasm
            .pop_section()
            .expect("section after pointer_field skip should pop");
        assert_eq!(out.as_ref(), &new_section[..]);
    }

    #[test]
    fn reassembler_ignores_continuation_before_pusi() {
        // Feed a non-PUSI payload first (no prior PUSI seen).
        // SectionReassembler should discard it and stay empty.
        let pkt = make_packet(0x00, 0x00, PAYLOAD_FLAG, &[0xAA, 0xBB, 0xCC]);

        let mut reasm = SectionReassembler::default();
        reasm.feed(&pkt[4..], false); // no PUSI

        assert!(
            reasm.pop_section().is_none(),
            "no section should appear without prior PUSI"
        );
        assert!(
            reasm.pop_section().is_none(),
            "second pop should also be none"
        );
    }

    /// A PUSI packet with an empty payload (adaptation field ate the body)
    /// is malformed but must not panic — it drops sync.
    #[test]
    fn reassembler_empty_pusi_payload_does_not_panic() {
        let mut reasm = SectionReassembler::default();
        reasm.feed(&[], true);
        assert!(reasm.pop_section().is_none());
        // Recovers on the next clean PUSI.
        let payload = vec![0x00u8, 0x72, 0x70, 0x01, 0x00];
        reasm.feed(&payload, true);
        assert!(reasm.pop_section().is_some());
    }

    /// A maximal short-form private section (section_length 0xFFF, total
    /// 4098 bytes) reassembles — the ceiling is 12-bit length + 3-byte
    /// header, not 4096.
    #[test]
    fn reassembler_accepts_maximal_private_section() {
        let mut section = vec![0x80u8, 0x7F, 0xFF]; // user-private tid, SSI=0, len 0xFFF
        section.resize(3 + 0xFFF, 0xAB);

        let mut reasm = SectionReassembler::default();
        // First TS payload: pointer_field 0 then the section start.
        let mut first = vec![0x00];
        first.extend_from_slice(&section[..183]);
        reasm.feed(&first, true);
        for chunk in section[183..].chunks(184) {
            reasm.feed(chunk, false);
        }
        let out = reasm.pop_section().expect("4098-byte section should pop");
        assert_eq!(out.len(), 4098);
        assert_eq!(out.as_ref(), &section[..]);
    }

    /// Issue #148: a near-maximal section whose final continuation packet
    /// carries the section tail followed by `0xFF` **stuffing** must still
    /// complete. The old overflow guard counted the trailing stuffing toward
    /// `MAX_SECTION_SIZE` and dropped the section.
    #[test]
    fn reassembler_completes_large_section_with_trailing_stuffing() {
        let body = vec![0x5Au8; 4096 - 3];
        let section = build_section(0x50, &body); // 4096 bytes total
        assert_eq!(section.len(), 4096);

        let mut reasm = SectionReassembler::default();
        // First payload (PUSI): pointer_field 0 + first 183 section bytes.
        let mut first = vec![0x00u8];
        first.extend_from_slice(&section[..183]);
        reasm.feed(&first, true);

        // Continuation payloads of a full 184 bytes each; the final one is
        // padded with 0xFF stuffing to a complete 184-byte payload, exactly as
        // a real TS packet would carry it.
        let mut pos = 183usize;
        while pos < section.len() {
            let take = (section.len() - pos).min(184);
            let mut payload = section[pos..pos + take].to_vec();
            if take < 184 {
                payload.resize(184, 0xFF); // stuffing
            }
            reasm.feed(&payload, false);
            pos += take;
        }

        let out = reasm
            .pop_section()
            .expect("4096-byte section must complete despite trailing stuffing (#148)");
        assert_eq!(out.len(), 4096);
        assert_eq!(out.as_ref(), &section[..]);
        assert!(reasm.is_empty(), "stuffing tail must be discarded");
    }

    // ── adaptation field / PCR (ISO/IEC 13818-1 §2.4.3.4–2.4.3.5) ──

    #[test]
    fn pcr_as_27mhz_known_value() {
        assert_eq!(
            Pcr {
                base: 10_000,
                extension: 0
            }
            .as_27mhz(),
            3_000_000
        );
        // base*300 + extension: 1*300 + 100 = 400.
        assert_eq!(
            Pcr {
                base: 1,
                extension: 100
            }
            .as_27mhz(),
            400
        );
    }

    #[test]
    fn pcr_decode_from_bytes() {
        // 6-byte PCR encoding base=10000, extension=0 (reserved bits set).
        let af = [0x10u8, 0x00, 0x00, 0x13, 0x88, 0x7E, 0x00];
        let pcr = Pcr::parse(&af, 1).expect("6 bytes present");
        assert_eq!(
            pcr,
            Pcr {
                base: 10_000,
                extension: 0
            }
        );
        assert_eq!(pcr.as_27mhz(), 3_000_000);
    }

    #[test]
    fn adaptation_field_flags_and_pcr() {
        let mut raw = [0xAAu8; TS_PACKET_SIZE];
        raw[0] = TS_SYNC_BYTE;
        raw[1] = 0x01; // pid 0x0100
        raw[2] = 0x00;
        raw[3] = ADAPTATION_FLAG | PAYLOAD_FLAG;
        raw[4] = 7; // adaptation_field_length: 1 flags + 6 PCR
        raw[5] = AF_DISCONTINUITY | AF_PCR_FLAG;
        raw[6..12].copy_from_slice(&[0x00, 0x00, 0x13, 0x88, 0x7E, 0x00]);
        // raw[12..] stays 0xAA = payload.

        let pkt = TsPacket::parse(&raw).expect("valid packet");
        let af = pkt
            .adaptation_field()
            .expect("has adaptation field")
            .expect("adaptation field parses");
        assert!(af.discontinuity_indicator);
        assert!(!af.random_access_indicator);
        assert_eq!(
            af.pcr,
            Some(Pcr {
                base: 10_000,
                extension: 0
            })
        );
        assert_eq!(af.pcr.unwrap().as_27mhz(), 3_000_000);
        assert!(af.opcr.is_none());
        assert!(af.splice_countdown.is_none());
        // Payload begins right after the adaptation field (cursor 4+1+7=12).
        let payload = pkt.payload.expect("payload present");
        assert_eq!(payload.len(), TS_PACKET_SIZE - 12);
        assert_eq!(payload[0], 0xAA);
    }

    #[test]
    fn no_adaptation_returns_none() {
        let mut raw = [0x00u8; TS_PACKET_SIZE];
        raw[0] = TS_SYNC_BYTE;
        raw[1] = 0x01;
        raw[3] = PAYLOAD_FLAG; // payload only
        let pkt = TsPacket::parse(&raw).expect("valid");
        assert!(pkt.adaptation_field().is_none());
        assert!(pkt.adaptation.is_none());
    }

    #[test]
    fn adaptation_field_splice_countdown_negative() {
        let mut raw = [0xAAu8; TS_PACKET_SIZE];
        raw[0] = TS_SYNC_BYTE;
        raw[1] = 0x01;
        raw[2] = 0x00;
        raw[3] = ADAPTATION_FLAG | PAYLOAD_FLAG;
        raw[4] = 2; // 1 flags + 1 splice_countdown
        raw[5] = AF_SPLICING_FLAG;
        raw[6] = 0xFB; // -5 as i8
        let pkt = TsPacket::parse(&raw).expect("valid");
        let af = pkt.adaptation_field().unwrap().unwrap();
        assert_eq!(af.splice_countdown, Some(-5));
        assert!(af.pcr.is_none());
    }
}
