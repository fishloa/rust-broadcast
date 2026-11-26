//! In-band framing the adapter has to reconstruct.
//!
//! `ffmpeg-next` 8.1 exposes no extradata setter: a decoder cannot be handed
//! the `avcC` record or the AAC `AudioSpecificConfig` a container carries. The
//! adapter therefore keeps libav's length-prefixed mode for H.264
//! (`is_avc`/`nal_length_size`) and supplies the parameter sets in-band, and
//! wraps a raw AAC access unit in the ADTS header libav's decoder accepts.
//!
//! Both transforms are this crate's own, written from the container record
//! layouts (ISO/IEC 14496-15:2017 for `avcC`, ISO/IEC 14496-3 for the
//! `AudioSpecificConfig` and ISO/IEC 13818-7 for the ADTS header). No FFmpeg
//! source is copied.

use std::borrow::Cow;
use std::string::{String, ToString};
use std::vec::Vec;

use broadcast_codec::{CodecError, CodecId};

/// `configurationVersion` a decodable `avcC` record starts with.
const AVC_CONFIG_VERSION: u8 = 1;
/// Offset of `lengthSizeMinusOne` inside the record.
const AVC_LENGTH_SIZE_OFFSET: usize = 4;
/// Its mask, after the six reserved bits.
const AVC_LENGTH_SIZE_MASK: u8 = 0x03;
/// `lengthSizeMinusOne` counts from zero.
const AVC_LENGTH_SIZE_BIAS: u8 = 1;
/// Offset of `numOfSequenceParameterSets`.
const AVC_SPS_COUNT_OFFSET: usize = 5;
/// Its mask, after the three reserved bits.
const AVC_SPS_COUNT_MASK: u8 = 0x1F;
/// Bytes in the length prefix of one parameter-set entry in the record.
const AVC_PARAMETER_SET_PREFIX: usize = 2;

/// Bits per byte, for the big-endian prefixes.
const BYTE_BITS: u32 = 8;

/// `AudioObjectType` is read from the top of the first `AudioSpecificConfig`
/// byte.
const ASC_OBJECT_TYPE_SHIFT: u8 = 3;
/// The low bits of that byte that continue into the sampling index.
const ASC_OBJECT_TYPE_MASK: u8 = 0x07;
/// The sampling-frequency index takes the rest of byte 0 and the top of
/// byte 1.
const ASC_FREQUENCY_SHIFT: u8 = 7;
/// The channel configuration starts at this bit of byte 1.
const ASC_CHANNEL_SHIFT: u8 = 3;
/// It is four bits wide.
const ASC_CHANNEL_MASK: u8 = 0x0F;
/// The first `AudioObjectType` an ADTS `profile` field can name.
const ADTS_PROFILE_BIAS: u8 = 1;
/// The ADTS `profile` field is two bits wide.
const ADTS_PROFILE_MAX: u8 = 3;

/// Bytes of one ADTS header (no CRC, one raw data block).
pub(crate) const ADTS_HEADER_LEN: usize = 7;
/// The 13-bit mask a frame length has to fit.
const ADTS_LENGTH_MAX: usize = 0x1FFF;
/// First two header bytes: syncword, MPEG-4, layer 0, no CRC.
const ADTS_SYNC: [u8; 2] = [0xFF, 0xF1];
/// `profile` starts at this bit of header byte 2.
const ADTS_PROFILE_SHIFT: u8 = 6;
/// The sampling-frequency index starts here.
const ADTS_FREQUENCY_SHIFT: u8 = 2;
/// The top channel-configuration bit is header byte 2's last bit.
const ADTS_CHANNEL_HIGH_BIT: u8 = 2;
/// The low two channel-configuration bits open header byte 3.
const ADTS_CHANNEL_LOW_SHIFT: u8 = 6;
/// Length bits in header byte 3, after the channel configuration.
const ADTS_LENGTH_HIGH_SHIFT: u8 = 11;
/// Header byte 4 carries the middle eight length bits.
const ADTS_LENGTH_MID_SHIFT: u8 = 3;
/// Header byte 5's top three bits finish the length.
const ADTS_LENGTH_LOW_SHIFT: u8 = 5;
/// The eleven `adts_buffer_fullness` bits say "variable rate".
const ADTS_BUFFER_FULLNESS: u8 = 0x1F;
/// Header byte 6: one raw data block, no CRC follows.
const ADTS_TAIL: u8 = 0xFC;
/// Mask of one byte.
const BYTE_MASK: usize = 0xFF;
/// Mask of the two channel-configuration bits header byte 3 carries.
const ADTS_CHANNEL_LOW_MASK: u8 = 0x03;
/// Mask of the two bits a length contributes to header byte 3.
const ADTS_LENGTH_HIGH_MASK: usize = 0x03;
/// Mask of the three length bits header byte 5 carries.
const ADTS_LENGTH_LOW_MASK: usize = 0x07;

