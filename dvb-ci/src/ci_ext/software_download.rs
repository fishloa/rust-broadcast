//! Software Download (CAM firmware) objects + DSM-CC download messages — ETSI
//! TS 101 699 V1.1.1 §6.7, Tables 74-83 (PDF pp. 64-74). See
//! `private/specs-verified/dvb/dvb-ci/ci_plus/software-download.md`.
//!
//! Resource ID `0x00051041` (fixed). A CI module acts as a firmware source for a
//! host; the host is the DSM-CC client and the module the download server, using
//! the DSM-CC (ISO/IEC 13818-6) User-Network Download protocol.
//!
//! APDU objects (§6.7.4):
//! - `download_enq` (`9F 80 00`, Table 75) — host → app: encapsulates a DSM-CC
//!   U-N message (DownloadInfoRequest / DownloadDataRequest / DownloadCancel).
//! - `download_reply` (`9F 80 01`, Table 76) — app → host: encapsulates a DSM-CC
//!   U-N message (DownloadInfoResponse / DownloadDataBlock / DownloadCancel).
//! - `user_authorization_initiate` (`9F 80 02`, Table 77) — host → app: the
//!   7-byte [`BinaryId`] + opaque `data_byte`s.
//! - `user_authorization_result` (`9F 80 03`, Table 78) — app → host: the
//!   7-byte [`BinaryId`] + opaque `result_byte`s.
//!
//! The encapsulated DSM-CC message itself is carried **opaque** by the
//! `download_enq` / `download_reply` objects (the `DSMCC_descriptor()` loop);
//! the DSM-CC message structures (Tables 79-83) are provided as separate
//! `Parse`/`Serialize` types ([`DownloadInfoRequest`], [`DownloadInfoResponse`],
//! [`DownloadCancel`], [`DownloadDataRequest`], [`DownloadDataBlock`]) that
//! decode those encapsulated bytes. Firmware payload, compatibility-descriptor
//! bodies, module info and private data are opaque borrowed `&[u8]` per §6.7.5.

use crate::error::{Error, Result};
use crate::objects;
use crate::wire::be_u16;
use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};

/// Resource-scoped `apdu_tag`s for the Download resource (Tables 75-78).
pub mod tag {
    use crate::tag::ApduTag;
    /// `download_enq_tag` = `9F 80 00`.
    pub const DOWNLOAD_ENQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x00);
    /// `download_rep_tag` = `9F 80 01`.
    pub const DOWNLOAD_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x01);
    /// `user_authorization_initiate_tag` = `9F 80 02`.
    pub const USER_AUTH_INITIATE: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x02);
    /// `user_authorization_result_tag` = `9F 80 03`.
    pub const USER_AUTH_RESULT: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x03);
}

/// `BinaryId` (Table 74, §6.7.3.1) — the 7-byte identification of a manufacturer
/// binary: a 24-bit IEEE OUI `specifier` + 16-bit `model` + 16-bit `version`.
pub const BINARY_ID_LEN: usize = 7;

/// Manufacturer-binary identification (Table 74).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct BinaryId {
    /// `specifier` — 24-bit IEEE OUI (low 24 bits).
    pub specifier: u32,
    /// `model` — 16-bit, semantics defined by the `specifier`.
    pub model: u16,
    /// `version` — 16-bit, semantics defined by the `specifier`.
    pub version: u16,
}

/// Width of `BinaryId::specifier` in bits (Table 77: `specifier` is 24 bits).
const BINARY_ID_SPECIFIER_BITS: u32 = 24;
/// Bytes of `BinaryId::specifier`.
const BINARY_ID_SPECIFIER_LEN: usize = 3;
/// Offsets of the `binary_id` fields (Table 74).
const BINARY_ID_SPECIFIER_OFFSET: usize = 0;
const BINARY_ID_MODEL_OFFSET: usize = BINARY_ID_SPECIFIER_LEN;
const BINARY_ID_VERSION_OFFSET: usize = BINARY_ID_MODEL_OFFSET + 2;

/// Bytes of the DSM-CC U-N message header (Tables 79-83).
const DSMCC_HEADER_LEN: usize = 12;
/// Offsets within the DSM-CC U-N message header.
const PROTOCOL_DISCRIMINATOR_OFFSET: usize = 0;
const DSMCC_TYPE_OFFSET: usize = 1;
const MESSAGE_ID_OFFSET: usize = 2;
const TRANSACTION_ID_OFFSET: usize = 4;
const HEADER_RESERVED_OFFSET: usize = 8;
const ADAPTATION_LENGTH_OFFSET: usize = 9;
const MESSAGE_LENGTH_OFFSET: usize = 10;
/// Value of the reserved header byte.
const DSMCC_RESERVED_BYTE: u8 = 0xFF;
/// Bit width of `adaptationLength`.
const ADAPTATION_LENGTH_BITS: u32 = 8;
/// Bit width of `messageLength`.
const MESSAGE_LENGTH_BITS: u32 = 16;

/// Bytes of the fixed part of `DownloadInfoRequest` before its compatibility
/// block (bufferSize(4) + maximumBlockSize(2)).
const INFO_REQUEST_FIXED: usize = core::mem::size_of::<u32>() + core::mem::size_of::<u16>();
/// Offsets within the `DownloadInfoResponse` fixed part.
const INFO_RESPONSE_DOWNLOAD_ID_OFFSET: usize = 0;
const INFO_RESPONSE_BLOCK_SIZE_OFFSET: usize = 4;
const INFO_RESPONSE_WINDOW_SIZE_OFFSET: usize = 6;
const INFO_RESPONSE_ACK_PERIOD_OFFSET: usize = 7;
const INFO_RESPONSE_TC_WINDOW_OFFSET: usize = 8;
const INFO_RESPONSE_TC_SCENARIO_OFFSET: usize = 12;
/// Bytes of the fixed part of `DownloadInfoResponse` before its compatibility
/// block (downloadId(4) blockSize(2) windowSize(1) ackPeriod(1)
/// tcDownloadWindow(4) tcDownloadScenario(4)).
const INFO_RESPONSE_FIXED: usize = 4 + 2 + 1 + 1 + 4 + 4;
/// Offsets within the `DownloadCancel` fixed part.
const CANCEL_DOWNLOAD_ID_OFFSET: usize = 0;
const CANCEL_MODULE_ID_OFFSET: usize = 4;
const CANCEL_BLOCK_NUMBER_OFFSET: usize = 6;
const CANCEL_REASON_OFFSET: usize = 8;
/// Bytes of the fixed part of `DownloadCancel` before its private data
/// (downloadId(4) moduleId(2) blockNumber(2) reason(1)).
const CANCEL_FIXED: usize = 4 + 2 + 2 + 1;
/// Offset of `download_reason` within a `DownloadDataRequest` body.
const DATA_REQUEST_REASON_OFFSET: usize = 4;
/// Bytes of the fixed part of `DownloadDataRequest` (moduleId(2)
/// blockNumber(2) reason(1)).
const DATA_REQUEST_FIXED: usize = 5;
/// Offset of `module_version` within a `DownloadDataBlock` body.
const DATA_BLOCK_VERSION_OFFSET: usize = 2;
/// Offset of `block_number` within a `DownloadDataBlock` body.
const DATA_BLOCK_BLOCK_NUMBER_OFFSET: usize = 4;
/// Bytes of the fixed part of `DownloadDataBlock` (moduleId(2) version(1)
/// reserved(1) blockNumber(2)).
const DATA_BLOCK_FIXED: usize = 6;
/// Bytes of one module-info entry's fixed part (moduleId(2) moduleSize(4)
/// moduleVersion(1) moduleInfoLength(1)).
const MODULE_INFO_FIXED: usize = 8;
/// Bytes of a length field that counts the bytes after itself.
const BLOCK_LENGTH_LEN: usize = core::mem::size_of::<u16>();

