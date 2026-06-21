//! Sample decryption resource objects — ETSI TS 103 205 V1.4.1 §7.4, Tables
//! 30-39 (PDF pp. 52-60). See `docs/ts_103_205/sample-decryption.md`.
//!
//! Resource ID `0x00920041` (Class 146, Type 1, Version 1). The resource controls
//! decryption by the CICAM of a set of consecutive media Samples packaged into an
//! MPEG-2 TS (the IP-delivery Host-player mode). Tags live in the CI Plus
//! `0x9F98xx` namespace.
//!
//! - `sd_info_req` (`9F 98 00`, Table 31) — Host → CICAM, header-only.
//! - `sd_info_reply` (`9F 98 01`, Table 32) — CICAM → Host.
//! - `sd_start` (`9F 98 02`, Table 33) — Host → CICAM.
//! - `sd_start_reply` (`9F 98 03`, Table 35) — CICAM → Host.
//! - `sd_update` (`9F 98 04`, Table 38) — Host → CICAM.
//! - `sd_update_reply` (`9F 98 05`, Table 39) — CICAM → Host.
//!
//! The `drm_metadata_byte` bodies (pssh/sinf/CASD/MPD/OSDT blobs, Table 34) are
//! **opaque** to the wire parser — carried as borrowed `&[u8]`.

use crate::error::{Error, Result};
use crate::objects;
use crate::tag::ApduTag;
use alloc::vec::Vec;
use dvb_common::{Parse, Serialize};

/// Resource-scoped `apdu_tag`s for the Sample decryption resource (Table 30).
pub mod tag {
    use crate::tag::ApduTag;
    /// `sd_info_req_tag` = `9F 98 00`.
    pub const SD_INFO_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x98, 0x00);
    /// `sd_info_reply_tag` = `9F 98 01`.
    pub const SD_INFO_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0x98, 0x01);
    /// `sd_start_tag` = `9F 98 02`.
    pub const SD_START: ApduTag = ApduTag::from_bytes(0x9F, 0x98, 0x02);
    /// `sd_start_reply_tag` = `9F 98 03`.
    pub const SD_START_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0x98, 0x03);
    /// `sd_update_tag` = `9F 98 04`.
    pub const SD_UPDATE: ApduTag = ApduTag::from_bytes(0x9F, 0x98, 0x04);
    /// `sd_update_reply_tag` = `9F 98 05`.
    pub const SD_UPDATE_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0x98, 0x05);
}

/// A 128-bit (16-byte) DRM UUID (`drm_uuid`). All-`0xFF` means "not used".
pub const DRM_UUID_LEN: usize = 16;

// --- sd_info_req (Table 31) ---

/// `sd_info_req()` (Table 31): Host → CICAM, header-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SdInfoReq;

impl<'a> Parse<'a> for SdInfoReq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        objects::parse_empty_apdu(bytes, tag::SD_INFO_REQ, "sd_info_req")?;
        Ok(Self)
    }
}
impl Serialize for SdInfoReq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::empty_apdu_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        objects::serialize_empty_apdu(tag::SD_INFO_REQ, buf)
    }
}

// --- sd_info_reply (Table 32) ---

/// `sd_info_reply()` (Table 32): CICAM → Host. Lists `drm_system_id`s and DRM
/// UUIDs the CICAM supports for Sample decryption.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SdInfoReply {
    /// `drm_system_id` list (loop count `number_of_drm_system_id`). Values are the
    /// same as `ca_system_id` per the DVB allocation \[11\].
    pub drm_system_ids: Vec<u16>,
    /// `drm_uuid` list (loop count `number_of_drm_uuid`), each 128 bits.
    pub drm_uuids: Vec<[u8; DRM_UUID_LEN]>,
}

