//! DSM-CC U-N download protocol messages — ISO/IEC 13818-6 §7.2/§7.3.
//!
//! Layouts per `docs/iso_13818_6_carousel.md` (hand-transcribed; ISO/IEC
//! 13818-6 is not freely redistributable), cross-checked against the vendored
//! TR 101 202 §4.6/§4.7.5 + TS 102 006 Table 15, and pinned live against the
//! `m6-single.ts` capture by the `carousel_fixture` integration test.
//!
//! Control messages (DSI/DII) are the payload of DSM-CC sections with
//! table_id 0x3B; data messages (DDB) of table_id 0x3C — see
//! [`crate::tables::dsmcc`] for the section framing.

use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// `protocolDiscriminator` — always 0x11 for MPEG-2 DSM-CC.
pub const PROTOCOL_DISCRIMINATOR: u8 = 0x11;
/// `dsmccType` for U-N download messages (§7.2: 0x03).
pub const DSMCC_TYPE_UN_DOWNLOAD: u8 = 0x03;
/// `messageId` of DownloadInfoIndication.
pub const MESSAGE_ID_DII: u16 = 0x1002;
/// `messageId` of DownloadDataBlock.
pub const MESSAGE_ID_DDB: u16 = 0x1003;
/// `messageId` of DownloadServerInitiate.
pub const MESSAGE_ID_DSI: u16 = 0x1006;

/// Bytes of dsmccMessageHeader / dsmccDownloadDataHeader before the
/// adaptation header: pd(1) + type(1) + messageId(2) + transactionId-or-
/// downloadId(4) + reserved(1) + adaptationLength(1) + messageLength(2).
const MESSAGE_HEADER_LEN: usize = 12;
/// serverId is a fixed 20-byte field in the DSI (DVB: all 0xFF).
const SERVER_ID_LEN: usize = 20;
/// 16-bit compatibilityDescriptorLength field.
const COMPAT_LEN_FIELD: usize = 2;
/// 16-bit privateDataLength field.
const PRIVATE_LEN_FIELD: usize = 2;
/// Fixed DII body bytes before the compatibilityDescriptor: downloadId(4) +
/// blockSize(2) + windowSize(1) + ackPeriod(1) + tCDownloadWindow(4) +
/// tCDownloadScenario(4).
const DII_FIXED_LEN: usize = 16;
/// Per-module fixed bytes: moduleId(2) + moduleSize(4) + moduleVersion(1) +
/// moduleInfoLength(1).
const MODULE_HEADER_LEN: usize = 8;
/// DDB body bytes before blockData: moduleId(2) + moduleVersion(1) +
/// reserved(1) + blockNumber(2).
const DDB_FIXED_LEN: usize = 6;

/// DownloadServerInitiate (§7.3.6, messageId 0x1006).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Dsi<'a> {
    /// 32-bit transactionId. DVB (TR 101 202 §4.7.9): the 2 LSBs are 0x0000
    /// for a DSI; bit 31 toggles on update.
    pub transaction_id: u32,
    /// Raw dsmccAdaptationHeader bytes (usually empty).
    pub adaptation: &'a [u8],
    /// 20-byte serverId — all 0xFF under the DVB profile.
    pub server_id: [u8; SERVER_ID_LEN],
    /// compatibilityDescriptor() body after its 16-bit length field, raw
    /// (TS 102 006 Table 15 documents the structure).
    pub compatibility_descriptor: &'a [u8],
    /// privateData, raw. SSU: GroupInfoIndication (TS 102 006 Table 6);
    /// object carousel: ServiceGatewayInfo (TR 101 202 Table 4.15).
    pub private_data: &'a [u8],
}

/// One module entry in a DII (§7.3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DiiModule<'a> {
    /// moduleId referenced by DDB messages.
    pub module_id: u16,
    /// Total module size in bytes.
    pub module_size: u32,
    /// moduleVersion; DDBs must match.
    pub module_version: u8,
    /// moduleInfo, raw (object carousel: BIOP::ModuleInfo, TR 101 202
    /// Table 4.14).
    pub module_info: &'a [u8],
}