impl BinaryId {
    fn read(b: &[u8]) -> Self {
        Self {
            specifier: (u32::from(b[BINARY_ID_SPECIFIER_OFFSET]) << MESSAGE_LENGTH_BITS)
                | (u32::from(b[BINARY_ID_SPECIFIER_OFFSET + 1]) << ADAPTATION_LENGTH_BITS)
                | u32::from(b[BINARY_ID_SPECIFIER_OFFSET + BINARY_ID_SPECIFIER_LEN - 1]),
            model: be_u16(b, BINARY_ID_MODEL_OFFSET),
            version: be_u16(b, BINARY_ID_VERSION_OFFSET),
        }
    }
    /// Write the 7-byte `binary_id`, rejecting a `specifier` that does not fit
    /// its 24-bit field instead of dropping the top byte (audit r10-O-8).
    fn write(self, buf: &mut [u8]) -> Result<()> {
        objects::fit_bits(
            u64::from(self.specifier),
            BINARY_ID_SPECIFIER_BITS,
            "binary_id specifier",
        )?;
        let be = self.specifier.to_be_bytes();
        buf[..BINARY_ID_SPECIFIER_LEN].copy_from_slice(&be[1..]);
        buf[BINARY_ID_MODEL_OFFSET..BINARY_ID_VERSION_OFFSET]
            .copy_from_slice(&self.model.to_be_bytes());
        buf[BINARY_ID_VERSION_OFFSET..BINARY_ID_VERSION_OFFSET + core::mem::size_of::<u16>()]
            .copy_from_slice(&self.version.to_be_bytes());
        Ok(())
    }
}

// =================== APDU objects ===================

/// `download_enq()` (Table 75): host → app — encapsulates one DSM-CC U-N
/// message, carried verbatim as a borrowed byte string.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DownloadEnquiry<'a> {
    /// How the `length_field` was coded on the wire: a non-minimal long form
    /// (e.g. `82 00 09`) is preserved so the object re-serializes
    /// byte-for-byte; [`LengthForm::Minimal`](crate::length::LengthForm::Minimal) for a hand-built value.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub length_form: crate::length::LengthForm,

    /// The encapsulated DSM-CC message bytes (the `DSMCC_descriptor()` loop).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub dsmcc_message: &'a [u8],
}

/// `download_reply()` (Table 76): app → host — encapsulates one DSM-CC U-N
/// message, carried verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DownloadReply<'a> {
    /// How the `length_field` was coded on the wire: a non-minimal long form
    /// (e.g. `82 00 09`) is preserved so the object re-serializes
    /// byte-for-byte; [`LengthForm::Minimal`](crate::length::LengthForm::Minimal) for a hand-built value.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub length_form: crate::length::LengthForm,

    /// The encapsulated DSM-CC message bytes (the `DSMCC_descriptor()` loop).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub dsmcc_message: &'a [u8],
}

/// `user_authorization_initiate()` (Table 77): host → app.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct UserAuthInitiate<'a> {
    /// How the `length_field` was coded on the wire: a non-minimal long form
    /// (e.g. `82 00 09`) is preserved so the object re-serializes
    /// byte-for-byte; [`LengthForm::Minimal`](crate::length::LengthForm::Minimal) for a hand-built value.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub length_form: crate::length::LengthForm,

    /// The 7-byte [`BinaryId`].
    pub binary_id: BinaryId,
    /// `data_byte`s — optional, meaning defined by the specifier.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub data: &'a [u8],
}

/// `user_authorization_result()` (Table 78): app → host.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct UserAuthResult<'a> {
    /// How the `length_field` was coded on the wire: a non-minimal long form
    /// (e.g. `82 00 09`) is preserved so the object re-serializes
    /// byte-for-byte; [`LengthForm::Minimal`](crate::length::LengthForm::Minimal) for a hand-built value.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub length_form: crate::length::LengthForm,

    /// The 7-byte [`BinaryId`].
    pub binary_id: BinaryId,
    /// `result_byte`s — conveys the user response; meaning defined by the specifier.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub result: &'a [u8],
}

crate::dispatch::declare_opaque_apdu!(
    DownloadEnquiry,
    dsmcc_message,
    tag::DOWNLOAD_ENQ,
    "download_enq"
);
crate::dispatch::declare_opaque_apdu!(
    DownloadReply,
    dsmcc_message,
    tag::DOWNLOAD_REPLY,
    "download_reply"
);

macro_rules! user_auth_object {
    ($ty:ident, $tag:expr, $what:literal, $field:ident) => {
        impl<'a> Parse<'a> for $ty<'a> {
            type Error = Error;
            fn parse(bytes: &'a [u8]) -> Result<Self> {
                let (length_form, body) = objects::parse_apdu_header(bytes, $tag, $what)?;
                need_in(body, BINARY_ID_LEN, $what)?;
                Ok(Self {
                    length_form,
                    binary_id: BinaryId::read(body),
                    $field: &body[BINARY_ID_LEN..],
                })
            }
        }
        impl Serialize for $ty<'_> {
            type Error = Error;
            fn serialized_len(&self) -> usize {
                objects::apdu_len(BINARY_ID_LEN + self.$field.len(), self.length_form)
            }
            fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                let body_len = BINARY_ID_LEN + self.$field.len();
                let mut pos = objects::write_apdu_header($tag, body_len, self.length_form, buf)?;
                self.binary_id.write(&mut buf[pos..])?;
                pos += BINARY_ID_LEN;
                buf[pos..pos + self.$field.len()].copy_from_slice(self.$field);
                Ok(pos + self.$field.len())
            }
        }
    };
}

user_auth_object!(
    UserAuthInitiate,
    tag::USER_AUTH_INITIATE,
    "user_authorization_initiate",
    data
);
user_auth_object!(
    UserAuthResult,
    tag::USER_AUTH_RESULT,
    "user_authorization_result",
    result
);

// =================== DSM-CC U-N Download messages (Tables 79-83) ===================
//
// Reproduced from ISO/IEC 13818-6 as carried inside the DSMCC_descriptor() loop
// of the download_enq / download_reply objects. These have no apdu_tag; their
// Parse consumes the whole input slice and Serialize emits exactly the wire
// bytes. Variable-length nested blocks (compatibility descriptor, module loop,
// adaptation, private data, block payload) are opaque borrowed &[u8].

/// DSM-CC `protocolDiscriminator` for MPEG-2 DSM-CC (`0x11`).
pub const DSMCC_PROTOCOL_DISCRIMINATOR: u8 = 0x11;
/// DSM-CC `dsmccType` for U-N Download messages (`0x03`).
pub const DSMCC_TYPE_DOWNLOAD: u8 = 0x03;
/// `messageId` of DownloadInfoRequest (`0x1001`).
pub const MSG_ID_DOWNLOAD_INFO_REQUEST: u16 = 0x1001;
/// `messageId` of DownloadInfoResponse (`0x1002`).
pub const MSG_ID_DOWNLOAD_INFO_RESPONSE: u16 = 0x1002;
/// `messageId` of DownloadDataBlock (`0x1003`).
pub const MSG_ID_DOWNLOAD_DATA_BLOCK: u16 = 0x1003;
/// `messageId` of DownloadDataRequest (`0x1004`).
pub const MSG_ID_DOWNLOAD_DATA_REQUEST: u16 = 0x1004;
/// `messageId` of DownloadCancel (`0x1005`).
pub const MSG_ID_DOWNLOAD_CANCEL: u16 = 0x1005;

