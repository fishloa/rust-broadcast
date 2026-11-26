//! Network Change Notify Descriptor — ETSI EN 300 468 §6.4.9 (tag_extension 0x07).
use super::*;
use alloc::vec::Vec;
use broadcast_core::len::{need_in, need_out};

/// Change type — ETSI EN 300 468 §6.4.10 Table 151
/// (`private/specs-verified/dvb/dvb-si/enums/en_300_468/151-change-type-coding.md`).
///
/// 4-bit field. Classifies the nature of the network change event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ChangeType {
    /// `0` — message only.
    MessageOnly,
    /// `1` — minor - default.
    MinorDefault,
    /// `2` — minor - multiplex removed.
    MinorMultiplexRemoved,
    /// `3` — minor - service changed.
    MinorServiceChanged,
    /// `4`–`7` — reserved for future use for other minor changes.
    MinorReserved(u8),
    /// `8` — major - default.
    MajorDefault,
    /// `9` — major - multiplex frequency changed.
    MajorFrequencyChanged,
    /// `10` — major - multiplex coverage changed.
    MajorCoverageChanged,
    /// `11` — major - multiplex added.
    MajorMultiplexAdded,
    /// `12`–`15` — reserved for future use for other major changes.
    MajorReserved(u8),
}

impl ChangeType {
    #[must_use]
    /// Creates a value from a 4-bit wire nibble (upper bits masked off),
    /// preserving every possible value for lossless round-trip.
    pub fn from_u8(v: u8) -> Self {
        match v & CHANGE_TYPE_MASK {
            0 => Self::MessageOnly,
            1 => Self::MinorDefault,
            2 => Self::MinorMultiplexRemoved,
            3 => Self::MinorServiceChanged,
            v @ MINOR_RESERVED_START..=MINOR_RESERVED_END => Self::MinorReserved(v),
            8 => Self::MajorDefault,
            9 => Self::MajorFrequencyChanged,
            10 => Self::MajorCoverageChanged,
            11 => Self::MajorMultiplexAdded,
            v => Self::MajorReserved(v),
        }
    }

    #[must_use]
    /// Returns the 4-bit wire nibble for this value.
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::MessageOnly => 0,
            Self::MinorDefault => 1,
            Self::MinorMultiplexRemoved => 2,
            Self::MinorServiceChanged => 3,
            Self::MinorReserved(v) | Self::MajorReserved(v) => v,
            Self::MajorDefault => 8,
            Self::MajorFrequencyChanged => 9,
            Self::MajorCoverageChanged => 10,
            Self::MajorMultiplexAdded => 11,
        }
    }

    #[must_use]
    /// Returns the spec token for this value.
    pub fn name(self) -> &'static str {
        match self {
            Self::MessageOnly => "message only",
            Self::MinorDefault => "minor - default",
            Self::MinorMultiplexRemoved => "minor - multiplex removed",
            Self::MinorServiceChanged => "minor - service changed",
            Self::MinorReserved(_) => "reserved (minor)",
            Self::MajorDefault => "major - default",
            Self::MajorFrequencyChanged => "major - multiplex frequency changed",
            Self::MajorCoverageChanged => "major - multiplex coverage changed",
            Self::MajorMultiplexAdded => "major - multiplex added",
            Self::MajorReserved(_) => "reserved (major)",
        }
    }
}
broadcast_core::impl_spec_display!(ChangeType, MinorReserved, MajorReserved);

