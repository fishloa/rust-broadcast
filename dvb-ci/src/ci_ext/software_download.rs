//! Software Download (CAM firmware) objects + DSM-CC download messages — ETSI
//! TS 101 699 V1.1.1 §6.7, Tables 74-83 (PDF pp. 64-74). See
//! `docs/ci_plus/software-download.md`.
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
use crate::tag::ApduTag;
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

impl BinaryId {
    fn read(b: &[u8]) -> Self {
        Self {
            specifier: ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32,
            model: u16::from_be_bytes([b[3], b[4]]),
            version: u16::from_be_bytes([b[5], b[6]]),
        }
    }
    fn write(self, buf: &mut [u8]) {
        buf[0] = (self.specifier >> 16) as u8;
        buf[1] = (self.specifier >> 8) as u8;
        buf[2] = self.specifier as u8;
        buf[3..5].copy_from_slice(&self.model.to_be_bytes());
        buf[5..7].copy_from_slice(&self.version.to_be_bytes());
    }
}

// =================== APDU objects ===================

/// `download_enq()` (Table 75): host → app — encapsulates one DSM-CC U-N
/// message, carried verbatim as a borrowed byte string.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DownloadEnquiry<'a> {
    /// The encapsulated DSM-CC message bytes (the `DSMCC_descriptor()` loop).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub dsmcc_message: &'a [u8],
}

/// `download_reply()` (Table 76): app → host — encapsulates one DSM-CC U-N
/// message, carried verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DownloadReply<'a> {
    /// The encapsulated DSM-CC message bytes (the `DSMCC_descriptor()` loop).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub dsmcc_message: &'a [u8],
}

/// `user_authorization_initiate()` (Table 77): host → app.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct UserAuthInitiate<'a> {
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
    /// The 7-byte [`BinaryId`].
    pub binary_id: BinaryId,
    /// `result_byte`s — conveys the user response; meaning defined by the specifier.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub result: &'a [u8],
}

macro_rules! opaque_dsmcc_object {
    ($ty:ident, $tag:expr, $what:literal) => {
        impl<'a> Parse<'a> for $ty<'a> {
            type Error = Error;
            fn parse(bytes: &'a [u8]) -> Result<Self> {
                let body = objects::parse_apdu_header(bytes, $tag, $what)?;
                Ok(Self {
                    dsmcc_message: body,
                })
            }
        }
        impl Serialize for $ty<'_> {
            type Error = Error;
            fn serialized_len(&self) -> usize {
                objects::apdu_len(self.dsmcc_message.len())
            }
            fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                let body_len = self.dsmcc_message.len();
                let pos = objects::write_apdu_header($tag, body_len, buf)?;
                buf[pos..pos + body_len].copy_from_slice(self.dsmcc_message);
                Ok(pos + body_len)
            }
        }
    };
}

opaque_dsmcc_object!(DownloadEnquiry, tag::DOWNLOAD_ENQ, "download_enq");
opaque_dsmcc_object!(DownloadReply, tag::DOWNLOAD_REPLY, "download_reply");

macro_rules! user_auth_object {
    ($ty:ident, $tag:expr, $what:literal, $field:ident) => {
        impl<'a> Parse<'a> for $ty<'a> {
            type Error = Error;
            fn parse(bytes: &'a [u8]) -> Result<Self> {
                let body = objects::parse_apdu_header(bytes, $tag, $what)?;
                if body.len() < BINARY_ID_LEN {
                    return Err(Error::BufferTooShort {
                        need: BINARY_ID_LEN,
                        have: body.len(),
                        what: $what,
                    });
                }
                Ok(Self {
                    binary_id: BinaryId::read(body),
                    $field: &body[BINARY_ID_LEN..],
                })
            }
        }
        impl Serialize for $ty<'_> {
            type Error = Error;
            fn serialized_len(&self) -> usize {
                objects::apdu_len(BINARY_ID_LEN + self.$field.len())
            }
            fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                let body_len = BINARY_ID_LEN + self.$field.len();
                let mut pos = objects::write_apdu_header($tag, body_len, buf)?;
                self.binary_id.write(&mut buf[pos..]);
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
/// 79/80/81): protocolDiscriminator + dsmccType + messageId + transactionId +
/// reserved + adaptationLength + messageLength, plus the optional adaptation
/// bytes (`adaptationType` + data, carried opaque).
fn parse_dsmcc_header<'a>(
    body: &'a [u8],
    what: &'static str,
) -> Result<(u32, u8, &'a [u8], &'a [u8])> {
    // protocolDiscriminator(1) dsmccType(1) messageId(2) transactionId(4)
    // reserved(1) adaptationLength(1) messageLength(2) = 12.
    const HDR: usize = 12;
    if body.len() < HDR {
        return Err(Error::BufferTooShort {
            need: HDR,
            have: body.len(),
            what,
        });
    }
    let transaction_id = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
    let adaptation_length = body[9] as usize;
    if body.len() < HDR + adaptation_length {
        return Err(Error::BufferTooShort {
            need: HDR + adaptation_length,
            have: body.len(),
            what,
        });
    }
    let adaptation = &body[HDR..HDR + adaptation_length];
    let rest = &body[HDR + adaptation_length..];
    Ok((transaction_id, adaptation_length as u8, adaptation, rest))
}