/// The DSM-CC message header common to the "info" / cancel messages (Tables
/// 79/80/81) and the data messages (Tables 82/83, whose `dsmccDownloadDataHeader`
/// carries a `DownloadId` in the same four bytes): protocolDiscriminator +
/// dsmccType + messageId + transactionId/DownloadId + reserved +
/// adaptationLength + messageLength, plus the optional adaptation bytes
/// (`adaptationType` + data, carried opaque). Returns
/// `(transaction_or_download_id, adaptation, rest_after_adaptation)`.
///
/// Each of these message types (`DownloadInfoRequest`/`Response`/`Cancel`) is
/// directly `Parse`-able on its own, with no outer `apdu_tag`/framing to
/// dispatch on first — `messageId` is the *only* field distinguishing one
/// from another (or from an unrelated DSM-CC message entirely), so
/// `expected_message_id` is checked here rather than left to the caller.
/// `protocolDiscriminator`/`dsmccType` are likewise checked: without it, any
/// 12+ byte buffer parses as a "valid" message of whichever type happened to
/// be asked for.
fn parse_dsmcc_header<'a>(
    body: &'a [u8],
    expected_message_id: u16,
    what: &'static str,
) -> Result<(u32, &'a [u8], &'a [u8])> {
    // protocolDiscriminator(1) dsmccType(1) messageId(2) transactionId(4)
    // reserved(1) adaptationLength(1) messageLength(2) = 12.
    const HDR: usize = DSMCC_HEADER_LEN;
    if body.len() < HDR {
        return Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: HDR,
                have: body.len(),
                what,
            },
        ));
    }
    if body[PROTOCOL_DISCRIMINATOR_OFFSET] != DSMCC_PROTOCOL_DISCRIMINATOR {
        return Err(Error::InvalidObject {
            what,
            reason: "protocolDiscriminator is not MPEG-2 DSM-CC (0x11)",
        });
    }
    if body[DSMCC_TYPE_OFFSET] != DSMCC_TYPE_DOWNLOAD {
        return Err(Error::InvalidObject {
            what,
            reason: "dsmccType is not U-N Download (0x03)",
        });
    }
    let message_id = be_u16(body, MESSAGE_ID_OFFSET);
    if message_id != expected_message_id {
        return Err(Error::InvalidObject {
            what,
            reason: "messageId does not match this message type",
        });
    }
    let id = u32::from_be_bytes([
        body[TRANSACTION_ID_OFFSET],
        body[TRANSACTION_ID_OFFSET + 1],
        body[TRANSACTION_ID_OFFSET + core::mem::size_of::<u16>()],
        body[TRANSACTION_ID_OFFSET + core::mem::size_of::<u16>() + 1],
    ]);
    let adaptation_length = body[ADAPTATION_LENGTH_OFFSET] as usize;
    if body.len() < HDR + adaptation_length {
        return Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: HDR + adaptation_length,
                have: body.len(),
                what,
            },
        ));
    }
    let declared_message_length = be_u16(body, MESSAGE_LENGTH_OFFSET) as usize;
    // messageLength counts everything after the messageLength field itself
    // (§6.7.5: adaptationType/adaptationDataBytes + the message body).
    let actual_message_length = body.len() - HDR;
    if declared_message_length != actual_message_length {
        return Err(Error::LengthMismatch(
            broadcast_common::error::LengthMismatch {
                what,
                declared: declared_message_length,
                actual: actual_message_length,
            },
        ));
    }
    let adaptation = &body[HDR..HDR + adaptation_length];
    let rest = &body[HDR + adaptation_length..];
    Ok((id, adaptation, rest))
}

fn write_dsmcc_header(
    buf: &mut [u8],
    message_id: u16,
    id: u32,
    adaptation: &[u8],
    message_length: usize,
) -> Result<usize> {
    objects::fit_bits(
        adaptation.len() as u64,
        ADAPTATION_LENGTH_BITS,
        "DSM-CC adaptationLength",
    )?;
    objects::fit_bits(
        message_length as u64,
        MESSAGE_LENGTH_BITS,
        "DSM-CC messageLength",
    )?;
    buf[PROTOCOL_DISCRIMINATOR_OFFSET] = DSMCC_PROTOCOL_DISCRIMINATOR;
    buf[DSMCC_TYPE_OFFSET] = DSMCC_TYPE_DOWNLOAD;
    buf[MESSAGE_ID_OFFSET..TRANSACTION_ID_OFFSET].copy_from_slice(&message_id.to_be_bytes());
    buf[TRANSACTION_ID_OFFSET..HEADER_RESERVED_OFFSET].copy_from_slice(&id.to_be_bytes());
    buf[HEADER_RESERVED_OFFSET] = DSMCC_RESERVED_BYTE; // reserved
    buf[ADAPTATION_LENGTH_OFFSET] = adaptation.len() as u8;
    buf[MESSAGE_LENGTH_OFFSET..DSMCC_HEADER_LEN]
        .copy_from_slice(&(message_length as u16).to_be_bytes());
    buf[DSMCC_HEADER_LEN..DSMCC_HEADER_LEN + adaptation.len()].copy_from_slice(adaptation);
    Ok(DSMCC_HEADER_LEN + adaptation.len())
}

/// `DownloadInfoRequest()` (Table 79) — client (host) → server. The
/// compatibility-descriptor block and private-data block are opaque (their
/// inner descriptor loops are manufacturer-defined per §6.7.5).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DownloadInfoRequest<'a> {
    /// `transactionId` (client assigned; 2 MSBs zero per DSM-CC).
    pub transaction_id: u32,
    /// `adaptationType` + `adaptationDataByte`s (opaque CA/private adaptation).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub adaptation: &'a [u8],
    /// `bufferSize`.
    pub buffer_size: u32,
    /// `maximumBlockSize`.
    pub maximum_block_size: u16,
    /// `compatibilityDescriptor()` — `compatibilityDescriptorLength` +
    /// `descriptorCount` + the descriptor loop, carried verbatim.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub compatibility_descriptor: &'a [u8],
    /// `privateDataByte`s (`privateDataLength`-prefixed; per §6.7.5.4 shall be empty).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub private_data: &'a [u8],
}