const CELL_HEADER_LEN: usize = 2; // cell_id(16)
const LOOP_LENGTH_LEN: usize = 1; // loop_length(8)
const CHANGE_BASE_LEN: usize = 12; // id(1)+ver(1)+start(5)+dur(3)+packed(1)+msg(1)
const INVARIANT_TS_LEN: usize = 4; // tsid(16)+onid(16)
/// Width of `cell_id`.
const CELL_ID_LEN: usize = core::mem::size_of::<u16>();
/// Width of `change_type`.
const CHANGE_TYPE_MASK: u8 = 0x0F;
/// First reserved minor `change_type`.
const MINOR_RESERVED_START: u8 = 4;
/// Last reserved minor `change_type`.
const MINOR_RESERVED_END: u8 = 7;
/// Width of `start_time_of_change` in bytes.
const START_TIME_LEN: usize = 5;
/// Offset of `start_time_of_change` in a change entry.
const START_TIME_OFFSET: usize = 2;
/// Offset of the low byte of `start_time_of_change`.
const START_TIME_LAST_OFFSET: usize = START_TIME_OFFSET + START_TIME_LEN - 1;
/// Offset of the 5 bytes within the 8-byte big-endian `u64`.
const START_TIME_U64_OFFSET: usize = core::mem::size_of::<u64>() - START_TIME_LEN;
/// Width of `start_time_of_change` in bits.
const START_TIME_BITS: u32 = 40;
/// Width of `change_duration` in bytes.
const DURATION_LEN: usize = 3;
/// Offset of `change_duration` in a change entry.
const DURATION_OFFSET: usize = 7;
/// Offset of the low byte of `change_duration`.
const DURATION_LAST_OFFSET: usize = DURATION_OFFSET + DURATION_LEN - 1;
/// Offset of the 3 bytes within the 4-byte big-endian `u32`.
const DURATION_U64_OFFSET: usize = core::mem::size_of::<u32>() - DURATION_LEN;
/// Width of `change_duration` in bits.
const DURATION_BITS: u32 = 24;
/// Shift of a high byte into a wider integer.
const HIGH_BYTE_SHIFT: u32 = 8;
/// Offset of the packed flags byte in a change entry.
const PACKED_OFFSET: usize = 10;
/// Shift of `receiver_category`.
const RECEIVER_CATEGORY_SHIFT: u32 = 5;
/// Mask of `receiver_category`.
const RECEIVER_CATEGORY_MASK: u8 = 0x07;
/// Width of `receiver_category`.
const RECEIVER_CATEGORY_BITS: u32 = 3;
/// Shift of `invariant_ts_present`.
const INVARIANT_TS_PRESENT_SHIFT: u32 = 4;
/// Offset of `message_id` in a change entry.
const MESSAGE_ID_OFFSET: usize = 11;
/// Offset of `invariant_ts_onid`.
const INVARIANT_ONID_OFFSET: usize = 2;
/// Offset of the 5 `start_time_of_change` bytes within the `u64`.
const START_TIME_U64_END: usize = core::mem::size_of::<u64>();

/// network_change_notify body (Table 149, §6.4.9). The two-level cell/change loop is unfolded.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NetworkChangeNotify {
    /// Per-cell change lists.
    pub cells: Vec<NetworkChangeCell>,
}

/// A cell in the network_change_notify outer loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NetworkChangeCell {
    /// cell_id(16).
    pub cell_id: u16,
    /// The cell's change entries.
    pub changes: Vec<NetworkChange>,
}

/// A change entry in the network_change_notify inner loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NetworkChange {
    /// network_change_id(8).
    pub network_change_id: u8,
    /// network_change_version(8).
    pub network_change_version: u8,
    /// start_time_of_change(40) — raw 40-bit value (MJD date + UTC BCD time), big-endian.
    pub start_time_of_change: u64,
    /// change_duration(24) — raw 24-bit BCD value, big-endian.
    pub change_duration: u32,
    /// receiver_category(3).
    pub receiver_category: u8,
    /// change_type(4) — [`ChangeType`].
    pub change_type: ChangeType,
    /// message_id(8).
    pub message_id: u8,
    /// invariant_ts (tsid, onid), present iff invariant_ts_present==1.
    pub invariant_ts: Option<InvariantTs>,
}

/// Conditional invariant-TS fields in a network_change_notify entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InvariantTs {
    /// invariant_ts_tsid(16).
    pub tsid: u16,
    /// invariant_ts_onid(16).
    pub onid: u16,
}

impl ExtensionBodyDef<'_> for NetworkChangeNotify {
    const TAG_EXTENSION: u8 = 0x07;
    const NAME: &'static str = "NETWORK_CHANGE_NOTIFY";
}

impl NetworkChange {
    /// `start_time_of_change` (40-bit MJD + BCD UTC) as a UTC datetime; `None`
    /// if a field is out of range.
    #[cfg(feature = "jiff")]
    #[must_use]
    pub fn start_time_of_change_utc(&self) -> Option<jiff::Timestamp> {
        let raw = self.start_time_of_change.to_be_bytes();
        let mut mjd = [0u8; START_TIME_LEN];
        mjd.copy_from_slice(&raw[START_TIME_U64_OFFSET..START_TIME_U64_END]);
        broadcast_common::time::decode_mjd_bcd_utc(mjd)
    }

