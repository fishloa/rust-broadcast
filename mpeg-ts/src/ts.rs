//! MPEG-TS packet parser and section reassembler — ITU-T H.222.0 §2.4 (= ISO/IEC 13818-1).

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

/// 2-bit `transport_scrambling_control` field — ITU-T H.222.0 (08/2023) Table 2-4
/// (defines only `00` = not scrambled); DVB assigns `01`/`10`/`11` in ETSI TS 100 289
/// V1.1.1 §5.1 Table 1 (reserved, even CW, odd CW).
///
/// MPEG-2 leaves `01`/`10`/`11` as user-defined; DVB's common-scrambling convention
/// assigns `10` = even control word, `11` = odd control word, `01` = reserved for future
/// DVB use. The field lives in the TS header and is never applied to the header itself —
/// only to the packet payload.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ScramblingControl {
    /// `00` — not scrambled. The only MPEG-2-defined value (H.222.0 Table 2-4).
    NotScrambled,
    /// `01` — reserved for future DVB use (ETSI TS 100 289 V1.1.1 §5.1 Table 1).
    Reserved,
    /// `10` — TS packet payload scrambled with the **even** control word
    /// (DVB common scrambling, ETSI TS 100 289 V1.1.1 §5.1 Table 1).
    EvenKey,
    /// `11` — TS packet payload scrambled with the **odd** control word
    /// (DVB common scrambling, ETSI TS 100 289 V1.1.1 §5.1 Table 1).
    OddKey,
}

impl ScramblingControl {
    /// Decode from the 2-bit `transport_scrambling_control` value (masked to `[1:0]`).
    pub fn from_bits(bits: u8) -> Self {
        match bits & 0b11 {
            0b00 => Self::NotScrambled,
            0b01 => Self::Reserved,
            0b10 => Self::EvenKey,
            0b11 => Self::OddKey,
            _ => unreachable!(),
        }
    }

    /// Encode as the 2-bit `transport_scrambling_control` field value (`[1:0]`).
    ///
    /// The returned byte is in the range `0x00`–`0x03`; the caller shifts it
    /// into position within the TS header byte 3 (H.222.0 Table 2-4).
    pub fn to_bits(self) -> u8 {
        match self {
            Self::NotScrambled => 0b00,
            Self::Reserved => 0b01,
            Self::EvenKey => 0b10,
            Self::OddKey => 0b11,
        }
    }

    /// Short label for this value, per the #204 convention.
    pub fn name(&self) -> &'static str {
        match self {
            Self::NotScrambled => "not_scrambled",
            Self::Reserved => "reserved",
            Self::EvenKey => "even_key",
            Self::OddKey => "odd_key",
        }
    }
}

broadcast_common::impl_spec_display!(ScramblingControl);

/// 2-bit `adaptation_field_control` field — ITU-T H.222.0 (08/2023) Table 2-5.
///
/// Decoders shall discard packets with value `00` (`Reserved`). Null packets use `01`
/// (`PayloadOnly`). The two flags `has_adaptation`/`has_payload` on [`TsHeader`] carry
/// the decoded booleans; this enum provides the typed composite view.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum AdaptationFieldControl {
    /// `00` — reserved for future use; decoders shall discard (H.222.0 Table 2-5).
    Reserved,
    /// `01` — no adaptation_field, payload only (H.222.0 Table 2-5).
    PayloadOnly,
    /// `10` — adaptation_field only, no payload (H.222.0 Table 2-5).
    AdaptationOnly,
    /// `11` — adaptation_field followed by payload (H.222.0 Table 2-5).
    AdaptationAndPayload,
}

impl AdaptationFieldControl {
    /// Derive from the two decoded boolean flags stored on [`TsHeader`].
    pub fn from_flags(has_adaptation: bool, has_payload: bool) -> Self {
        match (has_adaptation, has_payload) {
            (false, false) => Self::Reserved,
            (false, true) => Self::PayloadOnly,
            (true, false) => Self::AdaptationOnly,
            (true, true) => Self::AdaptationAndPayload,
        }
    }

    /// Encode as the 2-bit `adaptation_field_control` field value (`[1:0]`).
    ///
    /// Bit 1 = adaptation present, bit 0 = payload present (H.222.0 Table 2-5).
    /// The returned byte is in the range `0x00`–`0x03`; the caller shifts it
    /// into bits `[5:4]` of TS header byte 3.
    pub fn to_bits(self) -> u8 {
        match self {
            Self::Reserved => 0b00,
            Self::PayloadOnly => 0b01,
            Self::AdaptationOnly => 0b10,
            Self::AdaptationAndPayload => 0b11,
        }
    }

    /// Decode into the `(has_adaptation, has_payload)` flag pair stored on
    /// [`TsHeader`]. Exact inverse of [`from_flags`](Self::from_flags).
    pub fn to_flags(self) -> (bool, bool) {
        match self {
            Self::Reserved => (false, false),
            Self::PayloadOnly => (false, true),
            Self::AdaptationOnly => (true, false),
            Self::AdaptationAndPayload => (true, true),
        }
    }

    /// Short label for this value, per the #204 convention.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::PayloadOnly => "payload_only",
            Self::AdaptationOnly => "adaptation_only",
            Self::AdaptationAndPayload => "adaptation_and_payload",
        }
    }
}

broadcast_common::impl_spec_display!(AdaptationFieldControl);

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

    /// Typed view of the 2-bit `transport_scrambling_control` field.
    ///
    /// See [`ScramblingControl`] for the spec citation (H.222.0 Table 2-4 +
    /// ETSI TS 100 289 §5.1 Table 1).
    pub fn scrambling_control(&self) -> ScramblingControl {
        ScramblingControl::from_bits(self.scrambling)
    }

    /// Typed view of the `adaptation_field_control` 2-bit field, derived from the
    /// `has_adaptation`/`has_payload` flags.
    ///
    /// See [`AdaptationFieldControl`] for the spec citation (H.222.0 Table 2-5).
    pub fn adaptation_field_control(&self) -> AdaptationFieldControl {
        AdaptationFieldControl::from_flags(self.has_adaptation, self.has_payload)
    }
}