fn write_dsmcc_header(
    buf: &mut [u8],
    message_id: u16,
    transaction_id: u32,
    adaptation: &[u8],
    message_length: usize,
) -> usize {
    buf[0] = DSMCC_PROTOCOL_DISCRIMINATOR;
    buf[1] = DSMCC_TYPE_DOWNLOAD;
    buf[2..4].copy_from_slice(&message_id.to_be_bytes());
    buf[4..8].copy_from_slice(&transaction_id.to_be_bytes());
    buf[8] = 0xFF; // reserved
    buf[9] = adaptation.len() as u8;
    buf[10..12].copy_from_slice(&(message_length as u16).to_be_bytes());
    buf[12..12 + adaptation.len()].copy_from_slice(adaptation);
    12 + adaptation.len()
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
        let (transaction_id, _adapt_len, adaptation, rest) = parse_dsmcc_header(body, what)?;
        // bufferSize(4) maximumBlockSize(2) compatibilityDescriptorLength(2).
        if rest.len() < 8 {
            return Err(Error::BufferTooShort {
                need: 8,
                have: rest.len(),
                what,
            });
        }
        let buffer_size = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]);
        let maximum_block_size = u16::from_be_bytes([rest[4], rest[5]]);
        // compatibilityDescriptorLength counts the bytes *after* the length field
        // (descriptorCount + descriptor loop).
        let compat_len = u16::from_be_bytes([rest[6], rest[7]]) as usize;
        let compat_start = 6; // start of compatibilityDescriptor block (incl. its length field)
        let compat_block_end = compat_start + 2 + compat_len;
        if rest.len() < compat_block_end + 2 {
            return Err(Error::BufferTooShort {
                need: compat_block_end + 2,
                have: rest.len(),
                what,
            });
        }
        let compatibility_descriptor = &rest[compat_start..compat_block_end];
        let priv_len =
            u16::from_be_bytes([rest[compat_block_end], rest[compat_block_end + 1]]) as usize;
        let priv_start = compat_block_end + 2;
        let priv_end = priv_start + priv_len;
        if rest.len() < priv_end {
            return Err(Error::BufferTooShort {
                need: priv_end,
                have: rest.len(),
                what,
            });
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
        12 + self.adaptation.len()
            + 6
            + self.compatibility_descriptor.len()
            + 2
            + self.private_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        // messageLength = everything after the messageLength field itself.
        let message_length = total - 12;
        let mut pos = write_dsmcc_header(
            buf,
            MSG_ID_DOWNLOAD_INFO_REQUEST,
            self.transaction_id,
            self.adaptation,
            message_length,
        );
        buf[pos..pos + 4].copy_from_slice(&self.buffer_size.to_be_bytes());
        pos += 4;
        buf[pos..pos + 2].copy_from_slice(&self.maximum_block_size.to_be_bytes());
        pos += 2;
        buf[pos..pos + self.compatibility_descriptor.len()]
            .copy_from_slice(self.compatibility_descriptor);
        pos += self.compatibility_descriptor.len();
        buf[pos..pos + 2].copy_from_slice(&(self.private_data.len() as u16).to_be_bytes());
        pos += 2;
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
        let (transaction_id, _adapt_len, adaptation, rest) = parse_dsmcc_header(body, what)?;
        // downloadId(4) blockSize(2) windowSize(1) ackPeriod(1)
        // tCDownloadWindow(4) tCDownloadScenario(4) compatibilityDescriptorLength(2).
        const FIXED: usize = 4 + 2 + 1 + 1 + 4 + 4 + 2;
        if rest.len() < FIXED {
            return Err(Error::BufferTooShort {
                need: FIXED,
                have: rest.len(),
                what,
            });
        }
        let download_id = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]);
        let block_size = u16::from_be_bytes([rest[4], rest[5]]);
        let window_size = rest[6];
        let ack_period = rest[7];
        let tc_download_window = u32::from_be_bytes([rest[8], rest[9], rest[10], rest[11]]);
        let tc_download_scenario = u32::from_be_bytes([rest[12], rest[13], rest[14], rest[15]]);
        let compat_len_off = 16;
        let compat_len =
            u16::from_be_bytes([rest[compat_len_off], rest[compat_len_off + 1]]) as usize;
        let compat_block_end = compat_len_off + 2 + compat_len;
        // + numberOfModules(2) at compat_block_end.
        if rest.len() < compat_block_end + 2 {
            return Err(Error::BufferTooShort {
                need: compat_block_end + 2,
                have: rest.len(),
                what,
            });
        }
        let compatibility_descriptor = &rest[compat_len_off..compat_block_end];
        // Walk the module loop to find where private data begins.
        let number_of_modules =
            u16::from_be_bytes([rest[compat_block_end], rest[compat_block_end + 1]]) as usize;
        let mut mpos = compat_block_end + 2;
        for _ in 0..number_of_modules {
            // moduleId(2) moduleSize(4) moduleVersion(1) moduleInfoLength(1).
            if rest.len() < mpos + 8 {
                return Err(Error::BufferTooShort {
                    need: mpos + 8,
                    have: rest.len(),
                    what,
                });
            }
            let module_info_len = rest[mpos + 7] as usize;
            mpos += 8 + module_info_len;
            if rest.len() < mpos {
                return Err(Error::BufferTooShort {
                    need: mpos,
                    have: rest.len(),
                    what,
                });
            }
        }
        let modules = &rest[compat_block_end..mpos];
        if rest.len() < mpos + 2 {
            return Err(Error::BufferTooShort {
                need: mpos + 2,
                have: rest.len(),
                what,
            });
        }
        let priv_len = u16::from_be_bytes([rest[mpos], rest[mpos + 1]]) as usize;
        let priv_start = mpos + 2;
        let priv_end = priv_start + priv_len;
        if rest.len() < priv_end {
            return Err(Error::BufferTooShort {
                need: priv_end,
                have: rest.len(),
                what,
            });
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
        12 + self.adaptation.len()
            + 16
            + self.compatibility_descriptor.len()
            + self.modules.len()
            + 2
            + self.private_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        let message_length = total - 12;
        let mut pos = write_dsmcc_header(
            buf,
            MSG_ID_DOWNLOAD_INFO_RESPONSE,
            self.transaction_id,
            self.adaptation,
            message_length,
        );
        buf[pos..pos + 4].copy_from_slice(&self.download_id.to_be_bytes());
        pos += 4;
        buf[pos..pos + 2].copy_from_slice(&self.block_size.to_be_bytes());
        pos += 2;
        buf[pos] = self.window_size;
        buf[pos + 1] = self.ack_period;
        pos += 2;
        buf[pos..pos + 4].copy_from_slice(&self.tc_download_window.to_be_bytes());
        pos += 4;
        buf[pos..pos + 4].copy_from_slice(&self.tc_download_scenario.to_be_bytes());
        pos += 4;
        buf[pos..pos + self.compatibility_descriptor.len()]
            .copy_from_slice(self.compatibility_descriptor);
        pos += self.compatibility_descriptor.len();
        buf[pos..pos + self.modules.len()].copy_from_slice(self.modules);
        pos += self.modules.len();
        buf[pos..pos + 2].copy_from_slice(&(self.private_data.len() as u16).to_be_bytes());
        pos += 2;
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
        let (transaction_id, _adapt_len, adaptation, rest) = parse_dsmcc_header(body, what)?;
        // downloadId(4) moduleId(2) blockNumber(2) downloadCancelReason(1) privateDataLength(2).
        const FIXED: usize = 4 + 2 + 2 + 1 + 2;
        if rest.len() < FIXED {
            return Err(Error::BufferTooShort {
                need: FIXED,
                have: rest.len(),
                what,
            });
        }
        let download_id = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]);
        let module_id = u16::from_be_bytes([rest[4], rest[5]]);
        let block_number = u16::from_be_bytes([rest[6], rest[7]]);
        let download_cancel_reason = rest[8];
        let priv_len = u16::from_be_bytes([rest[9], rest[10]]) as usize;
        let priv_start = 11;
        let priv_end = priv_start + priv_len;
        if rest.len() < priv_end {
            return Err(Error::BufferTooShort {
                need: priv_end,
                have: rest.len(),
                what,
            });
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
        12 + self.adaptation.len() + 9 + 2 + self.private_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        let message_length = total - 12;
        let mut pos = write_dsmcc_header(
            buf,
            MSG_ID_DOWNLOAD_CANCEL,
            self.transaction_id,
            self.adaptation,
            message_length,
        );
        buf[pos..pos + 4].copy_from_slice(&self.download_id.to_be_bytes());
        pos += 4;
        buf[pos..pos + 2].copy_from_slice(&self.module_id.to_be_bytes());
        pos += 2;
        buf[pos..pos + 2].copy_from_slice(&self.block_number.to_be_bytes());
        pos += 2;
        buf[pos] = self.download_cancel_reason;
        pos += 1;
        buf[pos..pos + 2].copy_from_slice(&(self.private_data.len() as u16).to_be_bytes());
        pos += 2;
        buf[pos..pos + self.private_data.len()].copy_from_slice(self.private_data);
        Ok(pos + self.private_data.len())
    }
}