/// DownloadInfoIndication (§7.3.3, messageId 0x1002).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Dii<'a> {
    /// 32-bit transactionId (TR 101 202 Table 4.1 sub-fields).
    pub transaction_id: u32,
    /// Raw dsmccAdaptationHeader bytes (usually empty).
    pub adaptation: &'a [u8],
    /// downloadId — links this DII to its DDB messages.
    pub download_id: u32,
    /// Bytes per DDB block (every block except possibly the last).
    pub block_size: u16,
    /// windowSize — 0 under the DVB profile.
    pub window_size: u8,
    /// ackPeriod — 0 under the DVB profile.
    pub ack_period: u8,
    /// tCDownloadWindow — 0 under the DVB profile.
    pub t_c_download_window: u32,
    /// tCDownloadScenario.
    pub t_c_download_scenario: u32,
    /// compatibilityDescriptor() body after its 16-bit length field, raw.
    pub compatibility_descriptor: &'a [u8],
    /// Module entries in wire order.
    pub modules: Vec<DiiModule<'a>>,
    /// privateData, raw.
    pub private_data: &'a [u8],
}

/// A U-N download control message — payload of a table_id 0x3B DSM-CC
/// section, discriminated by `messageId`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum UnMessage<'a> {
    /// DownloadServerInitiate (messageId 0x1006).
    Dsi(Dsi<'a>),
    /// DownloadInfoIndication (messageId 0x1002).
    Dii(Dii<'a>),
}

/// DownloadDataBlock (§7.3.7.1, messageId 0x1003) — payload of a table_id
/// 0x3C DSM-CC section, including its dsmccDownloadDataHeader.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DownloadDataBlock<'a> {
    /// downloadId from the dsmccDownloadDataHeader — matches the DII.
    pub download_id: u32,
    /// Raw dsmccAdaptationHeader bytes (usually empty).
    pub adaptation: &'a [u8],
    /// moduleId of the module this block belongs to.
    pub module_id: u16,
    /// moduleVersion — must match the DII module entry.
    pub module_version: u8,
    /// Block index; byte offset within the module = blockNumber × blockSize.
    pub block_number: u16,
    /// The block payload.
    pub block_data: &'a [u8],
}

/// Parse the 12-byte dsmccMessageHeader / dsmccDownloadDataHeader common
/// shape. Returns (messageId, transaction_or_download_id, adaptation,
/// payload) where `payload` is bounded by `messageLength`.
fn parse_header<'a>(bytes: &'a [u8], what: &'static str) -> Result<(u16, u32, &'a [u8], &'a [u8])> {
    if bytes.len() < MESSAGE_HEADER_LEN {
        return Err(Error::BufferTooShort {
            need: MESSAGE_HEADER_LEN,
            have: bytes.len(),
            what,
        });
    }
    if bytes[0] != PROTOCOL_DISCRIMINATOR {
        return Err(Error::ReservedBitsViolation {
            field: "protocolDiscriminator",
            reason: "must be 0x11 (ISO/IEC 13818-6 §7.2)",
        });
    }
    if bytes[1] != DSMCC_TYPE_UN_DOWNLOAD {
        return Err(Error::ReservedBitsViolation {
            field: "dsmccType",
            reason: "must be 0x03 — U-N download (ISO/IEC 13818-6 §7.2)",
        });
    }
    let message_id = u16::from_be_bytes([bytes[2], bytes[3]]);
    let id = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let adaptation_length = bytes[9] as usize;
    let message_length = u16::from_be_bytes([bytes[10], bytes[11]]) as usize;
    let total = MESSAGE_HEADER_LEN + message_length;
    if bytes.len() < total {
        return Err(Error::SectionLengthOverflow {
            declared: message_length,
            available: bytes.len() - MESSAGE_HEADER_LEN,
        });
    }
    if adaptation_length > message_length {
        return Err(Error::SectionLengthOverflow {
            declared: adaptation_length,
            available: message_length,
        });
    }
    let adaptation = &bytes[MESSAGE_HEADER_LEN..MESSAGE_HEADER_LEN + adaptation_length];
    let payload = &bytes[MESSAGE_HEADER_LEN + adaptation_length..total];
    Ok((message_id, id, adaptation, payload))
}