impl<'a> Parse<'a> for DownloadInfoRequest<'a> {
    type Error = Error;
    fn parse(body: &'a [u8]) -> Result<Self> {
        let what = "DownloadInfoRequest";
        let (transaction_id, adaptation, rest) =
            parse_dsmcc_header(body, MSG_ID_DOWNLOAD_INFO_REQUEST, what)?;
        // bufferSize(4) maximumBlockSize(2) compatibilityDescriptorLength(2).
        if rest.len() < INFO_REQUEST_FIXED + BLOCK_LENGTH_LEN {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: INFO_REQUEST_FIXED + BLOCK_LENGTH_LEN,
                    have: rest.len(),
                    what,
                },
            ));
        }
        let buffer_size = u32::from_be_bytes([
            rest[0],
            rest[1],
            rest[core::mem::size_of::<u16>()],
            rest[core::mem::size_of::<u16>() + 1],
        ]);
        let maximum_block_size = be_u16(rest, core::mem::size_of::<u32>());
        // compatibilityDescriptorLength counts the bytes *after* the length field
        // (descriptorCount + descriptor loop).
        let compat_len = be_u16(rest, INFO_REQUEST_FIXED) as usize;
        // start of compatibilityDescriptor block (incl. its length field)
        let compat_start = INFO_REQUEST_FIXED;
        let compat_block_end = compat_start + BLOCK_LENGTH_LEN + compat_len;
        if rest.len() < compat_block_end + BLOCK_LENGTH_LEN {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: compat_block_end + BLOCK_LENGTH_LEN,
                    have: rest.len(),
                    what,
                },
            ));
        }
        let compatibility_descriptor = &rest[compat_start..compat_block_end];
        let priv_len = be_u16(rest, compat_block_end) as usize;
        let priv_start = compat_block_end + BLOCK_LENGTH_LEN;
        let priv_end = priv_start + priv_len;
        if rest.len() < priv_end {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: priv_end,
                    have: rest.len(),
                    what,
                },
            ));
        }
        Ok(Self {
            transaction_id,
            adaptation,
            buffer_size,
            maximum_block_size,
            compatibility_descriptor,
            private_data: &rest[priv_start..priv_end],
        })
    }
}
impl Serialize for DownloadInfoRequest<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        DSMCC_HEADER_LEN
            + self.adaptation.len()
            + INFO_REQUEST_FIXED
            + self.compatibility_descriptor.len()
            + BLOCK_LENGTH_LEN
            + self.private_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        need_out(buf, total)?;
        // messageLength = everything after the messageLength field itself.
        let message_length = total - DSMCC_HEADER_LEN;
        objects::fit_bits(
            self.private_data.len() as u64,
            MESSAGE_LENGTH_BITS,
            "DownloadInfoRequest privateDataLength",
        )?;
        let mut pos = write_dsmcc_header(
            buf,
            MSG_ID_DOWNLOAD_INFO_REQUEST,
            self.transaction_id,
            self.adaptation,
            message_length,
        )?;
        buf[pos..pos + core::mem::size_of::<u32>()]
            .copy_from_slice(&self.buffer_size.to_be_bytes());
        pos += core::mem::size_of::<u32>();
        buf[pos..pos + core::mem::size_of::<u16>()]
            .copy_from_slice(&self.maximum_block_size.to_be_bytes());
        pos += core::mem::size_of::<u16>();
        buf[pos..pos + self.compatibility_descriptor.len()]
            .copy_from_slice(self.compatibility_descriptor);
        pos += self.compatibility_descriptor.len();
        buf[pos..pos + BLOCK_LENGTH_LEN]
            .copy_from_slice(&(self.private_data.len() as u16).to_be_bytes());
        pos += BLOCK_LENGTH_LEN;
        buf[pos..pos + self.private_data.len()].copy_from_slice(self.private_data);
        Ok(pos + self.private_data.len())
    }
}

/// `DownloadInfoResponse()` (Table 80) — server (module) → client. The
/// compatibility-descriptor block, the per-module info block and the private
/// data are opaque borrowed `&[u8]`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DownloadInfoResponse<'a> {
    /// `transactionId` (matches the request).
    pub transaction_id: u32,
    /// Opaque adaptation bytes.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub adaptation: &'a [u8],
    /// `downloadId`.
    pub download_id: u32,
    /// `blockSize`.
    pub block_size: u16,
    /// `windowSize`.
    pub window_size: u8,
    /// `ackPeriod`.
    pub ack_period: u8,
    /// `tCDownloadWindow`.
    pub tc_download_window: u32,
    /// `tCDownloadScenario`.
    pub tc_download_scenario: u32,
    /// `compatibilityDescriptor()` block, carried verbatim (incl. its length field).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub compatibility_descriptor: &'a [u8],
    /// The `numberOfModules` module loop (`numberOfModules` + the per-module
    /// entries), carried verbatim.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub modules: &'a [u8],
    /// `privateDataByte`s.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub private_data: &'a [u8],
}