impl<'a> broadcast_common::Parse<'a> for TsHeader {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        TsHeader::parse(bytes)
    }
}

impl broadcast_common::Serialize for TsHeader {
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
    pub fn adaptation_field(&self) -> Option<crate::Result<AdaptationField<'a>>> {
        self.adaptation.map(AdaptationField::parse)
    }
}

// Adaptation-field flag bits, byte 0 (ISO/IEC 13818-1:2007 §2.4.3.4).
pub(crate) const AF_DISCONTINUITY: u8 = 0x80;
pub(crate) const AF_RANDOM_ACCESS: u8 = 0x40;
pub(crate) const AF_ES_PRIORITY: u8 = 0x20;
/// PCR present flag (bit 4 of adaptation field flags byte — §2.4.3.4).
pub const AF_PCR_FLAG: u8 = 0x10;
pub(crate) const AF_OPCR_FLAG: u8 = 0x08;
pub(crate) const AF_SPLICING_FLAG: u8 = 0x04;
pub(crate) const AF_TRANSPORT_PRIVATE_DATA_FLAG: u8 = 0x02;
pub(crate) const AF_EXTENSION_FLAG: u8 = 0x01;
/// Adaptation-field stuffing byte (ISO/IEC 13818-1:2007 §2.4.3.4 — `0xFF`).
pub(crate) const AF_STUFFING_BYTE: u8 = 0xFF;
/// Encoded PCR / OPCR field width: 33-bit base + 6 reserved + 9-bit extension.
pub(crate) const PCR_FIELD_LEN: usize = 6;

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
    ///
    /// ISO/IEC 13818-1:2007 §2.4.3.5: PCR = `PCR_base * 300 + PCR_ext`.
    #[must_use]
    pub fn as_27mhz(self) -> u64 {
        self.base * 300 + self.extension as u64
    }

    /// Construct a [`Pcr`] from an absolute 27 MHz clock value.
    ///
    /// Decomposes `ticks` into `base = ticks / 300` and
    /// `extension = ticks % 300`, clamping each to its wire width
    /// (33-bit base, 9-bit extension) — ISO/IEC 13818-1:2007 §2.4.3.5.
    ///
    /// Round-trips with [`as_27mhz`](Self::as_27mhz):
    /// `Pcr::from_27mhz(p.as_27mhz()) == p` for any valid `Pcr`.
    #[must_use]
    pub fn from_27mhz(ticks: u64) -> Self {
        // 33-bit base mask: (1 << 33) - 1 = 0x1_FFFF_FFFF
        const BASE_MASK: u64 = 0x1_FFFF_FFFF;
        // 9-bit extension mask.
        const EXT_MASK: u16 = 0x1FF;
        Self {
            base: (ticks / 300) & BASE_MASK,
            extension: ((ticks % 300) as u16) & EXT_MASK,
        }
    }

    /// Encode as the exact 6-byte PCR/OPCR field used in the adaptation field
    /// (ISO/IEC 13818-1:2007 §2.4.3.5).
    ///
    /// Wire layout (big-endian, 48 bits):
    /// `[base[32:25]][base[24:17]][base[16:9]][base[8:1]][base[0] | 6×reserved(1) | ext[8]][ext[7:0]]`
    ///
    /// The 6 reserved bits are set to `1` as per the spec's "reserved" convention.
    /// Exact inverse of the private `parse` function.
    #[must_use]
    pub fn to_field_bytes(self) -> [u8; PCR_FIELD_LEN] {
        let b = self.base;
        let e = self.extension as u64;
        [
            ((b >> 25) & 0xFF) as u8,
            ((b >> 17) & 0xFF) as u8,
            ((b >> 9) & 0xFF) as u8,
            ((b >> 1) & 0xFF) as u8,
            // byte 4: base[0] in bit 7, bits 6-1 = reserved (set to 1), ext[8] in bit 0.
            (((b & 0x01) as u8) << 7) | 0x7E | ((e >> 8) as u8 & 0x01),
            (e & 0xFF) as u8,
        ]
    }

    /// Decode the 6-byte PCR/OPCR field starting at `at` within `af`.
    pub(crate) fn parse(af: &[u8], at: usize) -> Result<Self> {
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

// ── Adaptation-field extension sub-structures (ISO/IEC 13818-1 §2.4.3.5) ──────

/// `legal_time_window` field within the adaptation-field extension
/// (ISO/IEC 13818-1:2007 §2.4.3.5, `ltw_flag == 1`).
///
/// Wire layout: `ltw_valid_flag(1) | ltw_offset(15)` = 2 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Ltw {
    /// LTW offset valid flag.
    pub ltw_valid_flag: bool,
    /// 15-bit `ltw_offset` (lower bound of the legal time window).
    pub ltw_offset: u16,
}

/// `seamless_splice` field within the adaptation-field extension
/// (ISO/IEC 13818-1:2007 §2.4.3.5, `seamless_splice_flag == 1`).
///
/// Wire layout: 5 bytes — `splice_type(4) | DTS_next_AU[32:30](3) | marker(1) |
/// DTS_next_AU[29:15](15) | marker(1) | DTS_next_AU[14:0](15) | marker(1)`.
/// The DTS field uses the same marker-bit encoding as PTS/DTS in PES headers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SeamlessSplice {
    /// 4-bit `splice_type`.
    pub splice_type: u8,
    /// 33-bit `DTS_next_AU` (90 kHz decoding time of the next splice unit).
    pub dts_next_au: u64,
}

/// Adaptation-field extension (ISO/IEC 13818-1:2007 §2.4.3.5,
/// `adaptation_field_extension_flag == 1`).
///
/// Contains optional sub-fields gated by `ltw_flag`,
/// `piecewise_rate_flag`, and `seamless_splice_flag`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AdaptationFieldExtension {
    /// LTW (legal time window), if `ltw_flag` is set.
    pub ltw: Option<Ltw>,
    /// 22-bit piecewise rate, if `piecewise_rate_flag` is set.
    pub piecewise_rate: Option<u32>,
    /// Seamless splice info, if `seamless_splice_flag` is set.
    pub seamless_splice: Option<SeamlessSplice>,
}