/// Serialize the common 12-byte header followed by the adaptation bytes.
/// `payload_len` is the body length after the adaptation header.
fn serialize_header(
    buf: &mut [u8],
    message_id: u16,
    id: u32,
    adaptation: &[u8],
    payload_len: usize,
) -> Result<usize> {
    let message_length = adaptation.len() + payload_len;
    if adaptation.len() > u8::MAX as usize {
        return Err(Error::SectionLengthOverflow {
            declared: adaptation.len(),
            available: u8::MAX as usize,
        });
    }
    if message_length > u16::MAX as usize {
        return Err(Error::SectionLengthOverflow {
            declared: message_length,
            available: u16::MAX as usize,
        });
    }
    buf[0] = PROTOCOL_DISCRIMINATOR;
    buf[1] = DSMCC_TYPE_UN_DOWNLOAD;
    buf[2..4].copy_from_slice(&message_id.to_be_bytes());
    buf[4..8].copy_from_slice(&id.to_be_bytes());
    buf[8] = 0xFF; // reserved
    buf[9] = adaptation.len() as u8;
    buf[10..12].copy_from_slice(&(message_length as u16).to_be_bytes());
    buf[MESSAGE_HEADER_LEN..MESSAGE_HEADER_LEN + adaptation.len()].copy_from_slice(adaptation);
    Ok(MESSAGE_HEADER_LEN + adaptation.len())
}

/// Read a 16-bit-length-prefixed slice at `pos`, bounds-checked against `end`.
fn length_prefixed(bytes: &[u8], pos: usize, end: usize) -> Result<(&[u8], usize)> {
    if pos + 2 > end {
        return Err(Error::BufferTooShort {
            need: pos + 2,
            have: end,
            what: "DSM-CC 16-bit length field",
        });
    }
    let len = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as usize;
    let start = pos + 2;
    if start + len > end {
        return Err(Error::SectionLengthOverflow {
            declared: len,
            available: end - start,
        });
    }
    Ok((&bytes[start..start + len], start + len))
}

impl<'a> Parse<'a> for UnMessage<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (message_id, transaction_id, adaptation, payload) =
            parse_header(bytes, "UnMessage header")?;
        let end = payload.len();
        match message_id {
            MESSAGE_ID_DSI => {
                if end < SERVER_ID_LEN + COMPAT_LEN_FIELD + PRIVATE_LEN_FIELD {
                    return Err(Error::BufferTooShort {
                        need: SERVER_ID_LEN + COMPAT_LEN_FIELD + PRIVATE_LEN_FIELD,
                        have: end,
                        what: "Dsi body",
                    });
                }
                let mut server_id = [0u8; SERVER_ID_LEN];
                server_id.copy_from_slice(&payload[..SERVER_ID_LEN]);
                let (compatibility_descriptor, pos) = length_prefixed(payload, SERVER_ID_LEN, end)?;
                let (private_data, _pos) = length_prefixed(payload, pos, end)?;
                Ok(UnMessage::Dsi(Dsi {
                    transaction_id,
                    adaptation,
                    server_id,
                    compatibility_descriptor,
                    private_data,
                }))
            }
            MESSAGE_ID_DII => {
                if end < DII_FIXED_LEN + COMPAT_LEN_FIELD {
                    return Err(Error::BufferTooShort {
                        need: DII_FIXED_LEN + COMPAT_LEN_FIELD,
                        have: end,
                        what: "Dii body",
                    });
                }
                let download_id =
                    u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let block_size = u16::from_be_bytes([payload[4], payload[5]]);
                let window_size = payload[6];
                let ack_period = payload[7];
                let t_c_download_window =
                    u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]);
                let t_c_download_scenario =
                    u32::from_be_bytes([payload[12], payload[13], payload[14], payload[15]]);
                let (compatibility_descriptor, mut pos) =
                    length_prefixed(payload, DII_FIXED_LEN, end)?;
                if pos + 2 > end {
                    return Err(Error::BufferTooShort {
                        need: pos + 2,
                        have: end,
                        what: "Dii numberOfModules",
                    });
                }
                let number_of_modules =
                    u16::from_be_bytes([payload[pos], payload[pos + 1]]) as usize;
                pos += 2;
                let mut modules = Vec::with_capacity(number_of_modules.min(256));
                for _ in 0..number_of_modules {
                    if pos + MODULE_HEADER_LEN > end {
                        return Err(Error::BufferTooShort {
                            need: pos + MODULE_HEADER_LEN,
                            have: end,
                            what: "Dii module entry",
                        });
                    }
                    let module_id = u16::from_be_bytes([payload[pos], payload[pos + 1]]);
                    let module_size = u32::from_be_bytes([
                        payload[pos + 2],
                        payload[pos + 3],
                        payload[pos + 4],
                        payload[pos + 5],
                    ]);
                    let module_version = payload[pos + 6];
                    let module_info_length = payload[pos + 7] as usize;
                    let info_start = pos + MODULE_HEADER_LEN;
                    if info_start + module_info_length > end {
                        return Err(Error::SectionLengthOverflow {
                            declared: module_info_length,
                            available: end - info_start,
                        });
                    }
                    modules.push(DiiModule {
                        module_id,
                        module_size,
                        module_version,
                        module_info: &payload[info_start..info_start + module_info_length],
                    });
                    pos = info_start + module_info_length;
                }
                let (private_data, _pos) = length_prefixed(payload, pos, end)?;
                Ok(UnMessage::Dii(Dii {
                    transaction_id,
                    adaptation,
                    download_id,
                    block_size,
                    window_size,
                    ack_period,
                    t_c_download_window,
                    t_c_download_scenario,
                    compatibility_descriptor,
                    modules,
                    private_data,
                }))
            }
            _ => Err(Error::ReservedBitsViolation {
                field: "messageId",
                reason: "expected 0x1002 (DII) or 0x1006 (DSI) on table_id 0x3B \
                         (ISO/IEC 13818-6 §7.3)",
            }),
        }
    }
}