impl<'a> Parse<'a> for DownloadInfoResponse<'a> {
    type Error = Error;
    fn parse(body: &'a [u8]) -> Result<Self> {
        let what = "DownloadInfoResponse";
        let (transaction_id, adaptation, rest) =
            parse_dsmcc_header(body, MSG_ID_DOWNLOAD_INFO_RESPONSE, what)?;
        // downloadId(4) blockSize(2) windowSize(1) ackPeriod(1)
        // tCDownloadWindow(4) tCDownloadScenario(4) compatibilityDescriptorLength(2).
        const FIXED: usize = INFO_RESPONSE_FIXED + BLOCK_LENGTH_LEN;
        if rest.len() < FIXED {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: FIXED,
                    have: rest.len(),
                    what,
                },
            ));
        }
        let download_id = u32::from_be_bytes([
            rest[INFO_RESPONSE_DOWNLOAD_ID_OFFSET],
            rest[INFO_RESPONSE_DOWNLOAD_ID_OFFSET + 1],
            rest[INFO_RESPONSE_DOWNLOAD_ID_OFFSET + core::mem::size_of::<u16>()],
            rest[INFO_RESPONSE_DOWNLOAD_ID_OFFSET + core::mem::size_of::<u16>() + 1],
        ]);
        let block_size = be_u16(rest, INFO_RESPONSE_BLOCK_SIZE_OFFSET);
        let window_size = rest[INFO_RESPONSE_WINDOW_SIZE_OFFSET];
        let ack_period = rest[INFO_RESPONSE_ACK_PERIOD_OFFSET];
        let tc_download_window = u32::from_be_bytes([
            rest[INFO_RESPONSE_TC_WINDOW_OFFSET],
            rest[INFO_RESPONSE_TC_WINDOW_OFFSET + 1],
            rest[INFO_RESPONSE_TC_WINDOW_OFFSET + core::mem::size_of::<u16>()],
            rest[INFO_RESPONSE_TC_WINDOW_OFFSET + core::mem::size_of::<u16>() + 1],
        ]);
        let tc_download_scenario = u32::from_be_bytes([
            rest[INFO_RESPONSE_TC_SCENARIO_OFFSET],
            rest[INFO_RESPONSE_TC_SCENARIO_OFFSET + 1],
            rest[INFO_RESPONSE_TC_SCENARIO_OFFSET + core::mem::size_of::<u16>()],
            rest[INFO_RESPONSE_TC_SCENARIO_OFFSET + core::mem::size_of::<u16>() + 1],
        ]);
        let compat_len_off = INFO_RESPONSE_FIXED;
        let compat_len = be_u16(rest, compat_len_off) as usize;
        let compat_block_end = compat_len_off + BLOCK_LENGTH_LEN + compat_len;
        // + numberOfModules(2) at compat_block_end.
        if rest.len() < compat_block_end + BLOCK_LENGTH_LEN {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: compat_block_end + BLOCK_LENGTH_LEN,
                    have: rest.len(),
                    what,
                },
            ));
        }
        let compatibility_descriptor = &rest[compat_len_off..compat_block_end];
        let number_of_modules = be_u16(rest, compat_block_end) as usize;
        let mut mpos = compat_block_end + BLOCK_LENGTH_LEN;
        for _ in 0..number_of_modules {
            // moduleId(2) moduleSize(4) moduleVersion(1) moduleInfoLength(1).
            if rest.len() < mpos + MODULE_INFO_FIXED {
                return Err(Error::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need: mpos + MODULE_INFO_FIXED,
                        have: rest.len(),
                        what,
                    },
                ));
            }
            let module_info_len = rest[mpos + MODULE_INFO_FIXED - 1] as usize;
            mpos += MODULE_INFO_FIXED + module_info_len;
            if rest.len() < mpos {
                return Err(Error::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need: mpos,
                        have: rest.len(),
                        what,
                    },
                ));
            }
        }
        let modules = &rest[compat_block_end..mpos];
        if rest.len() < mpos + BLOCK_LENGTH_LEN {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: mpos + BLOCK_LENGTH_LEN,
                    have: rest.len(),
                    what,
                },
            ));
        }
        let priv_len = be_u16(rest, mpos) as usize;
        let priv_start = mpos + BLOCK_LENGTH_LEN;
        let priv_end = priv_start + priv_len;
        if rest.len() < priv_end {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: priv_end,
                    have: rest.len(),
                    what,
                },
            ));
        }
        Ok(Self {
            transaction_id,
            adaptation,
            download_id,
            block_size,
            window_size,
            ack_period,
            tc_download_window,
            tc_download_scenario,
            compatibility_descriptor,
            modules,
            private_data: &rest[priv_start..priv_end],
        })
    }
}
impl Serialize for DownloadInfoResponse<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        DSMCC_HEADER_LEN
            + self.adaptation.len()
            + INFO_RESPONSE_FIXED
            + self.compatibility_descriptor.len()
            + self.modules.len()
            + BLOCK_LENGTH_LEN
            + self.private_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        need_out(buf, total)?;
        let message_length = total - DSMCC_HEADER_LEN;
        objects::fit_bits(
            self.private_data.len() as u64,
            MESSAGE_LENGTH_BITS,
            "DownloadInfoResponse privateDataLength",
        )?;
        let mut pos = write_dsmcc_header(
            buf,
            MSG_ID_DOWNLOAD_INFO_RESPONSE,
            self.transaction_id,
            self.adaptation,
            message_length,
        )?;
        buf[pos..pos + core::mem::size_of::<u32>()]
            .copy_from_slice(&self.download_id.to_be_bytes());
        pos += core::mem::size_of::<u32>();
        buf[pos..pos + core::mem::size_of::<u16>()].copy_from_slice(&self.block_size.to_be_bytes());
        pos += core::mem::size_of::<u16>();
        buf[pos] = self.window_size;
        buf[pos + 1] = self.ack_period;
        pos += core::mem::size_of::<u16>();
        buf[pos..pos + core::mem::size_of::<u32>()]
            .copy_from_slice(&self.tc_download_window.to_be_bytes());
        pos += core::mem::size_of::<u32>();
        buf[pos..pos + core::mem::size_of::<u32>()]
            .copy_from_slice(&self.tc_download_scenario.to_be_bytes());
        pos += core::mem::size_of::<u32>();
        buf[pos..pos + self.compatibility_descriptor.len()]
            .copy_from_slice(self.compatibility_descriptor);
        pos += self.compatibility_descriptor.len();
        buf[pos..pos + self.modules.len()].copy_from_slice(self.modules);
        pos += self.modules.len();
        buf[pos..pos + BLOCK_LENGTH_LEN]
            .copy_from_slice(&(self.private_data.len() as u16).to_be_bytes());
        pos += BLOCK_LENGTH_LEN;
        buf[pos..pos + self.private_data.len()].copy_from_slice(self.private_data);
        Ok(pos + self.private_data.len())
    }
}

/// `DownloadCancel()` (Table 81).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DownloadCancel<'a> {
    /// `transactionId` (server assigned).
    pub transaction_id: u32,
    /// Opaque adaptation bytes.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub adaptation: &'a [u8],
    /// `downloadId`.
    pub download_id: u32,
    /// `moduleId`.
    pub module_id: u16,
    /// `blockNumber`.
    pub block_number: u16,
    /// `downloadCancelReason`.
    pub download_cancel_reason: u8,
    /// `privateDataByte`s.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub private_data: &'a [u8],
}

impl<'a> Parse<'a> for DownloadCancel<'a> {
    type Error = Error;
    fn parse(body: &'a [u8]) -> Result<Self> {
        let what = "DownloadCancel";
        let (transaction_id, adaptation, rest) =
            parse_dsmcc_header(body, MSG_ID_DOWNLOAD_CANCEL, what)?;
        // downloadId(4) moduleId(2) blockNumber(2) downloadCancelReason(1) privateDataLength(2).
        const FIXED: usize = CANCEL_FIXED + BLOCK_LENGTH_LEN;
        if rest.len() < FIXED {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: FIXED,
                    have: rest.len(),
                    what,
                },
            ));
        }
        let download_id = u32::from_be_bytes([
            rest[CANCEL_DOWNLOAD_ID_OFFSET],
            rest[CANCEL_DOWNLOAD_ID_OFFSET + 1],
            rest[CANCEL_DOWNLOAD_ID_OFFSET + core::mem::size_of::<u16>()],
            rest[CANCEL_DOWNLOAD_ID_OFFSET + core::mem::size_of::<u16>() + 1],
        ]);
        let module_id = be_u16(rest, CANCEL_MODULE_ID_OFFSET);
        let block_number = be_u16(rest, CANCEL_BLOCK_NUMBER_OFFSET);
        let download_cancel_reason = rest[CANCEL_REASON_OFFSET];
        let priv_len = be_u16(rest, CANCEL_FIXED) as usize;
        let priv_start = CANCEL_FIXED + BLOCK_LENGTH_LEN;
        let priv_end = priv_start + priv_len;
        if rest.len() < priv_end {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: priv_end,
                    have: rest.len(),
                    what,
                },
            ));
        }
        Ok(Self {
            transaction_id,
            adaptation,
            download_id,
            module_id,
            block_number,
            download_cancel_reason,
            private_data: &rest[priv_start..priv_end],
        })
    }
}
impl Serialize for DownloadCancel<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        DSMCC_HEADER_LEN
            + self.adaptation.len()
            + CANCEL_FIXED
            + BLOCK_LENGTH_LEN
            + self.private_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        need_out(buf, total)?;
        let message_length = total - DSMCC_HEADER_LEN;
        objects::fit_bits(
            self.private_data.len() as u64,
            MESSAGE_LENGTH_BITS,
            "DownloadCancel privateDataLength",
        )?;
        let mut pos = write_dsmcc_header(
            buf,
            MSG_ID_DOWNLOAD_CANCEL,
            self.transaction_id,
            self.adaptation,
            message_length,
        )?;
        buf[pos..pos + core::mem::size_of::<u32>()]
            .copy_from_slice(&self.download_id.to_be_bytes());
        pos += core::mem::size_of::<u32>();
        buf[pos..pos + core::mem::size_of::<u16>()].copy_from_slice(&self.module_id.to_be_bytes());
        pos += core::mem::size_of::<u16>();
        buf[pos..pos + core::mem::size_of::<u16>()]
            .copy_from_slice(&self.block_number.to_be_bytes());
        pos += core::mem::size_of::<u16>();
        buf[pos] = self.download_cancel_reason;
        pos += 1;
        buf[pos..pos + BLOCK_LENGTH_LEN]
            .copy_from_slice(&(self.private_data.len() as u16).to_be_bytes());
        pos += BLOCK_LENGTH_LEN;
        buf[pos..pos + self.private_data.len()].copy_from_slice(self.private_data);
        Ok(pos + self.private_data.len())
    }
}