impl<'a> Parse<'a> for SdInfoReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::SD_INFO_REPLY, "sd_info_reply")?;
        let mut r = Reader::new(body, "sd_info_reply");
        let n_sys = r.u8()? as usize;
        let mut drm_system_ids = Vec::with_capacity(n_sys);
        for _ in 0..n_sys {
            drm_system_ids.push(r.u16()?);
        }
        let n_uuid = r.u8()? as usize;
        let mut drm_uuids = Vec::with_capacity(n_uuid);
        for _ in 0..n_uuid {
            drm_uuids.push(r.uuid()?);
        }
        Ok(Self {
            drm_system_ids,
            drm_uuids,
        })
    }
}
impl Serialize for SdInfoReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(
            1 + self.drm_system_ids.len() * 2 + 1 + self.drm_uuids.len() * DRM_UUID_LEN,
        )
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = 1 + self.drm_system_ids.len() * 2 + 1 + self.drm_uuids.len() * DRM_UUID_LEN;
        let pos = objects::write_apdu_header(tag::SD_INFO_REPLY, body_len, buf)?;
        let mut w = Writer::new(&mut buf[pos..]);
        w.u8(self.drm_system_ids.len() as u8);
        for id in &self.drm_system_ids {
            w.u16(*id);
        }
        w.u8(self.drm_uuids.len() as u8);
        for uuid in &self.drm_uuids {
            w.uuid(uuid);
        }
        Ok(pos + body_len)
    }
}

// --- DRM metadata record (shared by sd_start / sd_update, Table 33/34/38) ---

/// One `drm_metadata` record (Tables 33/38): the `drm_metadata_source`,
/// `drm_system_id`, `drm_uuid`, and the opaque `drm_metadata_byte` body.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DrmMetadataRecord<'a> {
    /// `drm_metadata_source` (8) — source of the metadata, per Table 34.
    pub drm_metadata_source: u8,
    /// `drm_system_id` (16) — DRM system the metadata relates to; `0xFFFF` if not
    /// used. Same values as `ca_system_id` \[11\].
    pub drm_system_id: u16,
    /// `drm_uuid` (128) — UUID of the DRM; all `0xFF` if not used.
    pub drm_uuid: [u8; DRM_UUID_LEN],
    /// `drm_metadata_byte` body (`drm_metadata_length` bytes) — opaque blob.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub drm_metadata: &'a [u8],
}

// drm_metadata_source(1) + drm_system_id(2) + drm_uuid(16) + drm_metadata_length(2).
const METADATA_FIXED: usize = 1 + 2 + DRM_UUID_LEN + 2;

impl<'a> DrmMetadataRecord<'a> {
    fn parse_from(r: &mut Reader<'a>) -> Result<Self> {
        let drm_metadata_source = r.u8()?;
        let drm_system_id = r.u16()?;
        let drm_uuid = r.uuid()?;
        let len = r.u16()? as usize;
        let drm_metadata = r.take(len)?;
        Ok(Self {
            drm_metadata_source,
            drm_system_id,
            drm_uuid,
            drm_metadata,
        })
    }
    fn body_len(&self) -> usize {
        METADATA_FIXED + self.drm_metadata.len()
    }
    fn write_into(&self, w: &mut Writer<'_>) {
        w.u8(self.drm_metadata_source);
        w.u16(self.drm_system_id);
        w.uuid(&self.drm_uuid);
        w.u16(self.drm_metadata.len() as u16);
        w.bytes(self.drm_metadata);
    }
}

/// One Sample Track entry (the `ts_flag == 0` branch of Tables 33/38):
/// `track_PID` + a `number_of_metadata_records` loop of [`DrmMetadataRecord`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleTrack<'a> {
    /// `track_PID` (13) — PID on which the Samples for this Sample Track are sent.
    /// Values `0x0000`–`0x001F` are reserved.
    pub track_pid: u16,
    /// The DRM metadata records for this track.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub records: Vec<DrmMetadataRecord<'a>>,
}

// reserved(3)+track_PID(13) = 2 bytes, then number_of_metadata_records(1).
const TRACK_FIXED: usize = 2 + 1;
const TRACK_PID_MASK: u16 = 0x1FFF;