impl Serialize for UnMessage<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        match self {
            UnMessage::Dsi(dsi) => {
                MESSAGE_HEADER_LEN
                    + dsi.adaptation.len()
                    + SERVER_ID_LEN
                    + COMPAT_LEN_FIELD
                    + dsi.compatibility_descriptor.len()
                    + PRIVATE_LEN_FIELD
                    + dsi.private_data.len()
            }
            UnMessage::Dii(dii) => {
                MESSAGE_HEADER_LEN
                    + dii.adaptation.len()
                    + DII_FIXED_LEN
                    + COMPAT_LEN_FIELD
                    + dii.compatibility_descriptor.len()
                    + 2 // numberOfModules
                    + dii
                        .modules
                        .iter()
                        .map(|m| MODULE_HEADER_LEN + m.module_info.len())
                        .sum::<usize>()
                    + PRIVATE_LEN_FIELD
                    + dii.private_data.len()
            }
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        match self {
            UnMessage::Dsi(dsi) => {
                let payload_len = len - MESSAGE_HEADER_LEN - dsi.adaptation.len();
                let mut pos = serialize_header(
                    buf,
                    MESSAGE_ID_DSI,
                    dsi.transaction_id,
                    dsi.adaptation,
                    payload_len,
                )?;
                buf[pos..pos + SERVER_ID_LEN].copy_from_slice(&dsi.server_id);
                pos += SERVER_ID_LEN;
                pos = put_length_prefixed(buf, pos, dsi.compatibility_descriptor)?;
                put_length_prefixed(buf, pos, dsi.private_data)?;
            }
            UnMessage::Dii(dii) => {
                let payload_len = len - MESSAGE_HEADER_LEN - dii.adaptation.len();
                let mut pos = serialize_header(
                    buf,
                    MESSAGE_ID_DII,
                    dii.transaction_id,
                    dii.adaptation,
                    payload_len,
                )?;
                buf[pos..pos + 4].copy_from_slice(&dii.download_id.to_be_bytes());
                buf[pos + 4..pos + 6].copy_from_slice(&dii.block_size.to_be_bytes());
                buf[pos + 6] = dii.window_size;
                buf[pos + 7] = dii.ack_period;
                buf[pos + 8..pos + 12].copy_from_slice(&dii.t_c_download_window.to_be_bytes());
                buf[pos + 12..pos + 16].copy_from_slice(&dii.t_c_download_scenario.to_be_bytes());
                pos += DII_FIXED_LEN;
                pos = put_length_prefixed(buf, pos, dii.compatibility_descriptor)?;
                if dii.modules.len() > u16::MAX as usize {
                    return Err(Error::SectionLengthOverflow {
                        declared: dii.modules.len(),
                        available: u16::MAX as usize,
                    });
                }
                buf[pos..pos + 2].copy_from_slice(&(dii.modules.len() as u16).to_be_bytes());
                pos += 2;
                for m in &dii.modules {
                    if m.module_info.len() > u8::MAX as usize {
                        return Err(Error::SectionLengthOverflow {
                            declared: m.module_info.len(),
                            available: u8::MAX as usize,
                        });
                    }
                    buf[pos..pos + 2].copy_from_slice(&m.module_id.to_be_bytes());
                    buf[pos + 2..pos + 6].copy_from_slice(&m.module_size.to_be_bytes());
                    buf[pos + 6] = m.module_version;
                    buf[pos + 7] = m.module_info.len() as u8;
                    pos += MODULE_HEADER_LEN;
                    buf[pos..pos + m.module_info.len()].copy_from_slice(m.module_info);
                    pos += m.module_info.len();
                }
                put_length_prefixed(buf, pos, dii.private_data)?;
            }
        }
        Ok(len)
    }
}