/// `DownloadDataRequest()` (Table 82) — client (host) → server.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DownloadDataRequest<'a> {
    /// `DownloadId`.
    pub download_id: u32,
    /// Opaque adaptation bytes.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub adaptation: &'a [u8],
    /// `moduleId`.
    pub module_id: u16,
    /// `blockNumber`.
    pub block_number: u16,
    /// `downloadReason`.
    pub download_reason: u8,
}

impl<'a> Parse<'a> for DownloadDataRequest<'a> {
    type Error = Error;
    fn parse(body: &'a [u8]) -> Result<Self> {
        let what = "DownloadDataRequest";
        let (download_id, adaptation, rest) =
            parse_dsmcc_header(body, MSG_ID_DOWNLOAD_DATA_REQUEST, what)?;
        // moduleId(2) blockNumber(2) downloadReason(1).
        const FIXED: usize = 5;
        if rest.len() < FIXED {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: FIXED,
                    have: rest.len(),
                    what,
                },
            ));
        }
        Ok(Self {
            download_id,
            adaptation,
            module_id: be_u16(rest, 0),
            block_number: be_u16(rest, core::mem::size_of::<u16>()),
            download_reason: rest[DATA_REQUEST_REASON_OFFSET],
        })
    }
}
impl Serialize for DownloadDataRequest<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        DSMCC_HEADER_LEN + self.adaptation.len() + DATA_REQUEST_FIXED
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        need_out(buf, total)?;
        let message_length = total - DSMCC_HEADER_LEN;
        let mut pos = write_dsmcc_header(
            buf,
            MSG_ID_DOWNLOAD_DATA_REQUEST,
            self.download_id,
            self.adaptation,
            message_length,
        )?;
        buf[pos..pos + core::mem::size_of::<u16>()].copy_from_slice(&self.module_id.to_be_bytes());
        pos += core::mem::size_of::<u16>();
        buf[pos..pos + core::mem::size_of::<u16>()]
            .copy_from_slice(&self.block_number.to_be_bytes());
        pos += core::mem::size_of::<u16>();
        buf[pos] = self.download_reason;
        Ok(pos + 1)
    }
}

/// `DownloadDataBlock()` (Table 83) — server (module) → client. The
/// `blockDataByte`s carry the (opaque) firmware payload.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DownloadDataBlock<'a> {
    /// `DownloadId`.
    pub download_id: u32,
    /// Opaque adaptation bytes.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub adaptation: &'a [u8],
    /// `moduleId`.
    pub module_id: u16,
    /// `moduleVersion`.
    pub module_version: u8,
    /// `blockNumber`.
    pub block_number: u16,
    /// `blockDataByte`s — opaque firmware payload.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub block_data: &'a [u8],
}

impl<'a> Parse<'a> for DownloadDataBlock<'a> {
    type Error = Error;
    fn parse(body: &'a [u8]) -> Result<Self> {
        let what = "DownloadDataBlock";
        let (download_id, adaptation, rest) =
            parse_dsmcc_header(body, MSG_ID_DOWNLOAD_DATA_BLOCK, what)?;
        // moduleId(2) moduleVersion(1) reserved(1) blockNumber(2).
        const FIXED: usize = 6;
        if rest.len() < FIXED {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: FIXED,
                    have: rest.len(),
                    what,
                },
            ));
        }
        Ok(Self {
            download_id,
            adaptation,
            module_id: be_u16(rest, 0),
            module_version: rest[DATA_BLOCK_VERSION_OFFSET],
            // rest[3] = reserved
            block_number: be_u16(rest, DATA_BLOCK_BLOCK_NUMBER_OFFSET),
            block_data: &rest[FIXED..],
        })
    }
}
impl Serialize for DownloadDataBlock<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        DSMCC_HEADER_LEN + self.adaptation.len() + DATA_BLOCK_FIXED + self.block_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        need_out(buf, total)?;
        let message_length = total - DSMCC_HEADER_LEN;
        let mut pos = write_dsmcc_header(
            buf,
            MSG_ID_DOWNLOAD_DATA_BLOCK,
            self.download_id,
            self.adaptation,
            message_length,
        )?;
        buf[pos..pos + core::mem::size_of::<u16>()].copy_from_slice(&self.module_id.to_be_bytes());
        pos += core::mem::size_of::<u16>();
        buf[pos] = self.module_version;
        buf[pos + 1] = DSMCC_RESERVED_BYTE; // reserved
        pos += core::mem::size_of::<u16>();
        buf[pos..pos + core::mem::size_of::<u16>()]
            .copy_from_slice(&self.block_number.to_be_bytes());
        pos += core::mem::size_of::<u16>();
        buf[pos..pos + self.block_data.len()].copy_from_slice(self.block_data);
        Ok(pos + self.block_data.len())
    }
}