/// The Sample-decryption payload that follows the per-LTS fixed header in both
/// `sd_start` (Table 33) and `sd_update` (Table 38): selected by `ts_flag`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum SamplePayload<'a> {
    /// `ts_flag == 1` — a TS-level metadata-record loop
    /// (`number_of_metadata_records`).
    Ts(#[cfg_attr(feature = "serde", serde(borrow))] Vec<DrmMetadataRecord<'a>>),
    /// `ts_flag == 0` — a `number_of_Sample_Tracks` loop, each a
    /// [`SampleTrack`].
    Tracks(#[cfg_attr(feature = "serde", serde(borrow))] Vec<SampleTrack<'a>>),
}

impl<'a> SamplePayload<'a> {
    /// `true` if this is the TS-level (`ts_flag == 1`) variant.
    #[must_use]
    pub fn ts_flag(&self) -> bool {
        matches!(self, Self::Ts(_))
    }

    fn parse_from(r: &mut Reader<'a>, ts_flag: bool) -> Result<Self> {
        if ts_flag {
            let n = r.u8()? as usize;
            let mut records = Vec::with_capacity(n);
            for _ in 0..n {
                records.push(DrmMetadataRecord::parse_from(r)?);
            }
            Ok(Self::Ts(records))
        } else {
            let n = r.u8()? as usize;
            let mut tracks = Vec::with_capacity(n);
            for _ in 0..n {
                let pid_word = r.u16()?;
                let track_pid = pid_word & TRACK_PID_MASK;
                let m = r.u8()? as usize;
                let mut records = Vec::with_capacity(m);
                for _ in 0..m {
                    records.push(DrmMetadataRecord::parse_from(r)?);
                }
                tracks.push(SampleTrack { track_pid, records });
            }
            Ok(Self::Tracks(tracks))
        }
    }

    fn body_len(&self) -> usize {
        match self {
            Self::Ts(records) => {
                1 + records
                    .iter()
                    .map(DrmMetadataRecord::body_len)
                    .sum::<usize>()
            }
            Self::Tracks(tracks) => {
                1 + tracks
                    .iter()
                    .map(|t| {
                        TRACK_FIXED
                            + t.records
                                .iter()
                                .map(DrmMetadataRecord::body_len)
                                .sum::<usize>()
                    })
                    .sum::<usize>()
            }
        }
    }

    fn write_into(&self, w: &mut Writer<'_>) {
        match self {
            Self::Ts(records) => {
                w.u8(records.len() as u8);
                for rec in records {
                    rec.write_into(w);
                }
            }
            Self::Tracks(tracks) => {
                w.u8(tracks.len() as u8);
                for track in tracks {
                    // reserved(3)='000' + track_PID(13).
                    w.u16(track.track_pid & TRACK_PID_MASK);
                    w.u8(track.records.len() as u8);
                    for rec in &track.records {
                        rec.write_into(w);
                    }
                }
            }
        }
    }
}

// --- sd_start (Table 33) ---

/// `sd_start()` (Table 33): Host → CICAM.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SdStart<'a> {
    /// `LTS_id` (8) — identifier of the Local TS.
    pub lts_id: u8,
    /// `program_number` (16) — used by the CICAM in URI messages.
    pub program_number: u16,
    /// The `ts_flag`-selected payload.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub payload: SamplePayload<'a>,
}

// LTS_id(1) + program_number(2) + reserved(7)+ts_flag(1) byte.
const SD_START_FIXED: usize = 1 + 2 + 1;
// ts_flag is the LSB of the reserved(7)+ts_flag(1) byte.
const TS_FLAG_BIT: u8 = 0x01;

impl<'a> Parse<'a> for SdStart<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::SD_START, "sd_start")?;
        let mut r = Reader::new(body, "sd_start");
        let lts_id = r.u8()?;
        let program_number = r.u16()?;
        let ts_flag = r.u8()? & TS_FLAG_BIT != 0;
        let payload = SamplePayload::parse_from(&mut r, ts_flag)?;
        Ok(Self {
            lts_id,
            program_number,
            payload,
        })
    }
}
impl Serialize for SdStart<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(SD_START_FIXED + self.payload.body_len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = SD_START_FIXED + self.payload.body_len();
        let pos = objects::write_apdu_header(tag::SD_START, body_len, buf)?;
        let mut w = Writer::new(&mut buf[pos..]);
        w.u8(self.lts_id);
        w.u16(self.program_number);
        w.u8(if self.payload.ts_flag() {
            TS_FLAG_BIT
        } else {
            0
        });
        self.payload.write_into(&mut w);
        Ok(pos + body_len)
    }
}