/// The DSM-CC `dsmccDownloadDataHeader` (Tables 82/83) differs from the message
/// header: the `DownloadId` (4) precedes `reserved`/`adaptationLength`/
/// `messageLength`. Returns `(download_id, adaptation, rest_after_adaptation)`.
fn parse_dsmcc_data_header<'a>(
    body: &'a [u8],
    what: &'static str,
) -> Result<(u32, &'a [u8], &'a [u8])> {
    // protocolDiscriminator(1) dsmccType(1) messageId(2) DownloadId(4)
    // reserved(1) adaptationLength(1) messageLength(2) = 12.
    const HDR: usize = 12;
    if body.len() < HDR {
        return Err(Error::BufferTooShort {
            need: HDR,
            have: body.len(),
            what,
        });
    }
    let download_id = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
    let adaptation_length = body[9] as usize;
    if body.len() < HDR + adaptation_length {
        return Err(Error::BufferTooShort {
            need: HDR + adaptation_length,
            have: body.len(),
            what,
        });
    }
    let adaptation = &body[HDR..HDR + adaptation_length];
    Ok((download_id, adaptation, &body[HDR + adaptation_length..]))
}

fn write_dsmcc_data_header(
    buf: &mut [u8],
    message_id: u16,
    download_id: u32,
    adaptation: &[u8],
    message_length: usize,
) -> usize {
    buf[0] = DSMCC_PROTOCOL_DISCRIMINATOR;
    buf[1] = DSMCC_TYPE_DOWNLOAD;
    buf[2..4].copy_from_slice(&message_id.to_be_bytes());
    buf[4..8].copy_from_slice(&download_id.to_be_bytes());
    buf[8] = 0xFF; // reserved
    buf[9] = adaptation.len() as u8;
    buf[10..12].copy_from_slice(&(message_length as u16).to_be_bytes());
    buf[12..12 + adaptation.len()].copy_from_slice(adaptation);
    12 + adaptation.len()
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
        let (download_id, adaptation, rest) = parse_dsmcc_data_header(body, what)?;
        // moduleId(2) blockNumber(2) downloadReason(1).
        const FIXED: usize = 5;
        if rest.len() < FIXED {
            return Err(Error::BufferTooShort {
                need: FIXED,
                have: rest.len(),
                what,
            });
        }
        Ok(Self {
            download_id,
            adaptation,
            module_id: u16::from_be_bytes([rest[0], rest[1]]),
            block_number: u16::from_be_bytes([rest[2], rest[3]]),
            download_reason: rest[4],
        })
    }
}
impl Serialize for DownloadDataRequest<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        12 + self.adaptation.len() + 5
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        let message_length = total - 12;
        let mut pos = write_dsmcc_data_header(
            buf,
            MSG_ID_DOWNLOAD_DATA_REQUEST,
            self.download_id,
            self.adaptation,
            message_length,
        );
        buf[pos..pos + 2].copy_from_slice(&self.module_id.to_be_bytes());
        pos += 2;
        buf[pos..pos + 2].copy_from_slice(&self.block_number.to_be_bytes());
        pos += 2;
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
        let (download_id, adaptation, rest) = parse_dsmcc_data_header(body, what)?;
        // moduleId(2) moduleVersion(1) reserved(1) blockNumber(2).
        const FIXED: usize = 6;
        if rest.len() < FIXED {
            return Err(Error::BufferTooShort {
                need: FIXED,
                have: rest.len(),
                what,
            });
        }
        Ok(Self {
            download_id,
            adaptation,
            module_id: u16::from_be_bytes([rest[0], rest[1]]),
            module_version: rest[2],
            // rest[3] = reserved
            block_number: u16::from_be_bytes([rest[4], rest[5]]),
            block_data: &rest[FIXED..],
        })
    }
}
impl Serialize for DownloadDataBlock<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        12 + self.adaptation.len() + 6 + self.block_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        let message_length = total - 12;
        let mut pos = write_dsmcc_data_header(
            buf,
            MSG_ID_DOWNLOAD_DATA_BLOCK,
            self.download_id,
            self.adaptation,
            message_length,
        );
        buf[pos..pos + 2].copy_from_slice(&self.module_id.to_be_bytes());
        pos += 2;
        buf[pos] = self.module_version;
        buf[pos + 1] = 0xFF; // reserved
        pos += 2;
        buf[pos..pos + 2].copy_from_slice(&self.block_number.to_be_bytes());
        pos += 2;
        buf[pos..pos + self.block_data.len()].copy_from_slice(self.block_data);
        Ok(pos + self.block_data.len())
    }
}