/// Write a 16-bit length then the slice; returns the new position.
fn put_length_prefixed(buf: &mut [u8], pos: usize, data: &[u8]) -> Result<usize> {
    if data.len() > u16::MAX as usize {
        return Err(Error::SectionLengthOverflow {
            declared: data.len(),
            available: u16::MAX as usize,
        });
    }
    buf[pos..pos + 2].copy_from_slice(&(data.len() as u16).to_be_bytes());
    buf[pos + 2..pos + 2 + data.len()].copy_from_slice(data);
    Ok(pos + 2 + data.len())
}

impl<'a> Parse<'a> for DownloadDataBlock<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (message_id, download_id, adaptation, payload) =
            parse_header(bytes, "DownloadDataBlock header")?;
        if message_id != MESSAGE_ID_DDB {
            return Err(Error::ReservedBitsViolation {
                field: "messageId",
                reason: "expected 0x1003 (DDB) on table_id 0x3C (ISO/IEC 13818-6 §7.3.7)",
            });
        }
        if payload.len() < DDB_FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: DDB_FIXED_LEN,
                have: payload.len(),
                what: "DownloadDataBlock body",
            });
        }
        Ok(DownloadDataBlock {
            download_id,
            adaptation,
            module_id: u16::from_be_bytes([payload[0], payload[1]]),
            module_version: payload[2],
            block_number: u16::from_be_bytes([payload[4], payload[5]]),
            block_data: &payload[DDB_FIXED_LEN..],
        })
    }
}