    /// `change_duration` (24-bit BCD `HHMMSS`) in seconds; `None` if a BCD
    /// nibble is not a decimal digit or a field is out of range.
    #[must_use]
    pub fn change_duration_secs(&self) -> Option<u32> {
        let raw = self.change_duration.to_be_bytes();
        let mut dur = [0u8; DURATION_LEN];
        dur.copy_from_slice(&raw[DURATION_U64_OFFSET..]);
        broadcast_common::time::decode_bcd_duration(dur)
            .and_then(|d| u32::try_from(d.as_secs()).ok())
    }
}

fn change_serialized_len(ch: &NetworkChange) -> usize {
    CHANGE_BASE_LEN
        + if ch.invariant_ts.is_some() {
            INVARIANT_TS_LEN
        } else {
            0
        }
}

impl<'a> Parse<'a> for NetworkChangeNotify {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        let mut cells = Vec::new();
        let mut pos = 0;
        while pos < sel.len() {
            need_in(
                sel,
                pos + CELL_HEADER_LEN + LOOP_LENGTH_LEN,
                "network_change_notify body",
            )?;
            let (cell_id_bytes, _) = sel
                .get(pos..)
                .and_then(|s| s.split_first_chunk::<{ CELL_ID_LEN }>())
                .ok_or(Error::BufferTooShort(
                    broadcast_core::error::BufferTooShort {
                        need: pos + CELL_HEADER_LEN + LOOP_LENGTH_LEN,
                        have: sel.len(),
                        what: "network_change_notify body",
                    },
                ))?;
            let cell_id = u16::from_be_bytes(*cell_id_bytes);
            let loop_length = sel[pos + CELL_HEADER_LEN] as usize;
            pos += CELL_HEADER_LEN + LOOP_LENGTH_LEN;

            need_in(sel, pos + loop_length, "network_change_notify body")?;

            let inner_end = pos + loop_length;
            let mut changes = Vec::new();
            while pos < inner_end {
                let remaining = inner_end - pos;
                // At minimum we need CHANGE_BASE_LEN bytes for a basic entry.
                if remaining < CHANGE_BASE_LEN {
                    return Err(Error::BufferTooShort(
                        broadcast_core::error::BufferTooShort {
                            need: inner_end - remaining + CHANGE_BASE_LEN,
                            have: sel.len(),
                            what: "network_change_notify body",
                        },
                    ));
                }
                let network_change_id = sel[pos];
                let network_change_version = sel[pos + 1];
                let start_time_of_change = (u64::from(crate::wire::be_u32(
                    sel,
                    pos + START_TIME_OFFSET,
                    "start_time_of_change",
                )?) << HIGH_BYTE_SHIFT)
                    | u64::from(sel[pos + START_TIME_LAST_OFFSET]);
                let change_duration = (u32::from(crate::wire::be_u16(
                    sel,
                    pos + DURATION_OFFSET,
                    "change_duration",
                )?) << HIGH_BYTE_SHIFT)
                    | u32::from(sel[pos + DURATION_LAST_OFFSET]);
                let packed = sel[pos + PACKED_OFFSET];
                let receiver_category = packed >> RECEIVER_CATEGORY_SHIFT;
                let invariant_ts_present = (packed >> INVARIANT_TS_PRESENT_SHIFT) & 1;
                let change_type = ChangeType::from_u8(packed & CHANGE_TYPE_MASK);
                let message_id = sel[pos + MESSAGE_ID_OFFSET];
                pos += CHANGE_BASE_LEN;

                let invariant_ts = if invariant_ts_present == 1 {
                    if pos + INVARIANT_TS_LEN > inner_end {
                        return Err(Error::BufferTooShort(
                            broadcast_core::error::BufferTooShort {
                                need: pos + INVARIANT_TS_LEN,
                                have: sel.len(),
                                what: "network_change_notify body",
                            },
                        ));
                    }
                    let (inv_bytes, _) = sel
                        .get(pos..)
                        .and_then(|s| s.split_first_chunk::<INVARIANT_TS_LEN>())
                        .ok_or(Error::BufferTooShort(
                            broadcast_core::error::BufferTooShort {
                                need: pos + INVARIANT_TS_LEN,
                                have: sel.len(),
                                what: "network_change_notify body",
                            },
                        ))?;
                    let ts = InvariantTs {
                        tsid: crate::wire::be_u16(inv_bytes, 0, "invariant_ts_tsid")?,
                        onid: crate::wire::be_u16(
                            inv_bytes,
                            INVARIANT_ONID_OFFSET,
                            "invariant_ts_onid",
                        )?,
                    };
                    pos += INVARIANT_TS_LEN;
                    Some(ts)
                } else {
                    None
                };

                changes.push(NetworkChange {
                    network_change_id,
                    network_change_version,
                    start_time_of_change,
                    change_duration,
                    receiver_category,
                    change_type,
                    message_id,
                    invariant_ts,
                });
            }

            if pos != inner_end {
                return Err(invalid("network_change_notify: change entry overruns loop"));
            }

            cells.push(NetworkChangeCell { cell_id, changes });
        }
        Ok(NetworkChangeNotify { cells })
    }
}