// --- sd_start_reply (Table 35) ---

/// `transmission_status` values (Table 36).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum TransmissionStatus {
    /// `0x00` — Ready to receive.
    ReadyToReceive,
    /// `0x01` — Error: CICAM busy.
    CicamBusy,
    /// `0x02` — Error: other reason.
    OtherReason,
    /// Reserved (`0x03`–`0xFF`).
    Reserved(u8),
}
impl TransmissionStatus {
    /// Decode a `transmission_status` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::ReadyToReceive,
            0x01 => Self::CicamBusy,
            0x02 => Self::OtherReason,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::ReadyToReceive => 0x00,
            Self::CicamBusy => 0x01,
            Self::OtherReason => 0x02,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ReadyToReceive => "ready_to_receive",
            Self::CicamBusy => "error_cicam_busy",
            Self::OtherReason => "error_other_reason",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(TransmissionStatus, Reserved);

/// `drm_status` values (Table 37).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DrmStatus {
    /// `0x00` — Decryption possible.
    DecryptionPossible,
    /// `0x01` — Status currently undetermined.
    Undetermined,
    /// `0x02` — Error: no entitlement.
    NoEntitlement,
    /// Reserved (`0x03`–`0xFF`).
    Reserved(u8),
}
impl DrmStatus {
    /// Decode a `drm_status` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::DecryptionPossible,
            0x01 => Self::Undetermined,
            0x02 => Self::NoEntitlement,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::DecryptionPossible => 0x00,
            Self::Undetermined => 0x01,
            Self::NoEntitlement => 0x02,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::DecryptionPossible => "decryption_possible",
            Self::Undetermined => "status_undetermined",
            Self::NoEntitlement => "error_no_entitlement",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(DrmStatus, Reserved);

/// `sd_start_reply()` (Table 35): CICAM → Host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SdStartReply {
    /// `LTS_id` (8).
    pub lts_id: u8,
    /// `transmission_status` (8) — Table 36.
    pub transmission_status: TransmissionStatus,
    /// `drm_status` (8) — Table 37.
    pub drm_status: DrmStatus,
    /// `drm_system_id` (16) — `0xFFFF` if not used.
    pub drm_system_id: u16,
    /// `drm_uuid` (128) — all `0xFF` if not used.
    pub drm_uuid: [u8; DRM_UUID_LEN],
    /// `buffer_size` (16) — CICAM buffer in transport packets (min 5000).
    pub buffer_size: u16,
    /// `data_block_size` (16) — `0` = no transfer-size constraints.
    pub data_block_size: u16,
}

// LTS_id(1)+transmission_status(1)+drm_status(1)+drm_system_id(2)+drm_uuid(16)
//   +buffer_size(2)+data_block_size(2).
const SD_START_REPLY_BODY: usize = 1 + 1 + 1 + 2 + DRM_UUID_LEN + 2 + 2;

impl<'a> Parse<'a> for SdStartReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::SD_START_REPLY, "sd_start_reply")?;
        if body.len() < SD_START_REPLY_BODY {
            return Err(Error::BufferTooShort {
                need: SD_START_REPLY_BODY,
                have: body.len(),
                what: "sd_start_reply",
            });
        }
        let mut r = Reader::new(body, "sd_start_reply");
        Ok(Self {
            lts_id: r.u8()?,
            transmission_status: TransmissionStatus::from_u8(r.u8()?),
            drm_status: DrmStatus::from_u8(r.u8()?),
            drm_system_id: r.u16()?,
            drm_uuid: r.uuid()?,
            buffer_size: r.u16()?,
            data_block_size: r.u16()?,
        })
    }
}
impl Serialize for SdStartReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(SD_START_REPLY_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::SD_START_REPLY, SD_START_REPLY_BODY, buf)?;
        let mut w = Writer::new(&mut buf[pos..]);
        w.u8(self.lts_id);
        w.u8(self.transmission_status.to_u8());
        w.u8(self.drm_status.to_u8());
        w.u16(self.drm_system_id);
        w.uuid(&self.drm_uuid);
        w.u16(self.buffer_size);
        w.u16(self.data_block_size);
        Ok(pos + SD_START_REPLY_BODY)
    }
}