/// Where one codec's access units come from, in the framing libav accepts.
#[derive(Debug, Clone)]
pub(crate) enum Inbound {
    /// The bytes go in unchanged.
    Raw,
    /// Length-prefixed H.264 access units, with the `avcC` parameter sets
    /// supplied in-band.
    Avc(Avc),
    /// Raw AAC access units, wrapped in an ADTS header.
    Adts(Adts),
}

impl Inbound {
    /// The framing `codec`'s access units are in, given its extradata.
    pub(crate) fn new(codec: CodecId, extradata: &[u8]) -> Self {
        match codec {
            CodecId::H264 => Avc::parse(extradata).map_or(Self::Raw, Self::Avc),
            CodecId::Aac => Adts::parse(extradata).map_or(Self::Raw, Self::Adts),
            _ => Self::Raw,
        }
    }

    /// The decoder options this framing needs before the codec is opened.
    pub(crate) fn options(&self) -> Vec<(&'static str, String)> {
        match self {
            Self::Avc(avc) => vec![
                ("is_avc", "1".to_string()),
                ("nal_length_size", avc.nal_length_size.to_string()),
            ],
            Self::Raw | Self::Adts(_) => Vec::new(),
        }
    }

    /// The bytes to hand libav for one access unit.
    ///
    /// `with_parameter_sets` states that this access unit starts a
    /// random-access point, so the H.264 parameter sets are repeated in front
    /// of it.
    pub(crate) fn prepare<'a>(
        &self,
        sample: &'a [u8],
        with_parameter_sets: bool,
    ) -> Result<Cow<'a, [u8]>, CodecError> {
        match self {
            Self::Raw => Ok(Cow::Borrowed(sample)),
            Self::Avc(avc) => Ok(Cow::Owned(avc.frame(sample, with_parameter_sets)?)),
            Self::Adts(adts) => adts.frame(sample).map(Cow::Owned),
        }
    }
}

/// The H.264 parameter sets an `avcC` record carries, ready to go in-band.
#[derive(Debug, Clone)]
pub(crate) struct Avc {
    /// Width of the length prefix in the access units themselves.
    nal_length_size: u8,
    /// The parameter-set NAL units, each with a length prefix of the same
    /// width, in the record's order.
    parameter_sets: Vec<u8>,
}

impl Avc {
    /// Read the record, or `None` when `extradata` is not a decodable one.
    pub(crate) fn parse(extradata: &[u8]) -> Option<Self> {
        if extradata.first().copied()? != AVC_CONFIG_VERSION {
            return None;
        }
        let length_byte = *extradata.get(AVC_LENGTH_SIZE_OFFSET)?;
        let nal_length_size = (length_byte & AVC_LENGTH_SIZE_MASK) + AVC_LENGTH_SIZE_BIAS;
        let sps_count = *extradata.get(AVC_SPS_COUNT_OFFSET)? & AVC_SPS_COUNT_MASK;
        let mut cursor = AVC_SPS_COUNT_OFFSET + 1;
        let mut parameter_sets = Vec::new();
        for _ in 0..sps_count {
            cursor = length_prefixed(extradata, cursor, nal_length_size, &mut parameter_sets)?;
        }
        let pps_count = *extradata.get(cursor)?;
        cursor += 1;
        for _ in 0..pps_count {
            cursor = length_prefixed(extradata, cursor, nal_length_size, &mut parameter_sets)?;
        }
        Some(Self {
            nal_length_size,
            parameter_sets,
        })
    }