/// Resource-scoped dispatch over the Download APDU objects (Tables 75-78).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DownloadApdu<'a> {
    /// `download_enq` (`9F 80 00`).
    DownloadEnquiry(DownloadEnquiry<'a>),
    /// `download_reply` (`9F 80 01`).
    DownloadReply(DownloadReply<'a>),
    /// `user_authorization_initiate` (`9F 80 02`).
    UserAuthInitiate(UserAuthInitiate<'a>),
    /// `user_authorization_result` (`9F 80 03`).
    UserAuthResult(UserAuthResult<'a>),
}

impl<'a> DownloadApdu<'a> {
    /// Parse a Download APDU, dispatching on the leading `apdu_tag`.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "download apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            tag::DOWNLOAD_ENQ => Ok(Self::DownloadEnquiry(DownloadEnquiry::parse(body)?)),
            tag::DOWNLOAD_REPLY => Ok(Self::DownloadReply(DownloadReply::parse(body)?)),
            tag::USER_AUTH_INITIATE => Ok(Self::UserAuthInitiate(UserAuthInitiate::parse(body)?)),
            tag::USER_AUTH_RESULT => Ok(Self::UserAuthResult(UserAuthResult::parse(body)?)),
            _ => Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: tag::DOWNLOAD_ENQ.as_u24(),
                what: "download",
            }),
        }
    }
}

