//! Network Change Notify Descriptor — ETSI EN 300 468 §6.4.9 (tag_extension 0x07).
use super::*;

const CELL_HEADER_LEN: usize = 2; // cell_id(16)
const LOOP_LENGTH_LEN: usize = 1; // loop_length(8)
const CHANGE_BASE_LEN: usize = 12; // id(1)+ver(1)+start(5)+dur(3)+packed(1)+msg(1)
const INVARIANT_TS_LEN: usize = 4; // tsid(16)+onid(16)

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
    /// change_type(4).
    pub change_type: u8,
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

impl super::sealed::Sealed for NetworkChangeNotify {}
impl ExtensionBodyDef for NetworkChangeNotify {
    const TAG_EXTENSION: u8 = 0x07;
    const NAME: &'static str = "NETWORK_CHANGE_NOTIFY";
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
            if pos + CELL_HEADER_LEN + LOOP_LENGTH_LEN > sel.len() {
                return Err(invalid("network_change_notify: cell header truncated"));
            }
            let cell_id = u16::from_be_bytes([sel[pos], sel[pos + 1]]);
            let loop_length = sel[pos + CELL_HEADER_LEN] as usize;
            pos += CELL_HEADER_LEN + LOOP_LENGTH_LEN;

            if pos + loop_length > sel.len() {
                return Err(invalid("network_change_notify: inner loop overruns body"));
            }

            let inner_end = pos + loop_length;
            let mut changes = Vec::new();
            while pos < inner_end {
                let remaining = inner_end - pos;
                // At minimum we need CHANGE_BASE_LEN bytes for a basic entry.
                if remaining < CHANGE_BASE_LEN {
                    return Err(invalid("network_change_notify: change entry overruns loop"));
                }
                let network_change_id = sel[pos];
                let network_change_version = sel[pos + 1];
                let start_time_of_change = (u64::from(sel[pos + 2]) << 32)
                    | (u64::from(sel[pos + 3]) << 24)
                    | (u64::from(sel[pos + 4]) << 16)
                    | (u64::from(sel[pos + 5]) << 8)
                    | u64::from(sel[pos + 6]);
                let change_duration = (u32::from(sel[pos + 7]) << 16)
                    | (u32::from(sel[pos + 8]) << 8)
                    | u32::from(sel[pos + 9]);
                let packed = sel[pos + 10];
                let receiver_category = packed >> 5;
                let invariant_ts_present = (packed >> 4) & 1;
                let change_type = packed & 0x0F;
                let message_id = sel[pos + 11];
                pos += CHANGE_BASE_LEN;

                let invariant_ts = if invariant_ts_present == 1 {
                    if pos + INVARIANT_TS_LEN > inner_end {
                        return Err(invalid("network_change_notify: change entry overruns loop"));
                    }
                    let ts = InvariantTs {
                        tsid: u16::from_be_bytes([sel[pos], sel[pos + 1]]),
                        onid: u16::from_be_bytes([sel[pos + 2], sel[pos + 3]]),
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
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let mut pos = 0;
        for cell in &self.cells {
            buf[pos..pos + 2].copy_from_slice(&cell.cell_id.to_be_bytes());
            pos += 2;
            let loop_length: usize = cell.changes.iter().map(change_serialized_len).sum();
            buf[pos] = loop_length as u8;
            pos += 1;
            for ch in &cell.changes {
                buf[pos] = ch.network_change_id;
                buf[pos + 1] = ch.network_change_version;
                let st = ch.start_time_of_change;
                buf[pos + 2] = (st >> 32) as u8;
                buf[pos + 3] = (st >> 24) as u8;
                buf[pos + 4] = (st >> 16) as u8;
                buf[pos + 5] = (st >> 8) as u8;
                buf[pos + 6] = st as u8;
                let dur = ch.change_duration;
                buf[pos + 7] = (dur >> 16) as u8;
                buf[pos + 8] = (dur >> 8) as u8;
                buf[pos + 9] = dur as u8;
                let packed = ((ch.receiver_category & 0x07) << 5)
                    | ((ch.invariant_ts.is_some() as u8) << 4)
                    | (ch.change_type & 0x0F);
                buf[pos + 10] = packed;
                buf[pos + 11] = ch.message_id;
                pos += CHANGE_BASE_LEN;
                if let Some(ref inv) = ch.invariant_ts {
                    buf[pos..pos + 2].copy_from_slice(&inv.tsid.to_be_bytes());
                    buf[pos + 2..pos + 4].copy_from_slice(&inv.onid.to_be_bytes());
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
                assert_eq!(ch0.change_type, 3);
                assert_eq!(ch0.message_id, 0x40);
                assert!(ch0.invariant_ts.is_none());

                let ch1 = &b.cells[1].changes[1];
                assert_eq!(ch1.network_change_id, 0x30);
                assert_eq!(ch1.network_change_version, 0x40);
                assert_eq!(ch1.start_time_of_change, 2);
                assert_eq!(ch1.change_duration, 2);
                assert_eq!(ch1.receiver_category, 2);
                assert_eq!(ch1.change_type, 4);
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
        let bytes =
            from_hex("7f230712340056781cabcde5cc2312340852030281ef67e5e20234561132453b83deadbeef");
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
                assert_eq!(ch0.change_type, 2);
                assert_eq!(ch0.message_id, 0x81);
                assert!(ch0.invariant_ts.is_none());

                // change 1: with invariant_ts
                let ch1 = &b.cells[1].changes[1];
                assert_eq!(ch1.network_change_id, 0xEF);
                assert_eq!(ch1.network_change_version, 0x67);
                assert_eq!(ch1.start_time_of_change, 0xE5E2023456);
                assert_eq!(ch1.change_duration, 0x113245);
                assert_eq!(ch1.receiver_category, 1);
                assert_eq!(ch1.change_type, 0xB);
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
}