    /// One access unit, with the parameter sets first when this is a
    /// random-access point.
    fn frame(&self, sample: &[u8], with_parameter_sets: bool) -> Result<Vec<u8>, CodecError> {
        if !with_parameter_sets {
            return Ok(sample.to_vec());
        }
        let mut out = Vec::with_capacity(self.parameter_sets.len() + sample.len());
        out.extend_from_slice(&self.parameter_sets);
        out.extend_from_slice(sample);
        Ok(out)
    }
}

/// Copy one `avcC` parameter-set entry into `out`, length-prefixed with
/// `nal_length_size` bytes; returns the offset after the entry.
fn length_prefixed(
    record: &[u8],
    offset: usize,
    nal_length_size: u8,
    out: &mut Vec<u8>,
) -> Option<usize> {
    let prefix = record.get(offset..offset + AVC_PARAMETER_SET_PREFIX)?;
    let length = usize::from(u16::from_be_bytes([prefix[0], prefix[1]]));
    let start = offset + AVC_PARAMETER_SET_PREFIX;
    let nal = record.get(start..start + length)?;
    let width = usize::from(nal_length_size);
    let encoded = length as u64;
    for index in (0..width).rev() {
        out.push((encoded >> (index as u32 * BYTE_BITS)) as u8);
    }
    out.extend_from_slice(nal);
    Some(start + length)
}

/// The fields an AAC `AudioSpecificConfig` implies for an ADTS header.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Adts {
    /// The ADTS `profile` field, `AudioObjectType - 1`.
    profile: u8,
    /// The sampling-frequency index.
    frequency_index: u8,
    /// The channel configuration.
    channel_configuration: u8,
}

impl Adts {
    /// Read the fields from a raw `AudioSpecificConfig`, or `None` when it is
    /// too short or names a profile ADTS cannot carry.
    pub(crate) fn parse(extradata: &[u8]) -> Option<Self> {
        let first = *extradata.first()?;
        let second = *extradata.get(1)?;
        let object_type = first >> ASC_OBJECT_TYPE_SHIFT;
        if object_type == 0 || object_type > ADTS_PROFILE_MAX + ADTS_PROFILE_BIAS {
            return None;
        }
        let frequency_index =
            ((first & ASC_OBJECT_TYPE_MASK) << 1) | (second >> ASC_FREQUENCY_SHIFT);
        let channel_configuration = (second >> ASC_CHANNEL_SHIFT) & ASC_CHANNEL_MASK;
        Some(Self {
            profile: object_type - ADTS_PROFILE_BIAS,
            frequency_index,
            channel_configuration,
        })
    }

    /// One raw access unit with an ADTS header in front of it.
    fn frame(&self, payload: &[u8]) -> Result<Vec<u8>, CodecError> {
        let length = payload.len() + ADTS_HEADER_LEN;
        if length > ADTS_LENGTH_MAX {
            return Err(CodecError::refused("AAC access unit too long for ADTS"));
        }
        let high = (self.channel_configuration >> ADTS_CHANNEL_HIGH_BIT) & 1;
        let low = (self.channel_configuration & ADTS_CHANNEL_LOW_MASK) << ADTS_CHANNEL_LOW_SHIFT;
        let mut out = Vec::with_capacity(length);
        out.extend_from_slice(&ADTS_SYNC);
        out.push(
            (self.profile << ADTS_PROFILE_SHIFT)
                | (self.frequency_index << ADTS_FREQUENCY_SHIFT)
                | high,
        );
        out.push(low | ((length >> ADTS_LENGTH_HIGH_SHIFT) & ADTS_LENGTH_HIGH_MASK) as u8);
        out.push(((length >> ADTS_LENGTH_MID_SHIFT) & BYTE_MASK) as u8);
        out.push(
            (((length & ADTS_LENGTH_LOW_MASK) << ADTS_LENGTH_LOW_SHIFT) as u8)
                | ADTS_BUFFER_FULLNESS,
        );
        out.push(ADTS_TAIL);
        out.extend_from_slice(payload);
        Ok(out)
    }
}