impl Serialize for NetworkChangeNotify {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        self.cells
            .iter()
            .map(|cell| {
                CELL_HEADER_LEN
                    + LOOP_LENGTH_LEN
                    + cell
                        .changes
                        .iter()
                        .map(change_serialized_len)
                        .sum::<usize>()
            })
            .sum()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        let mut pos = 0;
        for cell in &self.cells {
            buf[pos..pos + CELL_ID_LEN].copy_from_slice(&cell.cell_id.to_be_bytes());
            pos += CELL_ID_LEN;
            let loop_length: usize = cell.changes.iter().map(change_serialized_len).sum();
            let loop_length =
                broadcast_core::len::fit_bits(loop_length as u64, u8::BITS, "loop_length")?
                    as usize;
            buf[pos] = loop_length as u8;
            pos += 1;
            for ch in &cell.changes {
                let st = broadcast_core::len::fit_bits(
                    ch.start_time_of_change,
                    START_TIME_BITS,
                    "start_time_of_change",
                )?;
                let dur = broadcast_core::len::fit_bits(
                    u64::from(ch.change_duration),
                    DURATION_BITS,
                    "change_duration",
                )?;
                let receiver_category = broadcast_core::len::fit_bits(
                    u64::from(ch.receiver_category),
                    RECEIVER_CATEGORY_BITS,
                    "receiver_category",
                )? as u8;
                buf[pos] = ch.network_change_id;
                buf[pos + 1] = ch.network_change_version;
                buf[pos + START_TIME_OFFSET..pos + START_TIME_OFFSET + START_TIME_LEN]
                    .copy_from_slice(&st.to_be_bytes()[START_TIME_U64_OFFSET..START_TIME_U64_END]);
                let dur = dur as u32;
                buf[pos + DURATION_OFFSET..pos + DURATION_OFFSET + DURATION_LEN]
                    .copy_from_slice(&dur.to_be_bytes()[DURATION_U64_OFFSET..]);
                let packed = (broadcast_core::len::fit_mask(
                    receiver_category,
                    RECEIVER_CATEGORY_MASK,
                    "receiver_category",
                )? << RECEIVER_CATEGORY_SHIFT)
                    | ((ch.invariant_ts.is_some() as u8) << INVARIANT_TS_PRESENT_SHIFT)
                    | broadcast_core::len::fit_mask(
                        ch.change_type.to_u8(),
                        CHANGE_TYPE_MASK,
                        "change_type",
                    )?;
                buf[pos + PACKED_OFFSET] = packed;
                buf[pos + MESSAGE_ID_OFFSET] = ch.message_id;
                pos += CHANGE_BASE_LEN;
                if let Some(ref inv) = ch.invariant_ts {
                    buf[pos..pos + CELL_ID_LEN].copy_from_slice(&inv.tsid.to_be_bytes());
                    buf[pos + INVARIANT_ONID_OFFSET..pos + INVARIANT_ONID_OFFSET + CELL_ID_LEN]
                        .copy_from_slice(&inv.onid.to_be_bytes());
                    pos += INVARIANT_TS_LEN;
                }
            }
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor, ExtensionTag};
    use alloc::vec;

