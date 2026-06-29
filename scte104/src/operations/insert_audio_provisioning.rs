//! insert_audio_provisioning request — ANSI/SCTE 104 2023 §9.8.12 (opID 0x0112).
//!
//! Control usage. Specifies channel mode for audio service.
//! The spec transcription for this operation's table was incomplete.
//! The operation body is carried as raw bytes (round-trip safe).

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use broadcast_common::{Parse, Serialize};

/// `opID` for insert_audio_provisioning request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x0112;

/// insert_audio_provisioning request — §9.8.12.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InsertAudioProvisioning<'a> {
    /// Raw operation body (typed struct pending full spec transcription).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub data: &'a [u8],
}

impl<'a> Parse<'a> for InsertAudioProvisioning<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Ok(Self { data: bytes })
    }
}

impl Serialize for InsertAudioProvisioning<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        self.data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[..need].copy_from_slice(self.data);
        Ok(need)
    }
}

impl<'a> OperationDef<'a> for InsertAudioProvisioning<'a> {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "INSERT_AUDIO_PROVISIONING";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let op = InsertAudioProvisioning {
            data: &[0x01, 0x02],
        };
        let bytes = op.to_bytes();
        let back = InsertAudioProvisioning::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }
}