impl AdaptationFieldExtension {
    /// Parse the adaptation-field extension starting at `data[0]`
    /// (the `adaptation_field_extension_length` byte).
    fn parse(data: &[u8]) -> Result<Self> {
        if data.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "adaptation_field_extension_length",
            });
        }
        let ext_len = data[0] as usize;
        // At least 1 byte (the flags byte) must be present inside the extension.
        if data.len() < 1 + ext_len || ext_len < 1 {
            return Err(Error::BufferTooShort {
                need: 2.max(1 + ext_len),
                have: data.len(),
                what: "adaptation_field_extension body",
            });
        }
        let ext = &data[1..1 + ext_len]; // extension bytes, starts with flags byte
        let flags = ext[0];
        let mut cursor = 1usize;

        let ltw = if flags & 0x80 != 0 {
            if ext.len() < cursor + 2 {
                return Err(Error::BufferTooShort {
                    need: cursor + 2,
                    have: ext.len(),
                    what: "ltw_offset",
                });
            }
            let w0 = ext[cursor];
            let w1 = ext[cursor + 1];
            cursor += 2;
            Some(Ltw {
                ltw_valid_flag: (w0 & 0x80) != 0,
                ltw_offset: (((w0 & 0x7F) as u16) << 8) | (w1 as u16),
            })
        } else {
            None
        };

        let piecewise_rate = if flags & 0x40 != 0 {
            if ext.len() < cursor + 3 {
                return Err(Error::BufferTooShort {
                    need: cursor + 3,
                    have: ext.len(),
                    what: "piecewise_rate",
                });
            }
            let r = (((ext[cursor] & 0x3F) as u32) << 16)
                | ((ext[cursor + 1] as u32) << 8)
                | (ext[cursor + 2] as u32);
            cursor += 3;
            Some(r)
        } else {
            None
        };

        // seamless_splice: 5 bytes with splice_type(4) + DTS_next_AU in PTS-field encoding
        let seamless_splice = if flags & 0x20 != 0 {
            if ext.len() < cursor + 5 {
                return Err(Error::BufferTooShort {
                    need: cursor + 5,
                    have: ext.len(),
                    what: "seamless_splice DTS_next_AU",
                });
            }
            let b = &ext[cursor..cursor + 5];
            let splice_type = (b[0] >> 4) & 0x0F;
            // DTS_next_AU uses the same 5-byte marker-bit encoding as PTS/DTS.
            let hi = u64::from((b[0] >> 1) & 0x07); // [32:30]
            let mid = (u64::from(b[1]) << 7) | u64::from(b[2] >> 1); // [29:15]
            let lo = (u64::from(b[3]) << 7) | u64::from(b[4] >> 1); // [14:0]
            let dts_next_au = (hi << 30) | (mid << 15) | lo;
            cursor += 5;
            Some(SeamlessSplice {
                splice_type,
                dts_next_au,
            })
        } else {
            None
        };
        let _ = cursor; // remaining extension bytes (reserved) are skipped

        Ok(AdaptationFieldExtension {
            ltw,
            piecewise_rate,
            seamless_splice,
        })
    }

    /// Number of bytes written by [`serialize_into`](Self::serialize_into),
    /// **including** the leading `adaptation_field_extension_length` byte.
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        let body = 1 // flags byte
            + self.ltw.map_or(0, |_| 2)
            + self.piecewise_rate.map_or(0, |_| 3)
            + self.seamless_splice.map_or(0, |_| 5);
        1 + body // + length byte itself
    }

    /// Serialize into `buf` (includes the `adaptation_field_extension_length` byte).
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let body_len = need - 1;
        buf[0] = body_len as u8;

        let mut flags = 0u8;
        if self.ltw.is_some() {
            flags |= 0x80;
        }
        if self.piecewise_rate.is_some() {
            flags |= 0x40;
        }
        if self.seamless_splice.is_some() {
            flags |= 0x20;
        }
        buf[1] = flags;
        let mut cursor = 2usize;

        if let Some(ltw) = self.ltw {
            let ltw_valid = if ltw.ltw_valid_flag { 0x80u8 } else { 0x00 };
            buf[cursor] = ltw_valid | ((ltw.ltw_offset >> 8) as u8 & 0x7F);
            buf[cursor + 1] = (ltw.ltw_offset & 0xFF) as u8;
            cursor += 2;
        }
        if let Some(rate) = self.piecewise_rate {
            // 2 reserved bits (set to 1) | 22-bit rate (ISO/IEC 13818-1 §2.4.3.5).
            buf[cursor] = 0xC0 | ((rate >> 16) as u8 & 0x3F);
            buf[cursor + 1] = (rate >> 8) as u8;
            buf[cursor + 2] = rate as u8;
            cursor += 3;
        }
        if let Some(ss) = self.seamless_splice {
            let ts = ss.dts_next_au & 0x1_FFFF_FFFF;
            let st = ss.splice_type & 0x0F;
            // byte 0: splice_type(4) | DTS[32:30](3) | marker(1)
            buf[cursor] = (st << 4) | ((((ts >> 30) & 0x07) as u8) << 1) | 0x01;
            buf[cursor + 1] = ((ts >> 22) & 0xFF) as u8;
            buf[cursor + 2] = ((((ts >> 15) & 0x7F) as u8) << 1) | 0x01;
            buf[cursor + 3] = ((ts >> 7) & 0xFF) as u8;
            buf[cursor + 4] = (((ts & 0x7F) as u8) << 1) | 0x01;
            cursor += 5;
        }
        Ok(cursor)
    }
}