    #[test]
    fn parse_network_change_notify_structured() {
        // 2 cells: one with 0 changes, one with 2 changes (one without
        // invariant_ts, one with).
        let sel = [
            // cell 1: cell_id=0x0001, loop_length=0
            0x00, 0x01, 0x00, // cell 2: cell_id=0x0002, loop_length=28 (0x1C)
            0x00, 0x02, 0x1C,
            //   change 1 (no invariant_ts): 12 bytes
            0x10, // network_change_id
            0x20, // network_change_version
            0x00, 0x00, 0x00, 0x00, 0x01, // start_time_of_change = 1
            0x00, 0x00, 0x01, // change_duration = 1
            0x23, // packed: rec_cat=1, inv_ts=0, change_type=3
            0x40, // message_id
            //   change 2 (with invariant_ts): 16 bytes
            0x30, // network_change_id
            0x40, // network_change_version
            0x00, 0x00, 0x00, 0x00, 0x02, // start_time_of_change = 2
            0x00, 0x00, 0x02, // change_duration = 2
            0x54, // packed: rec_cat=2, inv_ts=1, change_type=4
            0x50, // message_id
            0xAA, 0xAA, // tsid = 0xAAAA
            0xBB, 0xBB, // onid = 0xBBBB
        ];
        let bytes = wrap(0x07, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::NetworkChangeNotify(b) => {
                assert_eq!(b.cells.len(), 2);

                assert_eq!(b.cells[0].cell_id, 0x0001);
                assert!(b.cells[0].changes.is_empty());

                assert_eq!(b.cells[1].cell_id, 0x0002);
                assert_eq!(b.cells[1].changes.len(), 2);

                let ch0 = &b.cells[1].changes[0];
                assert_eq!(ch0.network_change_id, 0x10);
                assert_eq!(ch0.network_change_version, 0x20);
                assert_eq!(ch0.start_time_of_change, 1);
                assert_eq!(ch0.change_duration, 1);
                assert_eq!(ch0.receiver_category, 1);
                assert_eq!(ch0.change_type, ChangeType::MinorServiceChanged);
                assert_eq!(ch0.message_id, 0x40);
                assert!(ch0.invariant_ts.is_none());

                let ch1 = &b.cells[1].changes[1];
                assert_eq!(ch1.network_change_id, 0x30);
                assert_eq!(ch1.network_change_version, 0x40);
                assert_eq!(ch1.start_time_of_change, 2);
                assert_eq!(ch1.change_duration, 2);
                assert_eq!(ch1.receiver_category, 2);
                assert_eq!(ch1.change_type, ChangeType::MinorReserved(4));
                assert_eq!(ch1.message_id, 0x50);
                let inv = ch1.invariant_ts.as_ref().unwrap();
                assert_eq!(inv.tsid, 0xAAAA);
                assert_eq!(inv.onid, 0xBBBB);
            }
            other => panic!("expected NetworkChangeNotify, got {other:?}"),
        }
        round_trip(&d);
    }

