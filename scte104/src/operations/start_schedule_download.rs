//! start_schedule_download_request_data() — ANSI/SCTE 104 2023 §9.7.1, Table 9-18 (opID 0x0103).
//!
//! Normal usage request. Prepares the injector to receive schedule_definition
//! data. Carries a variable-length loop of provider_avail_id.

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use broadcast_common::{Parse, Serialize};

/// `opID` for start_schedule_download_request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x0103;

/// start_schedule_download_request_data() — §9.7.1, Table 9-18.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StartScheduleDownload {
    /// `num_provider_avails` — 1 byte. Zero = no avail_descriptor.
    pub num_provider_avails: u8,
    /// Loop of `provider_avail_id` (4 bytes each).
    pub provider_avail_ids: Vec<u32>,
}

impl<'a> Parse<'a> for StartScheduleDownload {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "start_schedule num_provider_avails",
            });
        }
        let count = bytes[0] as usize;
        let need = 1 + count * 4;
        if bytes.len() < need {
            return Err(Error::BufferTooShort {
                need,
                have: bytes.len(),
                what: "start_schedule provider_avail_ids",
            });
        }
        let mut ids = Vec::with_capacity(count);
        for i in 0..count {
            let off = 1 + i * 4;
            ids.push(u32::from_be_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ]));
        }
        Ok(Self {
            num_provider_avails: count as u8,
            provider_avail_ids: ids,
        })
    }
}

impl Serialize for StartScheduleDownload {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        1 + self.provider_avail_ids.len() * 4
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0] = self.num_provider_avails;
        for (i, &id) in self.provider_avail_ids.iter().enumerate() {
            let off = 1 + i * 4;
            buf[off..off + 4].copy_from_slice(&id.to_be_bytes());
        }
        Ok(need)
    }
}

impl OperationDef<'_> for StartScheduleDownload {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "START_SCHEDULE_DOWNLOAD";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let op = StartScheduleDownload {
            num_provider_avails: 2,
            provider_avail_ids: alloc::vec![0xAAAA_BBBB, 0xCCCC_DDDD],
        };
        let bytes = op.to_bytes();
        let back = StartScheduleDownload::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn zero_round_trip() {
        let op = StartScheduleDownload::default();
        let bytes = op.to_bytes();
        assert_eq!(bytes.len(), 1);
        let back = StartScheduleDownload::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn mutate_field_changes_output() {
        let op = StartScheduleDownload {
            num_provider_avails: 1,
            provider_avail_ids: alloc::vec![1],
        };
        let bytes = op.to_bytes();
        let mut op2 = op.clone();
        op2.provider_avail_ids[0] = 999;
        assert_ne!(op2.to_bytes(), bytes);
    }
}