impl Serialize for DownloadDataBlock<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        MESSAGE_HEADER_LEN + self.adaptation.len() + DDB_FIXED_LEN + self.block_data.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let payload_len = DDB_FIXED_LEN + self.block_data.len();
        let pos = serialize_header(
            buf,
            MESSAGE_ID_DDB,
            self.download_id,
            self.adaptation,
            payload_len,
        )?;
        buf[pos..pos + 2].copy_from_slice(&self.module_id.to_be_bytes());
        buf[pos + 2] = self.module_version;
        buf[pos + 3] = 0xFF; // reserved
        buf[pos + 4..pos + 6].copy_from_slice(&self.block_number.to_be_bytes());
        buf[pos + DDB_FIXED_LEN..pos + DDB_FIXED_LEN + self.block_data.len()]
            .copy_from_slice(self.block_data);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_dsi() -> UnMessage<'static> {
        UnMessage::Dsi(Dsi {
            transaction_id: 0x8000_0000,
            adaptation: &[],
            server_id: [0xFF; 20],
            compatibility_descriptor: &[],
            private_data: &[0x0A, 0x0B],
        })
    }

    fn sample_dii() -> UnMessage<'static> {
        UnMessage::Dii(Dii {
            transaction_id: 0x8002_0002,
            adaptation: &[],
            download_id: 0x0000_00AB,
            block_size: 4066,
            window_size: 0,
            ack_period: 0,
            t_c_download_window: 0,
            t_c_download_scenario: 0,
            compatibility_descriptor: &[],
            modules: vec![
                DiiModule {
                    module_id: 1,
                    module_size: 8000,
                    module_version: 3,
                    module_info: &[0xDE, 0xAD],
                },
                DiiModule {
                    module_id: 2,
                    module_size: 100,
                    module_version: 1,
                    module_info: &[],
                },
            ],
            private_data: &[],
        })
    }

    #[test]
    fn dsi_round_trip() {
        let msg = sample_dsi();
        let mut buf = vec![0u8; msg.serialized_len()];
        msg.serialize_into(&mut buf).unwrap();
        assert_eq!(UnMessage::parse(&buf).unwrap(), msg);
    }

    #[test]
    fn dii_round_trip() {
        let msg = sample_dii();
        let mut buf = vec![0u8; msg.serialized_len()];
        msg.serialize_into(&mut buf).unwrap();
        assert_eq!(UnMessage::parse(&buf).unwrap(), msg);
    }

    #[test]
    fn ddb_round_trip() {
        let ddb = DownloadDataBlock {
            download_id: 0xAB,
            adaptation: &[],
            module_id: 1,
            module_version: 3,
            block_number: 2,
            block_data: &[0x55; 64],
        };
        let mut buf = vec![0u8; ddb.serialized_len()];
        ddb.serialize_into(&mut buf).unwrap();
        assert_eq!(DownloadDataBlock::parse(&buf).unwrap(), ddb);
    }

    #[test]
    fn header_fields_on_wire() {
        let msg = sample_dsi();
        let mut buf = vec![0u8; msg.serialized_len()];
        msg.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[0], 0x11); // protocolDiscriminator
        assert_eq!(buf[1], 0x03); // dsmccType
        assert_eq!(u16::from_be_bytes([buf[2], buf[3]]), MESSAGE_ID_DSI);
        assert_eq!(buf[8], 0xFF); // reserved
                                  // messageLength = bytes after the 12-byte header
        let ml = u16::from_be_bytes([buf[10], buf[11]]) as usize;
        assert_eq!(ml, buf.len() - 12);
    }

    #[test]
    fn parse_rejects_wrong_protocol_discriminator() {
        let msg = sample_dsi();
        let mut buf = vec![0u8; msg.serialized_len()];
        msg.serialize_into(&mut buf).unwrap();
        buf[0] = 0x12;
        assert!(matches!(
            UnMessage::parse(&buf).unwrap_err(),
            Error::ReservedBitsViolation {
                field: "protocolDiscriminator",
                ..
            }
        ));
    }

    #[test]
    fn parse_rejects_unknown_message_id() {
        let msg = sample_dsi();
        let mut buf = vec![0u8; msg.serialized_len()];
        msg.serialize_into(&mut buf).unwrap();
        buf[2] = 0x10;
        buf[3] = 0x01; // 0x1001 DownloadInfoRequest — not valid broadcast-side
        assert!(matches!(
            UnMessage::parse(&buf).unwrap_err(),
            Error::ReservedBitsViolation {
                field: "messageId",
                ..
            }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(matches!(
            UnMessage::parse(&[0x11, 0x03]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_message_length_overflow() {
        let msg = sample_dsi();
        let mut buf = vec![0u8; msg.serialized_len()];
        msg.serialize_into(&mut buf).unwrap();
        buf[10] = 0xFF;
        buf[11] = 0xFF; // declared messageLength way past the buffer
        assert!(matches!(
            UnMessage::parse(&buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn dii_module_info_overflow_rejected() {
        let msg = sample_dii();
        let mut buf = vec![0u8; msg.serialized_len()];
        msg.serialize_into(&mut buf).unwrap();
        // First module's moduleInfoLength is at header(12) + fixed(16) +
        // compatLen(2) + numberOfModules(2) + moduleHeader-1 = byte 39.
        buf[39] = 0xFF;
        assert!(matches!(
            UnMessage::parse(&buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn ddb_rejects_un_message_id() {
        let msg = sample_dsi();
        let mut buf = vec![0u8; msg.serialized_len()];
        msg.serialize_into(&mut buf).unwrap();
        assert!(matches!(
            DownloadDataBlock::parse(&buf).unwrap_err(),
            Error::ReservedBitsViolation {
                field: "messageId",
                ..
            }
        ));
    }

    #[test]
    fn adaptation_bytes_round_trip() {
        let ddb = DownloadDataBlock {
            download_id: 1,
            adaptation: &[0x01, 0x02, 0x03],
            module_id: 9,
            module_version: 0,
            block_number: 0,
            block_data: &[0xAA],
        };
        let mut buf = vec![0u8; ddb.serialized_len()];
        ddb.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[9], 3); // adaptationLength
        assert_eq!(DownloadDataBlock::parse(&buf).unwrap(), ddb);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn dii_serializes_to_valid_json() {
        let msg = sample_dii();
        let j = serde_json::to_string(&msg).unwrap();
        assert!(j.contains("\"download_id\":171"));
        assert!(j.contains("\"block_size\":4066"));
    }
}