// --- sd_update (Table 38) ---

/// `sd_update()` (Table 38): Host → CICAM. Same `ts_flag`-selected payload shape
/// as [`SdStart`], but the per-LTS header has no `program_number`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SdUpdate<'a> {
    /// `LTS_id` (8) — Local TS for which the update applies.
    pub lts_id: u8,
    /// The `ts_flag`-selected payload (shall match the `sd_start` `ts_flag`).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub payload: SamplePayload<'a>,
}

// LTS_id(1) + reserved(7)+ts_flag(1) byte.
const SD_UPDATE_FIXED: usize = 1 + 1;

impl<'a> Parse<'a> for SdUpdate<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::SD_UPDATE, "sd_update")?;
        let mut r = Reader::new(body, "sd_update");
        let lts_id = r.u8()?;
        let ts_flag = r.u8()? & TS_FLAG_BIT != 0;
        let payload = SamplePayload::parse_from(&mut r, ts_flag)?;
        Ok(Self { lts_id, payload })
    }
}
impl Serialize for SdUpdate<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(SD_UPDATE_FIXED + self.payload.body_len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = SD_UPDATE_FIXED + self.payload.body_len();
        let pos = objects::write_apdu_header(tag::SD_UPDATE, body_len, buf)?;
        let mut w = Writer::new(&mut buf[pos..]);
        w.u8(self.lts_id);
        w.u8(if self.payload.ts_flag() {
            TS_FLAG_BIT
        } else {
            0
        });
        self.payload.write_into(&mut w);
        Ok(pos + body_len)
    }
}

// --- sd_update_reply (Table 39) ---

/// `sd_update_reply()` (Table 39): CICAM → Host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SdUpdateReply {
    /// `LTS_id` (8).
    pub lts_id: u8,
    /// `drm_status` (8) — Table 37.
    pub drm_status: DrmStatus,
}

// LTS_id(1) + drm_status(1).
const SD_UPDATE_REPLY_BODY: usize = 2;

impl<'a> Parse<'a> for SdUpdateReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::SD_UPDATE_REPLY, "sd_update_reply")?;
        if body.len() < SD_UPDATE_REPLY_BODY {
            return Err(Error::BufferTooShort {
                need: SD_UPDATE_REPLY_BODY,
                have: body.len(),
                what: "sd_update_reply",
            });
        }
        Ok(Self {
            lts_id: body[0],
            drm_status: DrmStatus::from_u8(body[1]),
        })
    }
}
impl Serialize for SdUpdateReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(SD_UPDATE_REPLY_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::SD_UPDATE_REPLY, SD_UPDATE_REPLY_BODY, buf)?;
        buf[pos] = self.lts_id;
        buf[pos + 1] = self.drm_status.to_u8();
        Ok(pos + SD_UPDATE_REPLY_BODY)
    }
}

/// Resource-scoped dispatch over the Sample decryption resource objects.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SampleDecryptionApdu<'a> {
    /// `sd_info_req` (`9F 98 00`).
    SdInfoReq(SdInfoReq),
    /// `sd_info_reply` (`9F 98 01`).
    SdInfoReply(SdInfoReply),
    /// `sd_start` (`9F 98 02`).
    SdStart(#[cfg_attr(feature = "serde", serde(borrow))] SdStart<'a>),
    /// `sd_start_reply` (`9F 98 03`).
    SdStartReply(SdStartReply),
    /// `sd_update` (`9F 98 04`).
    SdUpdate(#[cfg_attr(feature = "serde", serde(borrow))] SdUpdate<'a>),
    /// `sd_update_reply` (`9F 98 05`).
    SdUpdateReply(SdUpdateReply),
}

impl<'a> SampleDecryptionApdu<'a> {
    /// Parse a Sample decryption APDU, dispatching on the leading `apdu_tag`.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "sample_decryption apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            tag::SD_INFO_REQ => Ok(Self::SdInfoReq(SdInfoReq::parse(body)?)),
            tag::SD_INFO_REPLY => Ok(Self::SdInfoReply(SdInfoReply::parse(body)?)),
            tag::SD_START => Ok(Self::SdStart(SdStart::parse(body)?)),
            tag::SD_START_REPLY => Ok(Self::SdStartReply(SdStartReply::parse(body)?)),
            tag::SD_UPDATE => Ok(Self::SdUpdate(SdUpdate::parse(body)?)),
            tag::SD_UPDATE_REPLY => Ok(Self::SdUpdateReply(SdUpdateReply::parse(body)?)),
            _ => Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: tag::SD_INFO_REQ.as_u24(),
                what: "sample_decryption",
            }),
        }
    }
}