    /// Byte-exact cross-check against a TSDuck-compiled descriptor (tsduck-test
    /// test-015). Parse, assert decoded fields, assert kind(), then re-serialize
    /// and verify byte-exact match.
    #[test]
    fn network_change_notify_tsduck_byte_exact() {
        let bytes = test_fixtures::hex_bytes(
            "7f230712340056781cabcde5cc2312340852030281ef67e5e20234561132453b83deadbeef",
        );
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::NetworkChangeNotify));

        match &d.body {
            ExtensionBody::NetworkChangeNotify(b) => {
                assert_eq!(b.cells.len(), 2);

                // cell 0: cell_id=0x1234, no changes
                assert_eq!(b.cells[0].cell_id, 0x1234);
                assert!(b.cells[0].changes.is_empty());

                // cell 1: cell_id=0x5678, 2 changes
                assert_eq!(b.cells[1].cell_id, 0x5678);
                assert_eq!(b.cells[1].changes.len(), 2);

                // change 0: no invariant_ts
                let ch0 = &b.cells[1].changes[0];
                assert_eq!(ch0.network_change_id, 0xAB);
                assert_eq!(ch0.network_change_version, 0xCD);
                assert_eq!(ch0.start_time_of_change, 0xE5CC231234);
                assert_eq!(ch0.change_duration, 0x085203);
                assert_eq!(ch0.receiver_category, 0);
                assert_eq!(ch0.change_type, ChangeType::MinorMultiplexRemoved);
                assert_eq!(ch0.message_id, 0x81);
                assert!(ch0.invariant_ts.is_none());

                // change 1: with invariant_ts
                let ch1 = &b.cells[1].changes[1];
                assert_eq!(ch1.network_change_id, 0xEF);
                assert_eq!(ch1.network_change_version, 0x67);
                assert_eq!(ch1.start_time_of_change, 0xE5E2023456);
                assert_eq!(ch1.change_duration, 0x113245);
                assert_eq!(ch1.change_duration_secs(), Some(11 * 3600 + 32 * 60 + 45));
                #[cfg(feature = "jiff")]
                {
                    let start = jiff::tz::TimeZone::UTC
                        .to_datetime(ch1.start_time_of_change_utc().expect("valid MJD+BCD"));
                    assert_eq!((start.year(), start.month(), start.day()), (2020, 1, 2));
                    assert_eq!((start.hour(), start.minute(), start.second()), (2, 34, 56));
                }
                assert_eq!(ch1.receiver_category, 1);
                assert_eq!(ch1.change_type, ChangeType::MajorMultiplexAdded);
                assert_eq!(ch1.message_id, 0x83);
                let inv = ch1.invariant_ts.as_ref().unwrap();
                assert_eq!(inv.tsid, 0xDEAD);
                assert_eq!(inv.onid, 0xBEEF);
            }
            other => panic!("expected NetworkChangeNotify, got {other:?}"),
        }

        let mut out = vec![0u8; d.serialized_len()];
        let n = d.serialize_into(&mut out).unwrap();
        assert_eq!(
            out[..n],
            bytes[..],
            "byte-exact re-serialize for TSDuck vector"
        );
    }

    #[test]
    fn change_duration_rejects_non_bcd_nibbles() {
        let change = |change_duration| NetworkChange {
            network_change_id: 0,
            network_change_version: 0,
            start_time_of_change: 0,
            change_duration,
            receiver_category: 0,
            change_type: ChangeType::MessageOnly,
            message_id: 0,
            invariant_ts: None,
        };
        assert_eq!(change(0x013045).change_duration_secs(), Some(5445));
        assert_eq!(change(0x01_1A_00).change_duration_secs(), None);
    }

    #[test]
    fn change_type_full_range_round_trip() {
        for v in 0u8..=0x0F {
            let ct = ChangeType::from_u8(v);
            assert_eq!(ct.to_u8(), v, "ChangeType round-trip failed for {v}");
        }
    }

    #[test]
    fn change_type_known_values() {
        assert_eq!(ChangeType::from_u8(0), ChangeType::MessageOnly);
        assert_eq!(ChangeType::from_u8(1), ChangeType::MinorDefault);
        assert_eq!(ChangeType::from_u8(4), ChangeType::MinorReserved(4));
        assert_eq!(ChangeType::from_u8(8), ChangeType::MajorDefault);
        assert_eq!(ChangeType::from_u8(9), ChangeType::MajorFrequencyChanged);
        assert_eq!(ChangeType::from_u8(10), ChangeType::MajorCoverageChanged);
        assert_eq!(ChangeType::from_u8(11), ChangeType::MajorMultiplexAdded);
        assert_eq!(ChangeType::from_u8(12), ChangeType::MajorReserved(12));
        assert_eq!(ChangeType::MessageOnly.name(), "message only");
        assert_eq!(ChangeType::MajorDefault.name(), "major - default");
        assert_eq!(ChangeType::MinorReserved(5).name(), "reserved (minor)");
        assert_eq!(ChangeType::MajorReserved(13).name(), "reserved (major)");
    }

    #[test]
    fn serialize_rejects_truncatable_fields() {
        // change_duration > 24 bits was silently clipped by `(dur >> 16) as u8`
        // chain; receiver_category > 3 bits by `& 0x07` (r03-W3).
        let change = NetworkChange {
            network_change_id: 1,
            network_change_version: 0,
            start_time_of_change: 1 << 40,
            change_duration: 0,
            receiver_category: 0,
            change_type: ChangeType::MinorDefault,
            message_id: 0,
            invariant_ts: None,
        };
        let d = NetworkChangeNotify {
            cells: alloc::vec![NetworkChangeCell {
                cell_id: 0,
                changes: alloc::vec![change.clone()],
            }],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
        let mut c2 = change.clone();
        c2.start_time_of_change = 0;
        c2.change_duration = 1 << 24;
        let d = NetworkChangeNotify {
            cells: alloc::vec![NetworkChangeCell {
                cell_id: 0,
                changes: alloc::vec![c2],
            }],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
        let mut c3 = change;
        c3.change_duration = 0;
        c3.receiver_category = 8;
        let d = NetworkChangeNotify {
            cells: alloc::vec![NetworkChangeCell {
                cell_id: 0,
                changes: alloc::vec![c3],
            }],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
    }
}
