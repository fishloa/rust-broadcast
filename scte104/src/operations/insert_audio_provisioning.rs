//! insert_audio_provisioning request — ANSI/SCTE 104 2023 §9.8.12, Tables 9-34
//! and 9-35 (opID 0x0112).
//!
//! Control usage. Carries, per audio service, the `channel_mode` (SCTE 242-1
//! Table 1) to be set and the `Codec_index` (Table 9-35). Transcription:
//! `docs/ansi_scte_104/audio-and-alternate-break.md`.

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use broadcast_common::{Parse, Serialize};

/// `opID` for insert_audio_provisioning request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x0112;

/// Wire size of one entry (`channel_mode` 1 + `Codec_index` 1).
pub const ENTRY_LEN: usize = 2;

/// `Codec_index` — Table 9-35.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CodecIndex {
    /// 0 — reserved.
    Reserved,
    /// 1 — AC-3.
    Ac3,
    /// 2 — E-AC-3.
    Eac3,
    /// 3 — AAC.
    Aac,
    /// 5 — AC-4.
    Ac4,
    /// 6 — MPEG-H.
    MpegH,
    /// 4 or any value above 6 (reserved by Table 9-35 or unassigned); the byte
    /// is kept so serialize reproduces it.
    Unknown(u8),
}

impl CodecIndex {
    /// Decode a `Codec_index` byte.
    #[must_use]
    pub fn from_u8(b: u8) -> Self {
        match b {
            0 => Self::Reserved,
            1 => Self::Ac3,
            2 => Self::Eac3,
            3 => Self::Aac,
            5 => Self::Ac4,
            6 => Self::MpegH,
            other => Self::Unknown(other),
        }
    }

    /// The wire byte.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Reserved => 0,
            Self::Ac3 => 1,
            Self::Eac3 => 2,
            Self::Aac => 3,
            Self::Ac4 => 5,
            Self::MpegH => 6,
            Self::Unknown(b) => b,
        }
    }

    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Ac3 => "AC-3",
            Self::Eac3 => "E-AC-3",
            Self::Aac => "AAC",
            Self::Ac4 => "AC-4",
            Self::MpegH => "MPEG-H",
            Self::Unknown(_) => "reserved",
        }
    }
}

broadcast_common::impl_spec_display!(CodecIndex, Unknown);

/// One `for (i<audio_count)` entry of Table 9-34.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AudioProvisioningEntry {
    /// `channel_mode` — 1 byte, per SCTE 242-1 Table 1.
    pub channel_mode: u8,
    /// `Codec_index` — Table 9-35.
    pub codec_index: CodecIndex,
}

/// insert_audio_provisioning request — §9.8.12, Table 9-34.
///
/// `audio_count` is `entries.len()`, written through
/// `broadcast_common::len::fit_u8` on serialize.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InsertAudioProvisioning {
    /// The `audio_count` entries.
    pub entries: Vec<AudioProvisioningEntry>,
}

impl<'a> Parse<'a> for InsertAudioProvisioning {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (&count, rest) = bytes.split_first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "insert_audio_provisioning audio_count",
        })?;
        let need = usize::from(count) * ENTRY_LEN;
        if rest.len() < need {
            return Err(Error::BufferTooShort {
                need: 1 + need,
                have: bytes.len(),
                what: "insert_audio_provisioning entries",
            });
        }
        let entries = rest[..need]
            .chunks_exact(ENTRY_LEN)
            .map(|e| AudioProvisioningEntry {
                channel_mode: e[0],
                codec_index: CodecIndex::from_u8(e[1]),
            })
            .collect();
        Ok(Self { entries })
    }
}

impl Serialize for InsertAudioProvisioning {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        1 + self.entries.len() * ENTRY_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0] = broadcast_common::len::fit_u8(
            self.entries.len(),
            "insert_audio_provisioning.audio_count",
        )?;
        for (chunk, e) in buf[1..need].chunks_exact_mut(ENTRY_LEN).zip(&self.entries) {
            chunk[0] = e.channel_mode;
            chunk[1] = e.codec_index.to_u8();
        }
        Ok(need)
    }
}

impl OperationDef<'_> for InsertAudioProvisioning {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "INSERT_AUDIO_PROVISIONING";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> InsertAudioProvisioning {
        InsertAudioProvisioning {
            entries: alloc::vec![
                AudioProvisioningEntry {
                    channel_mode: 1,
                    codec_index: CodecIndex::Ac3,
                },
                AudioProvisioningEntry {
                    channel_mode: 2,
                    codec_index: CodecIndex::MpegH,
                },
            ],
        }
    }

    /// Tables 9-34 / 9-35 byte by byte.
    #[test]
    fn wire_layout_matches_table_9_34() {
        let bytes = sample().try_to_bytes().unwrap();
        assert_eq!(bytes, [2, 1, 1, 2, 6]);
        assert_eq!(InsertAudioProvisioning::parse(&bytes).unwrap(), sample());
    }

    #[test]
    fn codec_index_values_match_table_9_35_and_unknown_survives() {
        for (b, c) in [
            (0, CodecIndex::Reserved),
            (1, CodecIndex::Ac3),
            (2, CodecIndex::Eac3),
            (3, CodecIndex::Aac),
            (5, CodecIndex::Ac4),
            (6, CodecIndex::MpegH),
        ] {
            assert_eq!(CodecIndex::from_u8(b), c);
            assert_eq!(c.to_u8(), b);
        }
        // 4 is reserved by the table and must round-trip, not collapse.
        assert_eq!(CodecIndex::from_u8(4), CodecIndex::Unknown(4));
        let bytes = [1, 9, 4];
        let op = InsertAudioProvisioning::parse(&bytes).unwrap();
        assert_eq!(op.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn truncated_entries_are_rejected() {
        assert!(InsertAudioProvisioning::parse(&[2, 1, 1, 2]).is_err());
        assert!(InsertAudioProvisioning::parse(&[]).is_err());
    }
}