crate::dispatch::declare_resource_apdus! {
    /// Resource-scoped dispatch over the Download APDU objects (Tables 75-78).
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize))]
    #[non_exhaustive]
    pub enum DownloadApdu<'a> ("download") {
        /// `download_enq` (`9F 80 00`).
        DownloadEnquiry(DownloadEnquiry<'a>) = tag::DOWNLOAD_ENQ,
        /// `download_reply` (`9F 80 01`).
        DownloadReply(DownloadReply<'a>) = tag::DOWNLOAD_REPLY,
        /// `user_authorization_initiate` (`9F 80 02`).
        UserAuthInitiate(UserAuthInitiate<'a>) = tag::USER_AUTH_INITIATE,
        /// `user_authorization_result` (`9F 80 03`).
        UserAuthResult(UserAuthResult<'a>) = tag::USER_AUTH_RESULT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_enquiry_round_trips_and_bites() {
        let enq = DownloadEnquiry {
            length_form: crate::length::LengthForm::Minimal,
            dsmcc_message: &[0x11, 0x03, 0x10, 0x01],
        };
        let bytes = enq.try_to_bytes().unwrap();
        assert_eq!(bytes, [0x9F, 0x80, 0x00, 0x04, 0x11, 0x03, 0x10, 0x01]);
        assert_eq!(DownloadEnquiry::parse(&bytes).unwrap(), enq);
        let other = DownloadEnquiry {
            length_form: crate::length::LengthForm::Minimal,
            dsmcc_message: &[0x11, 0x03, 0x10, 0x02],
        };
        assert_ne!(bytes, other.try_to_bytes().unwrap());
    }

    #[test]
    fn download_reply_round_trips() {
        let rep = DownloadReply {
            length_form: crate::length::LengthForm::Minimal,
            dsmcc_message: &[0xAA],
        };
        let bytes = rep.try_to_bytes().unwrap();
        assert_eq!(bytes, [0x9F, 0x80, 0x01, 0x01, 0xAA]);
        assert_eq!(DownloadReply::parse(&bytes).unwrap(), rep);
    }

    #[test]
    fn binary_id_specifier_over_24_bits_is_rejected_not_truncated() {
        let uai = UserAuthInitiate {
            length_form: crate::length::LengthForm::Minimal,
            binary_id: BinaryId {
                specifier: 0x01_00_1B_67, // 25 bits: the top byte used to vanish
                model: 0,
                version: 0,
            },
            data: &[],
        };
        let mut buf = [0u8; 32];
        assert!(matches!(
            uai.serialize_into(&mut buf),
            Err(Error::InvalidObject { .. })
        ));
    }

    #[test]
    fn user_auth_initiate_round_trips_and_bites() {
        let uai = UserAuthInitiate {
            length_form: crate::length::LengthForm::Minimal,
            binary_id: BinaryId {
                specifier: 0x00_1B_67,
                model: 0x1234,
                version: 0x0005,
            },
            data: &[0xCA, 0xFE],
        };
        let bytes = uai.try_to_bytes().unwrap();
        // tag(3) + len(1=9) + specifier(3) model(2) version(2) data(2).
        assert_eq!(
            bytes,
            [
                0x9F, 0x80, 0x02, 0x09, 0x00, 0x1B, 0x67, 0x12, 0x34, 0x00, 0x05, 0xCA, 0xFE
            ]
        );
        assert_eq!(UserAuthInitiate::parse(&bytes).unwrap(), uai);
        let mut other = uai.clone();
        other.binary_id.version = 0x0006;
        assert_ne!(bytes, other.try_to_bytes().unwrap());
    }

    #[test]
    fn user_auth_result_round_trips() {
        let uar = UserAuthResult {
            length_form: crate::length::LengthForm::Minimal,
            binary_id: BinaryId {
                specifier: 0xAABBCC & 0x00FF_FFFF,
                model: 1,
                version: 2,
            },
            result: &[0x01],
        };
        let bytes = uar.try_to_bytes().unwrap();
        assert_eq!(
            bytes,
            [
                0x9F, 0x80, 0x03, 0x08, 0xAA, 0xBB, 0xCC, 0x00, 0x01, 0x00, 0x02, 0x01
            ]
        );
        assert_eq!(UserAuthResult::parse(&bytes).unwrap(), uar);
    }

    #[test]
    fn download_info_request_round_trips_and_bites() {
        // compatibilityDescriptor block: length(2)=0x0004 + descriptorCount(2)=0x0000
        // (the two-byte length counts the bytes after itself = descriptorCount + loop;
        // here just descriptorCount, 2 bytes -> wait, 0x0004 means 4 bytes follow).
        // Use compat = [00 04, 00 00, AA BB]  (len 4 counts descriptorCount(2)+2 loop bytes).
        let compat = [0x00, 0x04, 0x00, 0x00, 0xAA, 0xBB];
        let req = DownloadInfoRequest {
            transaction_id: 0x0000_0001,
            adaptation: &[],
            buffer_size: 0x0001_0000,
            maximum_block_size: 0x0200,
            compatibility_descriptor: &compat,
            private_data: &[],
        };
        let bytes = req.try_to_bytes().unwrap();
        assert_eq!(DownloadInfoRequest::parse(&bytes).unwrap(), req);
        // Verify header is well-formed.
        assert_eq!(bytes[0], 0x11);
        assert_eq!(bytes[1], 0x03);
        assert_eq!(&bytes[2..4], &[0x10, 0x01]);
        let mut other = req.clone();
        other.buffer_size = 0x0002_0000;
        assert_ne!(bytes, other.try_to_bytes().unwrap());
    }

    /// r10-W-13: `parse_dsmcc_header`/`parse_dsmcc_data_header` must check
    /// `protocolDiscriminator`, `dsmccType`, `messageId` and `messageLength`
    /// rather than skip straight past them. Each of these message types is
    /// directly `Parse`-able with `messageId` the only field distinguishing
    /// it from any other DSM-CC download message — pre-fix, feeding a
    /// `DownloadCancel`'s own wire bytes into `DownloadInfoRequest::parse`
    /// silently produced a (nonsensical) `DownloadInfoRequest` instead of
    /// erroring.
    #[test]
    fn dsmcc_header_rejects_wrong_message_id() {
        let cancel = DownloadCancel {
            transaction_id: 1,
            adaptation: &[],
            download_id: 2,
            module_id: 3,
            block_number: 4,
            download_cancel_reason: 5,
            private_data: &[],
        };
        let cancel_bytes = cancel.try_to_bytes().unwrap();
        // Same header shape (parse_dsmcc_header), wrong messageId.
        assert!(matches!(
            DownloadInfoRequest::parse(&cancel_bytes),
            Err(Error::InvalidObject { .. })
        ));

        let data_req = DownloadDataRequest {
            download_id: 1,
            adaptation: &[],
            module_id: 2,
            block_number: 3,
            download_reason: 4,
        };
        let data_req_bytes = data_req.try_to_bytes().unwrap();
        // Same header shape (parse_dsmcc_data_header), wrong messageId.
        assert!(matches!(
            DownloadDataBlock::parse(&data_req_bytes),
            Err(Error::InvalidObject { .. })
        ));
    }

    #[test]
    fn dsmcc_header_rejects_wrong_protocol_discriminator_and_type() {
        let req = DownloadInfoRequest {
            transaction_id: 1,
            adaptation: &[],
            buffer_size: 0,
            maximum_block_size: 0,
            compatibility_descriptor: &[0x00, 0x00],
            private_data: &[0x00, 0x00],
        };
        let mut bytes = req.try_to_bytes().unwrap();
        bytes[0] = 0x12; // wrong protocolDiscriminator
        assert!(matches!(
            DownloadInfoRequest::parse(&bytes),
            Err(Error::InvalidObject { .. })
        ));

        let mut bytes = req.try_to_bytes().unwrap();
        bytes[1] = 0x04; // wrong dsmccType
        assert!(matches!(
            DownloadInfoRequest::parse(&bytes),
            Err(Error::InvalidObject { .. })
        ));
    }

    #[test]
    fn dsmcc_header_rejects_wrong_message_length() {
        let req = DownloadInfoRequest {
            transaction_id: 1,
            adaptation: &[],
            buffer_size: 0,
            maximum_block_size: 0,
            compatibility_descriptor: &[0x00, 0x00],
            private_data: &[0x00, 0x00],
        };
        let mut bytes = req.try_to_bytes().unwrap();
        // messageLength is bytes[10..12]; corrupt it without changing the
        // buffer's actual length.
        bytes[11] ^= 0xFF;
        assert!(matches!(
            DownloadInfoRequest::parse(&bytes),
            Err(Error::LengthMismatch(..))
        ));
    }

    #[test]
    fn download_info_request_with_adaptation() {
        let compat = [0x00, 0x02, 0x00, 0x00];
        let req = DownloadInfoRequest {
            transaction_id: 0x12,
            adaptation: &[0x01, 0x02, 0x03],
            buffer_size: 1,
            maximum_block_size: 2,
            compatibility_descriptor: &compat,
            private_data: &[],
        };
        let bytes = req.try_to_bytes().unwrap();
        assert_eq!(bytes[9], 0x03); // adaptationLength
        assert_eq!(DownloadInfoRequest::parse(&bytes).unwrap(), req);
    }

    #[test]
    fn download_info_response_multi_module_round_trips_and_bites() {
        let compat = [0x00, 0x02, 0x00, 0x00];
        // Two modules (>=2 loop): each moduleId(2) moduleSize(4) moduleVersion(1)
        // moduleInfoLength(1) [info].
        // numberOfModules(2)=0x0002, then mod0 (infoLen 1) + mod1 (infoLen 0).
        let modules = [
            0x00, 0x02, // numberOfModules = 2
            0x00, 0x01, 0x00, 0x00, 0x00, 0x10, 0x01, 0x01, 0xFF, // module 0 (info=[FF])
            0x00, 0x02, 0x00, 0x00, 0x00, 0x20, 0x02, 0x00, // module 1 (info empty)
        ];
        let resp = DownloadInfoResponse {
            transaction_id: 1,
            adaptation: &[],
            download_id: 0xDEAD_BEEF,
            block_size: 0x0100,
            window_size: 4,
            ack_period: 2,
            tc_download_window: 1000,
            tc_download_scenario: 2000,
            compatibility_descriptor: &compat,
            modules: &modules,
            private_data: &[],
        };
        let bytes = resp.try_to_bytes().unwrap();
        assert_eq!(DownloadInfoResponse::parse(&bytes).unwrap(), resp);
        let mut other = resp.clone();
        other.window_size = 5;
        assert_ne!(bytes, other.try_to_bytes().unwrap());
    }

    #[test]
    fn download_cancel_round_trips() {
        let cancel = DownloadCancel {
            transaction_id: 0x10,
            adaptation: &[],
            download_id: 0x01,
            module_id: 0x02,
            block_number: 0x03,
            download_cancel_reason: 0x05,
            private_data: &[],
        };
        let bytes = cancel.try_to_bytes().unwrap();
        assert_eq!(DownloadCancel::parse(&bytes).unwrap(), cancel);
        assert_eq!(&bytes[2..4], &[0x10, 0x05]); // messageId DownloadCancel
    }

    #[test]
    fn download_data_request_round_trips_and_bites() {
        let req = DownloadDataRequest {
            download_id: 0xDEAD_BEEF,
            adaptation: &[],
            module_id: 0x0001,
            block_number: 0x0002,
            download_reason: 0x00,
        };
        let bytes = req.try_to_bytes().unwrap();
        assert_eq!(DownloadDataRequest::parse(&bytes).unwrap(), req);
        // messageId DownloadDataRequest, then DownloadId (the data header puts it
        // at offset 4, before reserved/adaptation/messageLength).
        assert_eq!(&bytes[2..4], &[0x10, 0x04]);
        assert_eq!(&bytes[4..8], &0xDEAD_BEEFu32.to_be_bytes());
        let mut other = req.clone();
        other.block_number = 0x0003;
        assert_ne!(bytes, other.try_to_bytes().unwrap());
    }

    #[test]
    fn download_data_block_round_trips_and_bites() {
        let block = DownloadDataBlock {
            download_id: 0x0000_0001,
            adaptation: &[],
            module_id: 0x0001,
            module_version: 0x02,
            block_number: 0x0003,
            block_data: &[0xFE, 0xED, 0xFA, 0xCE],
        };
        let bytes = block.try_to_bytes().unwrap();
        assert_eq!(DownloadDataBlock::parse(&bytes).unwrap(), block);
        assert_eq!(&bytes[2..4], &[0x10, 0x03]); // messageId DownloadDataBlock
        let mut other = block.clone();
        other.block_data = &[0xFE, 0xED, 0xFA, 0xCF];
        assert_ne!(bytes, other.try_to_bytes().unwrap());
    }

    #[test]
    fn enquiry_round_trips_a_real_dsmcc_message() {
        // Build a DownloadDataRequest, wrap it in a DownloadEnquiry, round-trip both.
        let inner = DownloadDataRequest {
            download_id: 0x1234_5678,
            adaptation: &[],
            module_id: 1,
            block_number: 1,
            download_reason: 0,
        };
        let inner_bytes = inner.try_to_bytes().unwrap();
        let enq = DownloadEnquiry {
            length_form: crate::length::LengthForm::Minimal,
            dsmcc_message: &inner_bytes,
        };
        let outer = enq.try_to_bytes().unwrap();
        let parsed = DownloadEnquiry::parse(&outer).unwrap();
        assert_eq!(parsed, enq);
        // And the encapsulated message parses back.
        assert_eq!(
            DownloadDataRequest::parse(parsed.dsmcc_message).unwrap(),
            inner
        );
    }

    #[test]
    fn dispatch_routes_each_tag() {
        let enq = DownloadEnquiry {
            length_form: crate::length::LengthForm::Minimal,
            dsmcc_message: &[0x11],
        }
        .try_to_bytes()
        .unwrap();
        assert!(matches!(
            DownloadApdu::parse(&enq).unwrap(),
            DownloadApdu::DownloadEnquiry(_)
        ));
        let uar = UserAuthResult {
            length_form: crate::length::LengthForm::Minimal,
            binary_id: BinaryId::default(),
            result: &[0x01],
        }
        .try_to_bytes()
        .unwrap();
        let parsed = DownloadApdu::parse(&uar).unwrap();
        assert!(matches!(parsed, DownloadApdu::UserAuthResult(_)));
        assert_eq!(parsed.try_to_bytes().unwrap(), uar);
    }

    #[test]
    fn oversized_block_data_wraps_message_length_before_fix_rejected_after() {
        // A 65 536-byte block_data body makes messageLength (the u16
        // DSM-CC field) exceed 0xFFFF; before the fix this wrapped and
        // returned Ok.
        let big = alloc::vec![0u8; 65_536];
        let block = DownloadDataBlock {
            download_id: 1,
            adaptation: &[],
            module_id: 1,
            module_version: 0,
            block_number: 0,
            block_data: &big,
        };
        let mut buf = alloc::vec![0u8; 65_600];
        assert!(matches!(
            block.serialize_into(&mut buf),
            Err(Error::InvalidObject { .. })
        ));
    }

    #[test]
    fn max_message_length_still_serializes_and_round_trips() {
        // messageLength = 12 (fixed part after the length field) + block_data.
        let data = alloc::vec![0xABu8; 0xFFFF - 12 - 6];
        let block = DownloadDataBlock {
            download_id: 1,
            adaptation: &[],
            module_id: 1,
            module_version: 0,
            block_number: 0,
            block_data: &data,
        };
        let bytes = block.try_to_bytes().unwrap();
        assert_eq!(DownloadDataBlock::parse(&bytes).unwrap(), block);
    }

    #[test]
    fn oversized_adaptation_is_rejected_not_wrapped() {
        // 256 bytes silently wrapped adaptationLength (u8) to 0 before the fix.
        let big = alloc::vec![0xCDu8; 256];
        let compat = [0x00, 0x02, 0x00, 0x00];
        let req = DownloadInfoRequest {
            transaction_id: 1,
            adaptation: &big,
            buffer_size: 1,
            maximum_block_size: 2,
            compatibility_descriptor: &compat,
            private_data: &[],
        };
        let mut buf = alloc::vec![0u8; 512];
        assert!(matches!(
            req.serialize_into(&mut buf),
            Err(Error::InvalidObject { .. })
        ));
    }

    #[test]
    fn oversized_private_data_is_rejected_not_wrapped() {
        // A 65 536-byte private_data body exceeds the 16-bit
        // privateDataLength field.
        let big = alloc::vec![0u8; 65_536];
        let compat = [0x00, 0x02, 0x00, 0x00];
        let req = DownloadInfoRequest {
            transaction_id: 1,
            adaptation: &[],
            buffer_size: 1,
            maximum_block_size: 2,
            compatibility_descriptor: &compat,
            private_data: &big,
        };
        let mut buf = alloc::vec![0u8; 65_600];
        assert!(matches!(
            req.serialize_into(&mut buf),
            Err(Error::InvalidObject { .. })
        ));
    }
}