/// Decoded adaptation field — full §2.4.3.4 layout including transport-private
/// data and the adaptation-field extension sub-structure.
///
/// The `transport_private_data` slice borrows from the original packet buffer
/// (it is genuinely opaque caller-defined bytes per the spec — `&[u8]` is the
/// correct public type). All other fields are fully typed.
///
/// ISO/IEC 13818-1:2007 §2.4.3.4.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AdaptationField<'a> {
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
    /// Opaque transport-private data (caller-defined; `&[u8]` is spec-correct here).
    ///
    /// Present iff `transport_private_data_flag` is set in the flags byte
    /// (ISO/IEC 13818-1:2007 §2.4.3.4).
    pub transport_private_data: Option<&'a [u8]>,
    /// Typed adaptation-field extension sub-structure, if the flag is set.
    pub extension: Option<AdaptationFieldExtension>,
    /// Number of trailing `0xFF` stuffing bytes that pad the adaptation-field
    /// body out to its declared `adaptation_field_length` (ISO/IEC 13818-1:2007
    /// §2.4.3.4 — "stuffing_byte: fixed 8-bit value `0xFF`").
    ///
    /// Real encoders pad the adaptation field so the packet payload begins at a
    /// fixed offset; these bytes are part of the wire image. Capturing the count
    /// lets [`serialize_into`](Self::serialize_into) reproduce the packet
    /// byte-for-byte. Set to `0` when constructing an adaptation field with no
    /// stuffing.
    pub stuffing_len: usize,
}

impl<'a> AdaptationField<'a> {
    /// Parse the adaptation-field bytes (those following the length byte).
    ///
    /// `af` must be exactly the adaptation-field body bytes: the slice starts
    /// at the **flags byte** (byte 0 of `af`), i.e. the bytes AFTER the
    /// `adaptation_field_length` byte itself. This matches how
    /// `TsPacket::parse` captures and hands them off.
    pub(crate) fn parse(af: &'a [u8]) -> Result<Self> {
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
            cursor += 1;
            Some(b as i8)
        } else {
            None
        };

        // transport_private_data (ISO/IEC 13818-1 §2.4.3.4):
        // transport_private_data_length(8) + transport_private_data_byte * N
        let transport_private_data = if flags & AF_TRANSPORT_PRIVATE_DATA_FLAG != 0 {
            let tpd_len = *af.get(cursor).ok_or(Error::BufferTooShort {
                need: cursor + 1,
                have: af.len(),
                what: "transport_private_data_length",
            })? as usize;
            cursor += 1;
            let end = cursor + tpd_len;
            let slice = af.get(cursor..end).ok_or(Error::BufferTooShort {
                need: end,
                have: af.len(),
                what: "transport_private_data",
            })?;
            cursor = end;
            Some(slice)
        } else {
            None
        };

        // adaptation_field_extension (ISO/IEC 13818-1 §2.4.3.5):
        let extension = if flags & AF_EXTENSION_FLAG != 0 {
            let ext_data = af.get(cursor..).ok_or(Error::BufferTooShort {
                need: cursor + 1,
                have: af.len(),
                what: "adaptation_field_extension",
            })?;
            let ext = AdaptationFieldExtension::parse(ext_data)?;
            // advance cursor past the extension (ext_data[0] = length byte)
            if !ext_data.is_empty() {
                let _ext_len = ext_data[0] as usize;
                cursor += 1 + _ext_len;
            }
            Some(ext)
        } else {
            None
        };

        // Any bytes after the last present field, up to `adaptation_field_length`
        // (= `af.len()`), are `0xFF` stuffing (ISO/IEC 13818-1:2007 §2.4.3.4).
        // Record the count so serialization reproduces the body byte-for-byte.
        let stuffing_len = af.len().saturating_sub(cursor);

        Ok(AdaptationField {
            discontinuity_indicator: flags & AF_DISCONTINUITY != 0,
            random_access_indicator: flags & AF_RANDOM_ACCESS != 0,
            elementary_stream_priority_indicator: flags & AF_ES_PRIORITY != 0,
            pcr,
            opcr,
            splice_countdown,
            transport_private_data,
            extension,
            stuffing_len,
        })
    }

    /// Number of bytes written by [`serialize_into`](Self::serialize_into).
    ///
    /// This is the body length **excluding** the leading `adaptation_field_length`
    /// byte — it is the value carried in that length byte itself
    /// (ISO/IEC 13818-1:2007 §2.4.3.4).
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        let mut n = 1usize; // flags byte
        if self.pcr.is_some() {
            n += PCR_FIELD_LEN;
        }
        if self.opcr.is_some() {
            n += PCR_FIELD_LEN;
        }
        if self.splice_countdown.is_some() {
            n += 1;
        }
        if let Some(tpd) = self.transport_private_data {
            n += 1 + tpd.len(); // length byte + data
        }
        if let Some(ref ext) = self.extension {
            n += ext.serialized_len();
        }
        n += self.stuffing_len;
        n
    }

    /// Serialize the adaptation field into `buf`.
    ///
    /// Writes the **body** bytes — the flags byte plus optional fields in the
    /// order specified by ISO/IEC 13818-1:2007 §2.4.3.4. The
    /// `adaptation_field_length` byte itself is **not** written here; the caller
    /// must prepend it (it equals `serialized_len()`).
    ///
    /// Returns the number of bytes written on success, or
    /// [`Error::OutputBufferTooSmall`] if `buf` is shorter than
    /// `serialized_len()`.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }

        // Byte 0: flags (ISO/IEC 13818-1:2007 §2.4.3.4).
        let mut flags = 0u8;
        if self.discontinuity_indicator {
            flags |= AF_DISCONTINUITY;
        }
        if self.random_access_indicator {
            flags |= AF_RANDOM_ACCESS;
        }
        if self.elementary_stream_priority_indicator {
            flags |= AF_ES_PRIORITY;
        }
        if self.pcr.is_some() {
            flags |= AF_PCR_FLAG;
        }
        if self.opcr.is_some() {
            flags |= AF_OPCR_FLAG;
        }
        if self.splice_countdown.is_some() {
            flags |= AF_SPLICING_FLAG;
        }
        if self.transport_private_data.is_some() {
            flags |= AF_TRANSPORT_PRIVATE_DATA_FLAG;
        }
        if self.extension.is_some() {
            flags |= AF_EXTENSION_FLAG;
        }
        buf[0] = flags;

        let mut cursor = 1usize;
        if let Some(pcr) = self.pcr {
            buf[cursor..cursor + PCR_FIELD_LEN].copy_from_slice(&pcr.to_field_bytes());
            cursor += PCR_FIELD_LEN;
        }
        if let Some(opcr) = self.opcr {
            buf[cursor..cursor + PCR_FIELD_LEN].copy_from_slice(&opcr.to_field_bytes());
            cursor += PCR_FIELD_LEN;
        }
        if let Some(sc) = self.splice_countdown {
            buf[cursor] = sc as u8;
            cursor += 1;
        }
        if let Some(tpd) = self.transport_private_data {
            buf[cursor] = tpd.len() as u8;
            cursor += 1;
            buf[cursor..cursor + tpd.len()].copy_from_slice(tpd);
            cursor += tpd.len();
        }
        if let Some(ref ext) = self.extension {
            let written = ext.serialize_into(&mut buf[cursor..])?;
            cursor += written;
        }

        // Trailing `0xFF` stuffing (ISO/IEC 13818-1:2007 §2.4.3.4), reproducing
        // the padding the encoder used to fill `adaptation_field_length`.
        for b in buf[cursor..cursor + self.stuffing_len].iter_mut() {
            *b = AF_STUFFING_BYTE;
        }
        cursor += self.stuffing_len;

        Ok(cursor)
    }
}

