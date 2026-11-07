//! insert_avail_descriptor_request_data() — ANSI/SCTE 104 2023 §9.8.4, Table 9-26 (opID 0x010A).
//!
//! Supplemental usage. Adds an avail_descriptor to the resulting SCTE 35 section.

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use broadcast_common::{Parse, Serialize};

/// `opID` for insert_avail_descriptor_request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x010A;

/// insert_avail_descriptor_request_data() — §9.8.4, Table 9-26.
///
/// `num_provider_avails` is not a stored field: it is always
/// `provider_avail_ids.len()`, written through `broadcast_common::len::fit_u8`
/// on serialize (audit run-09 S4-W1) — the same class of bug as
/// `InsertDescriptor`'s former `descriptor_count` field.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InsertAvailDescriptor {
    /// Loop of `provider_avail_id` (4 bytes each).
    pub provider_avail_ids: Vec<u32>,
}

impl InsertAvailDescriptor {
    /// The `num_provider_avails` value that will be written on serialize:
    /// `provider_avail_ids.len()`.
    #[must_use]
    pub fn num_provider_avails(&self) -> usize {
        self.provider_avail_ids.len()
    }
}

impl<'a> Parse<'a> for InsertAvailDescriptor {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Ok(Self {
            provider_avail_ids: super::parse_provider_avail_ids(
                bytes,
                "insert_avail_descriptor num_provider_avails",
                "insert_avail_descriptor provider_avail_ids",
            )?,
        })
    }
}

impl Serialize for InsertAvailDescriptor {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::provider_avail_ids_len(&self.provider_avail_ids)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        super::write_provider_avail_ids(
            &self.provider_avail_ids,
            buf,
            "insert_avail_descriptor.num_provider_avails",
        )
    }
}

impl OperationDef<'_> for InsertAvailDescriptor {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "INSERT_AVAIL_DESCRIPTOR";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let op = InsertAvailDescriptor {
            provider_avail_ids: alloc::vec![0xAAAA_BBBB, 0xCCCC_DDDD],
        };
        assert_eq!(op.num_provider_avails(), 2);
        let bytes = op.try_to_bytes().unwrap();
        assert_eq!(bytes.len(), 9);
        assert_eq!(bytes[0], 2);
        let back = InsertAvailDescriptor::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn zero_avails_round_trip() {
        let op = InsertAvailDescriptor {
            provider_avail_ids: alloc::vec![],
        };
        let bytes = op.try_to_bytes().unwrap();
        assert_eq!(bytes.len(), 1);
        let back = InsertAvailDescriptor::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn mutate_field_changes_output() {
        let op = InsertAvailDescriptor {
            provider_avail_ids: alloc::vec![0xAAAA_BBBB],
        };
        let bytes = op.try_to_bytes().unwrap();
        let mut op2 = op.clone();
        op2.provider_avail_ids[0] = 0x1111_2222;
        assert_ne!(op2.try_to_bytes().unwrap(), bytes);
    }

    /// S4-W1/S4-W3 (#1103): `num_provider_avails` is derived from
    /// `provider_avail_ids.len()`, so over 255 ids is rejected rather than
    /// silently wrapping the count byte.
    #[test]
    fn over_255_avails_rejected_not_wrapped() {
        let op = InsertAvailDescriptor {
            provider_avail_ids: alloc::vec![0xAAAA_BBBB; 256],
        };
        assert_eq!(
            op.try_to_bytes().err(),
            Some(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                field: "insert_avail_descriptor.num_provider_avails",
                value: 256,
                max: 255
            }))
        );
    }
}