impl Serialize for DownloadApdu<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::DownloadEnquiry(o) => o.serialized_len(),
            Self::DownloadReply(o) => o.serialized_len(),
            Self::UserAuthInitiate(o) => o.serialized_len(),
            Self::UserAuthResult(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::DownloadEnquiry(o) => o.serialize_into(buf),
            Self::DownloadReply(o) => o.serialize_into(buf),
            Self::UserAuthInitiate(o) => o.serialize_into(buf),
            Self::UserAuthResult(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_enquiry_round_trips_and_bites() {
        let enq = DownloadEnquiry {
            dsmcc_message: &[0x11, 0x03, 0x10, 0x01],
        };
        let bytes = enq.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x00, 0x04, 0x11, 0x03, 0x10, 0x01]);
        assert_eq!(DownloadEnquiry::parse(&bytes).unwrap(), enq);
        let other = DownloadEnquiry {
            dsmcc_message: &[0x11, 0x03, 0x10, 0x02],
        };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn download_reply_round_trips() {
        let rep = DownloadReply {
            dsmcc_message: &[0xAA],
        };
        let bytes = rep.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x01, 0x01, 0xAA]);
        assert_eq!(DownloadReply::parse(&bytes).unwrap(), rep);
    }

    #[test]
    fn user_auth_initiate_round_trips_and_bites() {
        let uai = UserAuthInitiate {
            binary_id: BinaryId {
                specifier: 0x00_1B_67,
                model: 0x1234,
                version: 0x0005,
            },
            data: &[0xCA, 0xFE],
        };
        let bytes = uai.to_bytes();
        // tag(3) + len(1=9) + specifier(3) model(2) version(2) data(2).
        assert_eq!(
            bytes,
            [0x9F, 0x80, 0x02, 0x09, 0x00, 0x1B, 0x67, 0x12, 0x34, 0x00, 0x05, 0xCA, 0xFE]
        );
        assert_eq!(UserAuthInitiate::parse(&bytes).unwrap(), uai);
        let mut other = uai.clone();
        other.binary_id.version = 0x0006;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn user_auth_result_round_trips() {
        let uar = UserAuthResult {
            binary_id: BinaryId {
                specifier: 0xAABBCC & 0x00FF_FFFF,
                model: 1,
                version: 2,
            },
            result: &[0x01],
        };
        let bytes = uar.to_bytes();
        assert_eq!(
            bytes,
            [0x9F, 0x80, 0x03, 0x08, 0xAA, 0xBB, 0xCC, 0x00, 0x01, 0x00, 0x02, 0x01]
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
        let bytes = req.to_bytes();
        assert_eq!(DownloadInfoRequest::parse(&bytes).unwrap(), req);
        // Verify header is well-formed.
        assert_eq!(bytes[0], 0x11);
        assert_eq!(bytes[1], 0x03);
        assert_eq!(&bytes[2..4], &[0x10, 0x01]);
        let mut other = req.clone();
        other.buffer_size = 0x0002_0000;
        assert_ne!(bytes, other.to_bytes());
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
        let bytes = req.to_bytes();
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
        let bytes = resp.to_bytes();
        assert_eq!(DownloadInfoResponse::parse(&bytes).unwrap(), resp);
        let mut other = resp.clone();
        other.window_size = 5;
        assert_ne!(bytes, other.to_bytes());
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
        let bytes = cancel.to_bytes();
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
        let bytes = req.to_bytes();
        assert_eq!(DownloadDataRequest::parse(&bytes).unwrap(), req);
        // messageId DownloadDataRequest, then DownloadId (the data header puts it
        // at offset 4, before reserved/adaptation/messageLength).
        assert_eq!(&bytes[2..4], &[0x10, 0x04]);
        assert_eq!(&bytes[4..8], &0xDEAD_BEEFu32.to_be_bytes());
        let mut other = req.clone();
        other.block_number = 0x0003;
        assert_ne!(bytes, other.to_bytes());
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
        let bytes = block.to_bytes();
        assert_eq!(DownloadDataBlock::parse(&bytes).unwrap(), block);
        assert_eq!(&bytes[2..4], &[0x10, 0x03]); // messageId DownloadDataBlock
        let mut other = block.clone();
        other.block_data = &[0xFE, 0xED, 0xFA, 0xCF];
        assert_ne!(bytes, other.to_bytes());
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
        let inner_bytes = inner.to_bytes();
        let enq = DownloadEnquiry {
            dsmcc_message: &inner_bytes,
        };
        let outer = enq.to_bytes();
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
            dsmcc_message: &[0x11],
        }
        .to_bytes();
        assert!(matches!(
            DownloadApdu::parse(&enq).unwrap(),
            DownloadApdu::DownloadEnquiry(_)
        ));
        let uar = UserAuthResult {
            binary_id: BinaryId::default(),
            result: &[0x01],
        }
        .to_bytes();
        let parsed = DownloadApdu::parse(&uar).unwrap();
        assert!(matches!(parsed, DownloadApdu::UserAuthResult(_)));
        assert_eq!(parsed.to_bytes(), uar);
    }
}