/// Reassembles PSI/SI sections from TS packets on a single PID.
///
/// Feed each TS packet's payload with `feed`. Complete sections are
/// appended to an internal queue; drain them with `pop_section`.
#[derive(Default)]
pub struct SectionReassembler {
    buf: bytes::BytesMut,
    ready: alloc::collections::VecDeque<bytes::Bytes>,
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

/// Iterate over all valid TS packets in a byte buffer.
///
/// Slices `buf` into 188-byte chunks (using [`slice::chunks_exact`]) and yields
/// each chunk for which [`TsPacket::parse`] succeeds. Chunks with a bad sync byte
/// (`!= 0x47`) or insufficient length are silently skipped — use
/// [`crate::resync::TsResync`] for byte-stream resynchronisation before calling
/// this when byte alignment is not guaranteed.
///
/// # Example
///
/// ```no_run
/// # use mpeg_ts::ts::iter_packets;
/// # let data: &[u8] = &[];
/// for pkt in iter_packets(data) {
///     println!("PID: 0x{:04X}", pkt.header.pid);
/// }
/// ```
pub fn iter_packets(buf: &[u8]) -> impl Iterator<Item = TsPacket<'_>> {
    buf.chunks_exact(TS_PACKET_SIZE)
        .filter_map(|chunk| TsPacket::parse(chunk).ok())
}