impl Serialize for SampleDecryptionApdu<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::SdInfoReq(o) => o.serialized_len(),
            Self::SdInfoReply(o) => o.serialized_len(),
            Self::SdStart(o) => o.serialized_len(),
            Self::SdStartReply(o) => o.serialized_len(),
            Self::SdUpdate(o) => o.serialized_len(),
            Self::SdUpdateReply(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::SdInfoReq(o) => o.serialize_into(buf),
            Self::SdInfoReply(o) => o.serialize_into(buf),
            Self::SdStart(o) => o.serialize_into(buf),
            Self::SdStartReply(o) => o.serialize_into(buf),
            Self::SdUpdate(o) => o.serialize_into(buf),
            Self::SdUpdateReply(o) => o.serialize_into(buf),
        }
    }
}

// --- Small big-endian cursor helpers (parse/serialize without magic offsets) ---

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    what: &'static str,
}
impl<'a> Reader<'a> {
    fn new(buf: &'a [u8], what: &'static str) -> Self {
        Self { buf, pos: 0, what }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.buf.len() < self.pos + n {
            return Err(Error::BufferTooShort {
                need: n,
                have: self.buf.len().saturating_sub(self.pos),
                what: self.what,
            });
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        let s = self.take(2)?;
        Ok(u16::from_be_bytes([s[0], s[1]]))
    }
    fn uuid(&mut self) -> Result<[u8; DRM_UUID_LEN]> {
        let s = self.take(DRM_UUID_LEN)?;
        let mut u = [0u8; DRM_UUID_LEN];
        u.copy_from_slice(s);
        Ok(u)
    }
}

struct Writer<'a> {
    buf: &'a mut [u8],
    pos: usize,
}
impl<'a> Writer<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn u8(&mut self, v: u8) {
        self.buf[self.pos] = v;
        self.pos += 1;
    }
    fn u16(&mut self, v: u16) {
        self.buf[self.pos..self.pos + 2].copy_from_slice(&v.to_be_bytes());
        self.pos += 2;
    }
    fn uuid(&mut self, v: &[u8; DRM_UUID_LEN]) {
        self.buf[self.pos..self.pos + DRM_UUID_LEN].copy_from_slice(v);
        self.pos += DRM_UUID_LEN;
    }
    fn bytes(&mut self, v: &[u8]) {
        self.buf[self.pos..self.pos + v.len()].copy_from_slice(v);
        self.pos += v.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID_A: [u8; DRM_UUID_LEN] = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE,
        0xFF,
    ];
    const UUID_B: [u8; DRM_UUID_LEN] = [0xFF; DRM_UUID_LEN];

    #[test]
    fn sd_info_req_round_trips() {
        let bytes = SdInfoReq.to_bytes();
        assert_eq!(bytes, [0x9F, 0x98, 0x00, 0x00]);
        assert_eq!(SdInfoReq::parse(&bytes).unwrap(), SdInfoReq);
    }

    #[test]
    fn sd_info_reply_round_trips_and_bites() {
        let r = SdInfoReply {
            drm_system_ids: alloc::vec![0x4AD4, 0x1234],
            drm_uuids: alloc::vec![UUID_A, UUID_B],
        };
        let bytes = r.to_bytes();
        // body: n_sys(02) 4A D4 12 34, n_uuid(02), 16+16 uuid bytes.
        // body_len = 1 + 4 + 1 + 32 = 38 = 0x26.
        assert_eq!(bytes[0..4], [0x9F, 0x98, 0x01, 0x26]);
        assert_eq!(bytes[4], 0x02);
        assert_eq!(&bytes[5..9], &[0x4A, 0xD4, 0x12, 0x34]);
        assert_eq!(bytes[9], 0x02);
        assert_eq!(&bytes[10..26], &UUID_A);
        assert_eq!(&bytes[26..42], &UUID_B);
        assert_eq!(SdInfoReply::parse(&bytes).unwrap(), r);
        let mut other = r.clone();
        other.drm_system_ids[0] = 0x0000;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn sd_info_reply_empty_loops() {
        let r = SdInfoReply {
            drm_system_ids: Vec::new(),
            drm_uuids: Vec::new(),
        };
        let bytes = r.to_bytes();
        assert_eq!(bytes, [0x9F, 0x98, 0x01, 0x02, 0x00, 0x00]);
        assert_eq!(SdInfoReply::parse(&bytes).unwrap(), r);
    }

    #[test]
    fn sd_start_ts_flag_round_trips_and_bites() {
        let s = SdStart {
            lts_id: 0x07,
            program_number: 0x0042,
            payload: SamplePayload::Ts(alloc::vec![DrmMetadataRecord {
                drm_metadata_source: 0x03, // pssh
                drm_system_id: 0xFFFF,
                drm_uuid: UUID_A,
                drm_metadata: &[0xDE, 0xAD, 0xBE, 0xEF],
            }]),
        };
        let bytes = s.to_bytes();
        // LTS_id(07) prog(00 42) ts_flag-byte(01) n_records(01)
        //   source(03) sysid(FF FF) uuid(16) len(00 04) data(DE AD BE EF)
        // body_len = 1+2+1 + 1 + (1+2+16+2+4) = 5 + 26 = 30... let's just check head.
        assert_eq!(bytes[0..3], [0x9F, 0x98, 0x02]);
        assert_eq!(bytes[4], 0x07); // LTS_id
        assert_eq!(&bytes[5..7], &[0x00, 0x42]); // program_number
        assert_eq!(bytes[7], 0x01); // ts_flag
        assert_eq!(bytes[8], 0x01); // number_of_metadata_records
        assert_eq!(bytes[9], 0x03); // drm_metadata_source
        assert_eq!(&bytes[10..12], &[0xFF, 0xFF]); // drm_system_id
        assert_eq!(&bytes[12..28], &UUID_A); // drm_uuid
        assert_eq!(&bytes[28..30], &[0x00, 0x04]); // drm_metadata_length
        assert_eq!(&bytes[30..34], &[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(SdStart::parse(&bytes).unwrap(), s);
        // Mutation: flip a metadata byte.
        let mut other = s.clone();
        other.payload = SamplePayload::Ts(alloc::vec![DrmMetadataRecord {
            drm_metadata_source: 0x03,
            drm_system_id: 0xFFFF,
            drm_uuid: UUID_A,
            drm_metadata: &[0xDE, 0xAD, 0xBE, 0x00],
        }]);
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn sd_start_tracks_two_tracks() {
        let s = SdStart {
            lts_id: 0x01,
            program_number: 0x1000,
            payload: SamplePayload::Tracks(alloc::vec![
                SampleTrack {
                    track_pid: 0x0100,
                    records: alloc::vec![DrmMetadataRecord {
                        drm_metadata_source: 0x01,
                        drm_system_id: 0x4AD4,
                        drm_uuid: UUID_A,
                        drm_metadata: &[0x01, 0x02],
                    }],
                },
                SampleTrack {
                    track_pid: 0x0101,
                    records: Vec::new(),
                },
            ]),
        };
        let bytes = s.to_bytes();
        assert_eq!(bytes[7], 0x00); // ts_flag == 0
        assert_eq!(bytes[8], 0x02); // number_of_Sample_Tracks
                                    // track0: reserved(3)+PID 0x0100 => 0x01 0x00, n_records=01.
        assert_eq!(&bytes[9..11], &[0x01, 0x00]);
        assert_eq!(bytes[11], 0x01);
        assert_eq!(SdStart::parse(&bytes).unwrap(), s);
        // 13-bit PID masking: top 3 bits ignored on the wire.
        let parsed = SdStart::parse(&bytes).unwrap();
        if let SamplePayload::Tracks(t) = &parsed.payload {
            assert_eq!(t[0].track_pid, 0x0100);
            assert_eq!(t[1].track_pid, 0x0101);
            assert_eq!(t.len(), 2);
        } else {
            panic!("expected Tracks");
        }
    }

    #[test]
    fn sd_start_reply_round_trips_and_bites() {
        let r = SdStartReply {
            lts_id: 0x05,
            transmission_status: TransmissionStatus::ReadyToReceive,
            drm_status: DrmStatus::DecryptionPossible,
            drm_system_id: 0x4AD4,
            drm_uuid: UUID_A,
            buffer_size: 5000,
            data_block_size: 0,
        };
        let bytes = r.to_bytes();
        // body_len = 1+1+1+2+16+2+2 = 25 = 0x19.
        assert_eq!(bytes[0..4], [0x9F, 0x98, 0x03, 0x19]);
        assert_eq!(bytes[4], 0x05); // LTS_id
        assert_eq!(bytes[5], 0x00); // transmission_status
        assert_eq!(bytes[6], 0x00); // drm_status
        assert_eq!(&bytes[7..9], &[0x4A, 0xD4]);
        assert_eq!(&bytes[9..25], &UUID_A);
        assert_eq!(&bytes[25..27], &5000u16.to_be_bytes());
        assert_eq!(&bytes[27..29], &[0x00, 0x00]);
        assert_eq!(SdStartReply::parse(&bytes).unwrap(), r);
        let mut other = r;
        other.drm_status = DrmStatus::NoEntitlement;
        assert_eq!(other.to_bytes()[6], 0x02);
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn sd_update_round_trips() {
        let u = SdUpdate {
            lts_id: 0x09,
            payload: SamplePayload::Tracks(alloc::vec![
                SampleTrack {
                    track_pid: 0x0200,
                    records: alloc::vec![DrmMetadataRecord {
                        drm_metadata_source: 0x05,
                        drm_system_id: 0xFFFF,
                        drm_uuid: UUID_B,
                        drm_metadata: &[],
                    }],
                },
                SampleTrack {
                    track_pid: 0x0201,
                    records: Vec::new(),
                },
            ]),
        };
        let bytes = u.to_bytes();
        assert_eq!(bytes[0..3], [0x9F, 0x98, 0x04]);
        assert_eq!(bytes[4], 0x09); // LTS_id
        assert_eq!(bytes[5], 0x00); // ts_flag
        assert_eq!(bytes[6], 0x02); // number_of_Sample_Tracks
        assert_eq!(SdUpdate::parse(&bytes).unwrap(), u);
    }

    #[test]
    fn sd_update_reply_round_trips_and_bites() {
        let r = SdUpdateReply {
            lts_id: 0x03,
            drm_status: DrmStatus::Undetermined,
        };
        let bytes = r.to_bytes();
        assert_eq!(bytes, [0x9F, 0x98, 0x05, 0x02, 0x03, 0x01]);
        assert_eq!(SdUpdateReply::parse(&bytes).unwrap(), r);
        let other = SdUpdateReply {
            lts_id: 0x03,
            drm_status: DrmStatus::DecryptionPossible,
        };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn dispatch_routes_each_tag() {
        let req = SdInfoReq.to_bytes();
        assert!(matches!(
            SampleDecryptionApdu::parse(&req).unwrap(),
            SampleDecryptionApdu::SdInfoReq(_)
        ));
        let reply = SdUpdateReply {
            lts_id: 0,
            drm_status: DrmStatus::DecryptionPossible,
        }
        .to_bytes();
        let parsed = SampleDecryptionApdu::parse(&reply).unwrap();
        assert!(matches!(parsed, SampleDecryptionApdu::SdUpdateReply(_)));
        assert_eq!(parsed.to_bytes(), reply);
        assert!(matches!(
            SampleDecryptionApdu::parse(&[0x9F, 0x98, 0x7E, 0x00]),
            Err(Error::UnexpectedApduTag { .. })
        ));
    }
}