/// Extract the payload bytes from a raw 188-byte TS packet slice.
///
/// Returns `None` when:
/// - `pkt` is fewer than 4 bytes,
/// - `adaptation_field_control` is `00` (reserved) or `10` (adaptation only), or
/// - the adaptation field length would place the payload start past the packet end.
///
/// No sync-byte check is performed — the caller is responsible for ensuring the
/// slice is properly aligned. Spec: ITU-T H.222.0 (08/2023) §2.4.3.3 Table 2-5.
pub fn extract_ts_payload(pkt: &[u8]) -> Option<&[u8]> {
    if pkt.len() < 4 {
        return None;
    }
    let afc = (pkt[3] >> 4) & 0x3;
    match afc {
        0x1 => {
            // payload only: payload starts at byte 4
            if pkt.len() > 4 {
                Some(&pkt[4..])
            } else {
                None
            }
        }
        0x3 => {
            // adaptation field + payload
            if pkt.len() < 5 {
                return None;
            }
            let af_len = pkt[4] as usize;
            let start = 5 + af_len;
            if start < pkt.len() {
                Some(&pkt[start..])
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;
    use alloc::vec::Vec;

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
        let got: Vec<_> = core::iter::from_fn(|| reasm.pop_section()).collect();
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

        let got: Vec<_> = core::iter::from_fn(|| reasm.pop_section()).collect();
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
        let got: Vec<_> = core::iter::from_fn(|| reasm.pop_section()).collect();
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

    // ── ScramblingControl / AdaptationFieldControl enums ──

    #[test]
    fn scrambling_control_all_values() {
        assert_eq!(
            ScramblingControl::from_bits(0b00),
            ScramblingControl::NotScrambled
        );
        assert_eq!(
            ScramblingControl::from_bits(0b01),
            ScramblingControl::Reserved
        );
        assert_eq!(
            ScramblingControl::from_bits(0b10),
            ScramblingControl::EvenKey
        );
        assert_eq!(
            ScramblingControl::from_bits(0b11),
            ScramblingControl::OddKey
        );
        // name() labels
        assert_eq!(ScramblingControl::NotScrambled.name(), "not_scrambled");
        assert_eq!(ScramblingControl::Reserved.name(), "reserved");
        assert_eq!(ScramblingControl::EvenKey.name(), "even_key");
        assert_eq!(ScramblingControl::OddKey.name(), "odd_key");
        // Display delegates to name()
        assert_eq!(ScramblingControl::NotScrambled.to_string(), "not_scrambled");
        assert_eq!(ScramblingControl::OddKey.to_string(), "odd_key");
        // Masking: only low 2 bits matter
        assert_eq!(
            ScramblingControl::from_bits(0xFF),
            ScramblingControl::OddKey
        );
    }

    #[test]
    fn adaptation_field_control_all_values() {
        assert_eq!(
            AdaptationFieldControl::from_flags(false, false),
            AdaptationFieldControl::Reserved
        );
        assert_eq!(
            AdaptationFieldControl::from_flags(false, true),
            AdaptationFieldControl::PayloadOnly
        );
        assert_eq!(
            AdaptationFieldControl::from_flags(true, false),
            AdaptationFieldControl::AdaptationOnly
        );
        assert_eq!(
            AdaptationFieldControl::from_flags(true, true),
            AdaptationFieldControl::AdaptationAndPayload
        );
        // name()
        assert_eq!(AdaptationFieldControl::Reserved.name(), "reserved");
        assert_eq!(AdaptationFieldControl::PayloadOnly.name(), "payload_only");
        assert_eq!(
            AdaptationFieldControl::AdaptationOnly.name(),
            "adaptation_only"
        );
        assert_eq!(
            AdaptationFieldControl::AdaptationAndPayload.name(),
            "adaptation_and_payload"
        );
        // Display
        assert_eq!(
            AdaptationFieldControl::PayloadOnly.to_string(),
            "payload_only"
        );
    }

    #[test]
    fn ts_header_scrambling_control_accessor() {
        let hdr = TsHeader {
            tei: false,
            pusi: false,
            pid: 0x0100,
            scrambling: 0b10,
            has_adaptation: false,
            has_payload: true,
            continuity_counter: 0,
        };
        assert_eq!(hdr.scrambling_control(), ScramblingControl::EvenKey);
    }

    #[test]
    fn ts_header_adaptation_field_control_accessor() {
        let hdr_payload_only = TsHeader {
            tei: false,
            pusi: false,
            pid: 0x0100,
            scrambling: 0,
            has_adaptation: false,
            has_payload: true,
            continuity_counter: 0,
        };
        assert_eq!(
            hdr_payload_only.adaptation_field_control(),
            AdaptationFieldControl::PayloadOnly
        );

        let hdr_both = TsHeader {
            tei: false,
            pusi: false,
            pid: 0x0100,
            scrambling: 0,
            has_adaptation: true,
            has_payload: true,
            continuity_counter: 0,
        };
        assert_eq!(
            hdr_both.adaptation_field_control(),
            AdaptationFieldControl::AdaptationAndPayload
        );
    }

    // ── iter_packets / extract_ts_payload helpers ──

    #[test]
    fn iter_packets_yields_valid_and_skips_bad_sync() {
        // Two valid packets back-to-back, then one bad-sync packet.
        let pkt1 = make_packet(0x00, 0x00, PAYLOAD_FLAG, &[0xAA; 10]);
        let pkt2 = make_packet(0x40, 0x64, PAYLOAD_FLAG, &[0xBB; 10]);
        let mut bad = [0u8; TS_PACKET_SIZE];
        bad[0] = 0x00; // bad sync byte

        let mut buf = Vec::new();
        buf.extend_from_slice(&pkt1);
        buf.extend_from_slice(&pkt2);
        buf.extend_from_slice(&bad);

        let pkts: Vec<_> = super::iter_packets(&buf).collect();
        assert_eq!(pkts.len(), 2, "bad sync packet must be skipped");
        assert_eq!(pkts[0].header.pid, 0x0000);
        assert_eq!(pkts[1].header.pid, 0x0064);
    }

    #[test]
    fn extract_ts_payload_payload_only() {
        let pkt = make_packet(0x00, 0x00, PAYLOAD_FLAG, &[0xABu8; 10]);
        let p = super::extract_ts_payload(&pkt).expect("payload present");
        assert_eq!(p[0], 0xAB);
        assert_eq!(p.len(), TS_PACKET_SIZE - 4);
    }

    #[test]
    fn extract_ts_payload_adaptation_only_returns_none() {
        let pkt = make_packet(0x00, 0x00, ADAPTATION_FLAG, &[]);
        assert!(super::extract_ts_payload(&pkt).is_none());
    }

    // ── Pcr write-side ──────────────────────────────────────────────────────

    /// `from_27mhz(v).as_27mhz() == v` for representative values
    /// (ISO/IEC 13818-1:2007 §2.4.3.5).
    #[test]
    fn pcr_from_27mhz_round_trips() {
        for &ticks in &[0u64, 1, 300, 27_000_000, u64::from(u32::MAX), 8_589_934_591] {
            let pcr = Pcr::from_27mhz(ticks);
            assert_eq!(pcr.as_27mhz(), ticks, "ticks={ticks}");
        }
    }

    /// `to_field_bytes` → `parse` → same `Pcr` (field-bytes round-trip).
    #[test]
    fn pcr_to_field_bytes_round_trips_parse() {
        let cases = [
            Pcr {
                base: 0,
                extension: 0,
            },
            Pcr {
                base: 10_000,
                extension: 0,
            },
            Pcr {
                base: 1,
                extension: 100,
            },
            Pcr {
                base: 0x1_FFFF_FFFF,
                extension: 0x1FF,
            },
        ];
        for pcr in cases {
            let bytes = pcr.to_field_bytes();
            // Prefix the 6 bytes with a dummy flags byte so the offset is 1,
            // matching the parse() calling convention inside AdaptationField::parse.
            let mut af = [0u8; 7];
            af[1..7].copy_from_slice(&bytes);
            let decoded = Pcr::parse(&af, 1).expect("parse round-trip");
            assert_eq!(decoded, pcr, "round-trip failed for {pcr:?}");
        }
    }

    /// Known vector from ts.rs existing test — base=10000, extension=0 produces
    /// the 6-byte encoding `[0x00, 0x00, 0x13, 0x88, 0x7E, 0x00]`.
    #[test]
    fn pcr_to_field_bytes_known_vector() {
        let pcr = Pcr {
            base: 10_000,
            extension: 0,
        };
        let bytes = pcr.to_field_bytes();
        assert_eq!(bytes, [0x00, 0x00, 0x13, 0x88, 0x7E, 0x00]);
    }

    // ── ScramblingControl to_bits ───────────────────────────────────────────

    #[test]
    fn scrambling_control_to_bits_inverse_of_from_bits() {
        for bits in 0u8..=3 {
            let sc = ScramblingControl::from_bits(bits);
            assert_eq!(sc.to_bits(), bits, "to_bits() != from_bits() for {bits}");
        }
    }

    // ── AdaptationFieldControl to_bits / to_flags ──────────────────────────

    #[test]
    fn adaptation_field_control_to_bits_inverse_of_from_flags() {
        let cases = [
            (false, false, 0b00u8),
            (false, true, 0b01),
            (true, false, 0b10),
            (true, true, 0b11),
        ];
        for (has_af, has_pl, expected_bits) in cases {
            let afc = AdaptationFieldControl::from_flags(has_af, has_pl);
            assert_eq!(afc.to_bits(), expected_bits);
            assert_eq!(afc.to_flags(), (has_af, has_pl));
        }
    }

    // ── AdaptationField serialize_into ─────────────────────────────────────

    /// Build an adaptation field with PCR, serialize → parse → verify equal.
    #[test]
    fn adaptation_field_serialize_round_trip_with_pcr() {
        let original = AdaptationField {
            discontinuity_indicator: true,
            random_access_indicator: false,
            elementary_stream_priority_indicator: false,
            pcr: Some(Pcr {
                base: 10_000,
                extension: 0,
            }),
            opcr: None,
            splice_countdown: None,
            transport_private_data: None,
            extension: None,
            stuffing_len: 0,
        };
        let len = original.serialized_len();
        assert_eq!(len, 7); // 1 flags + 6 PCR
        let mut buf = vec![0u8; len];
        let written = original.serialize_into(&mut buf).expect("serialize");
        assert_eq!(written, len);
        let decoded = AdaptationField::parse(&buf).expect("parse round-trip");
        assert_eq!(decoded, original);
    }

    /// Known-bytes test: flags=0x30 (discontinuity + PCR), known PCR vector.
    #[test]
    fn adaptation_field_serialize_produces_known_bytes() {
        // Matches the packet in ts.rs `adaptation_field_flags_and_pcr` test:
        // raw[5] = AF_DISCONTINUITY | AF_PCR_FLAG = 0x90
        // raw[6..12] = [0x00, 0x00, 0x13, 0x88, 0x7E, 0x00]
        let af = AdaptationField {
            discontinuity_indicator: true,
            random_access_indicator: false,
            elementary_stream_priority_indicator: false,
            pcr: Some(Pcr {
                base: 10_000,
                extension: 0,
            }),
            opcr: None,
            splice_countdown: None,
            transport_private_data: None,
            extension: None,
            stuffing_len: 0,
        };
        let mut buf = [0u8; 7];
        af.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[0], AF_DISCONTINUITY | AF_PCR_FLAG);
        assert_eq!(&buf[1..7], &[0x00, 0x00, 0x13, 0x88, 0x7E, 0x00]);
    }

    /// Serialize → parse round-trip for AdaptationField with OPCR + splice_countdown.
    #[test]
    fn adaptation_field_serialize_round_trip_opcr_and_splice() {
        let original = AdaptationField {
            discontinuity_indicator: false,
            random_access_indicator: true,
            elementary_stream_priority_indicator: true,
            pcr: Some(Pcr {
                base: 1,
                extension: 100,
            }),
            opcr: Some(Pcr {
                base: 999,
                extension: 5,
            }),
            splice_countdown: Some(-3),
            transport_private_data: None,
            extension: None,
            stuffing_len: 0,
        };
        let len = original.serialized_len();
        assert_eq!(len, 1 + 6 + 6 + 1); // flags + PCR + OPCR + splice
        let mut buf = vec![0u8; len];
        original.serialize_into(&mut buf).unwrap();
        let decoded = AdaptationField::parse(&buf).expect("parse");
        assert_eq!(decoded, original);
    }

    /// Flags-only AdaptationField (no PCR/OPCR/splice).
    #[test]
    fn adaptation_field_serialize_flags_only() {
        let af = AdaptationField {
            discontinuity_indicator: false,
            random_access_indicator: true,
            elementary_stream_priority_indicator: false,
            pcr: None,
            opcr: None,
            splice_countdown: None,
            transport_private_data: None,
            extension: None,
            stuffing_len: 0,
        };
        assert_eq!(af.serialized_len(), 1);
        let mut buf = [0u8; 1];
        af.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[0], AF_RANDOM_ACCESS);
        let decoded = AdaptationField::parse(&buf).unwrap();
        assert_eq!(decoded, af);
    }

    /// OutputBufferTooSmall returned when buffer is too short.
    #[test]
    fn adaptation_field_serialize_rejects_small_buffer() {
        let af = AdaptationField {
            discontinuity_indicator: false,
            random_access_indicator: false,
            elementary_stream_priority_indicator: false,
            pcr: Some(Pcr {
                base: 0,
                extension: 0,
            }),
            opcr: None,
            splice_countdown: None,
            transport_private_data: None,
            extension: None,
            stuffing_len: 0,
        };
        let mut buf = [0u8; 3]; // need 7
        assert!(matches!(
            af.serialize_into(&mut buf),
            Err(Error::OutputBufferTooSmall { .. })
        ));
    }

    // ── AdaptationFieldExtension (§2.4.3.5) ────────────────────────────────

    /// Round-trip LTW field.
    #[test]
    fn adaptation_field_extension_ltw_round_trip() {
        let ext = AdaptationFieldExtension {
            ltw: Some(Ltw {
                ltw_valid_flag: true,
                ltw_offset: 0x1234,
            }),
            piecewise_rate: None,
            seamless_splice: None,
        };
        let mut buf = vec![0u8; ext.serialized_len()];
        ext.serialize_into(&mut buf).unwrap();
        // Parse via AdaptationField (extension is the last field)
        // Build a full AdaptationField with only the extension set.
        let af = AdaptationField {
            discontinuity_indicator: false,
            random_access_indicator: false,
            elementary_stream_priority_indicator: false,
            pcr: None,
            opcr: None,
            splice_countdown: None,
            transport_private_data: None,
            extension: Some(ext),
            stuffing_len: 0,
        };
        let len = af.serialized_len();
        let mut abuf = vec![0u8; len];
        af.serialize_into(&mut abuf).unwrap();
        let decoded = AdaptationField::parse(&abuf).unwrap();
        assert_eq!(decoded.extension, Some(ext));
    }

    /// Round-trip piecewise_rate field.
    #[test]
    fn adaptation_field_extension_piecewise_rate_round_trip() {
        let af = AdaptationField {
            discontinuity_indicator: false,
            random_access_indicator: false,
            elementary_stream_priority_indicator: false,
            pcr: None,
            opcr: None,
            splice_countdown: None,
            transport_private_data: None,
            extension: Some(AdaptationFieldExtension {
                ltw: None,
                piecewise_rate: Some(0x3FFFFF), // max 22-bit
                seamless_splice: None,
            }),
            stuffing_len: 0,
        };
        let mut buf = vec![0u8; af.serialized_len()];
        af.serialize_into(&mut buf).unwrap();
        let decoded = AdaptationField::parse(&buf).unwrap();
        assert_eq!(decoded.extension.unwrap().piecewise_rate, Some(0x3FFFFF));
    }

    /// Round-trip seamless_splice field.
    #[test]
    fn adaptation_field_extension_seamless_splice_round_trip() {
        let af = AdaptationField {
            discontinuity_indicator: false,
            random_access_indicator: false,
            elementary_stream_priority_indicator: false,
            pcr: None,
            opcr: None,
            splice_countdown: None,
            transport_private_data: None,
            extension: Some(AdaptationFieldExtension {
                ltw: None,
                piecewise_rate: None,
                seamless_splice: Some(SeamlessSplice {
                    splice_type: 0xA,
                    dts_next_au: 0x1_2345_6789,
                }),
            }),
            stuffing_len: 0,
        };
        let mut buf = vec![0u8; af.serialized_len()];
        af.serialize_into(&mut buf).unwrap();
        let decoded = AdaptationField::parse(&buf).unwrap();
        let ss = decoded.extension.unwrap().seamless_splice.unwrap();
        assert_eq!(ss.splice_type, 0xA);
        assert_eq!(ss.dts_next_au, 0x1_2345_6789);
    }

    /// Round-trip with transport_private_data.
    #[test]
    fn adaptation_field_transport_private_data_round_trip() {
        let tpd = [0xDE, 0xAD, 0xBE, 0xEF];
        let af = AdaptationField {
            discontinuity_indicator: false,
            random_access_indicator: false,
            elementary_stream_priority_indicator: false,
            pcr: None,
            opcr: None,
            splice_countdown: None,
            transport_private_data: Some(&tpd),
            extension: None,
            stuffing_len: 0,
        };
        let mut buf = vec![0u8; af.serialized_len()];
        af.serialize_into(&mut buf).unwrap();
        let decoded = AdaptationField::parse(&buf).unwrap();
        assert_eq!(decoded.transport_private_data, Some(tpd.as_slice()));
    }

    /// PCR + `0xFF` stuffing round-trips byte-identical, and the stuffing is
    /// re-emitted as `0xFF` (ISO/IEC 13818-1:2007 §2.4.3.4).
    #[test]
    fn adaptation_field_stuffing_round_trip() {
        let af = AdaptationField {
            discontinuity_indicator: false,
            random_access_indicator: false,
            elementary_stream_priority_indicator: false,
            pcr: Some(Pcr {
                base: 12_345,
                extension: 7,
            }),
            opcr: None,
            splice_countdown: None,
            transport_private_data: None,
            extension: None,
            stuffing_len: 20,
        };
        // 1 flags + 6 PCR + 20 stuffing.
        assert_eq!(af.serialized_len(), 27);
        let mut buf = vec![0u8; af.serialized_len()];
        af.serialize_into(&mut buf).unwrap();
        // Trailing bytes are 0xFF stuffing.
        assert!(buf[7..27].iter().all(|&b| b == AF_STUFFING_BYTE));
        let decoded = AdaptationField::parse(&buf).unwrap();
        assert_eq!(decoded.stuffing_len, 20);
        assert_eq!(decoded, af);
        // Pure stuffing (flags-only body padded out) also round-trips.
        let pure = AdaptationField {
            discontinuity_indicator: false,
            random_access_indicator: false,
            elementary_stream_priority_indicator: false,
            pcr: None,
            opcr: None,
            splice_countdown: None,
            transport_private_data: None,
            extension: None,
            stuffing_len: 5,
        };
        let mut pbuf = vec![0u8; pure.serialized_len()];
        pure.serialize_into(&mut pbuf).unwrap();
        assert_eq!(pbuf, vec![0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(AdaptationField::parse(&pbuf).unwrap(), pure);
    }

    /// All optional fields together: PCR + splice + TPD + extension.
    #[test]
    fn adaptation_field_all_fields_round_trip() {
        let tpd = [0x01u8, 0x02, 0x03];
        let af = AdaptationField {
            discontinuity_indicator: true,
            random_access_indicator: false,
            elementary_stream_priority_indicator: false,
            pcr: Some(Pcr {
                base: 90_000,
                extension: 50,
            }),
            opcr: None,
            splice_countdown: Some(10),
            transport_private_data: Some(&tpd),
            extension: Some(AdaptationFieldExtension {
                ltw: Some(Ltw {
                    ltw_valid_flag: false,
                    ltw_offset: 500,
                }),
                piecewise_rate: Some(12345),
                seamless_splice: None,
            }),
            stuffing_len: 0,
        };
        let len = af.serialized_len();
        let mut buf = vec![0u8; len];
        af.serialize_into(&mut buf).unwrap();
        let decoded = AdaptationField::parse(&buf).unwrap();
        assert_eq!(decoded.pcr, af.pcr);
        assert_eq!(decoded.splice_countdown, af.splice_countdown);
        assert_eq!(decoded.transport_private_data, af.transport_private_data);
        assert_eq!(decoded.extension, af.extension);
        assert!(decoded.discontinuity_indicator);
    }

    /// Full round-trip: build a packet with PCR in adaptation field, parse
    /// it, re-serialize the adaptation field, re-parse, assert PCR matches.
    #[test]
    fn adaptation_field_serialize_from_real_packet_bytes() {
        // Replicate the raw packet from `adaptation_field_flags_and_pcr`.
        let mut raw = [0xAAu8; TS_PACKET_SIZE];
        raw[0] = TS_SYNC_BYTE;
        raw[1] = 0x01;
        raw[2] = 0x00;
        raw[3] = ADAPTATION_FLAG | PAYLOAD_FLAG;
        raw[4] = 7;
        raw[5] = AF_DISCONTINUITY | AF_PCR_FLAG;
        raw[6..12].copy_from_slice(&[0x00, 0x00, 0x13, 0x88, 0x7E, 0x00]);

        let pkt = TsPacket::parse(&raw).unwrap();
        let af = pkt.adaptation_field().unwrap().unwrap();

        // Re-serialize.
        let mut ser = vec![0u8; af.serialized_len()];
        af.serialize_into(&mut ser).unwrap();
        let decoded = AdaptationField::parse(&ser).unwrap();
        assert_eq!(
            decoded.pcr,
            Some(Pcr {
                base: 10_000,
                extension: 0
            })
        );
        assert!(decoded.discontinuity_indicator);
    }
}
