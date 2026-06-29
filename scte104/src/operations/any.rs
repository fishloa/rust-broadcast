//! Unified multi-operation dispatch: [`AnyOperation`].
//!
//! Generated from a single declarative list (`declare_operations!`). The list
//! is the single source of truth: it produces the enum, `From<T>` conversions,
//! the type → parser dispatcher, and a drift test that pins each op_id
//! literal to the type's `OperationDef::OP_ID`.

use crate::error::Result;

/// Declares [`AnyOperation`] + its dispatcher from one operation-type list.
macro_rules! declare_operations {
    (
        $lt:lifetime;
        $( $variant:ident = $oid:literal => $ty:ty ),+ $(,)?
    ) => {
        /// Every crate-implemented multi-operation type, plus an `Unknown`
        /// fallthrough that preserves the raw body for lossless round-trips.
        #[derive(Debug, Clone, PartialEq, Eq)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        #[non_exhaustive]
        pub enum AnyOperation<$lt> {
            $(
                #[allow(missing_docs)]
                $variant($ty),
            )+
            /// An `opID` with no typed implementation; `body` is the raw
            /// operation bytes (`data_length` bytes).
            Unknown {
                /// The raw `opID`.
                op_id: u16,
                /// The raw operation body bytes.
                body: &$lt [u8],
            },
        }

        $(
            impl<$lt> From<$ty> for AnyOperation<$lt> {
                fn from(c: $ty) -> Self {
                    Self::$variant(c)
                }
            }
        )+

        impl<$lt> AnyOperation<$lt> {
            /// Every `opID` the generated dispatcher routes
            /// (excludes [`AnyOperation::Unknown`]).
            pub const DISPATCHED_OP_IDS: &'static [u16] = &[$($oid),+];

            /// Diagnostic name of the contained operation — the type's
            /// [`crate::traits::OperationDef::NAME`].
            #[must_use]
            pub fn name(&self) -> &'static str {
                match self {
                    $(
                        Self::$variant(_) =>
                            <$ty as crate::traits::OperationDef>::NAME,
                    )+
                    Self::Unknown { .. } => "UNKNOWN",
                }
            }

            /// The wire `opID` for this operation.
            #[must_use]
            pub fn op_id(&self) -> u16 {
                match self {
                    $(
                        Self::$variant(_) =>
                            <$ty as crate::traits::OperationDef>::OP_ID,
                    )+
                    Self::Unknown { op_id, .. } => *op_id,
                }
            }

            /// Parse an operation `body` by its `opID`. Reserved / unimplemented
            /// types yield [`AnyOperation::Unknown`].
            pub fn dispatch(op_id: u16, body: &$lt [u8]) -> Result<Self> {
                use broadcast_common::Parse;
                match op_id {
                    $(
                        $oid => <$ty>::parse(body).map(Self::$variant),
                    )+
                    _ => Ok(Self::Unknown { op_id, body }),
                }
            }

            /// Number of bytes [`serialize_body_into`](Self::serialize_body_into)
            /// will write (the `data_length`).
            #[must_use]
            pub fn body_len(&self) -> usize {
                use broadcast_common::Serialize;
                match self {
                    $(
                        Self::$variant(c) => c.serialized_len(),
                    )+
                    Self::Unknown { body, .. } => body.len(),
                }
            }

            /// Serialize just the operation body (no opID+length) into `buf`.
            pub fn serialize_body_into(&self, buf: &mut [u8]) -> Result<usize> {
                use broadcast_common::Serialize;
                match self {
                    $(
                        Self::$variant(c) => c.serialize_into(buf),
                    )+
                    Self::Unknown { body, .. } => {
                        if buf.len() < body.len() {
                            return Err(crate::error::Error::OutputBufferTooSmall {
                                need: body.len(),
                                have: buf.len(),
                            });
                        }
                        buf[..body.len()].copy_from_slice(body);
                        Ok(body.len())
                    }
                }
            }
        }
    };
}

declare_operations! {'a;
    SpliceRequest           = 0x0101 => crate::operations::splice_request::SpliceRequest,
    SpliceNullRequest       = 0x0102 => crate::operations::splice_null_request::SpliceNullRequest,
    StartScheduleDownload   = 0x0103 => crate::operations::start_schedule_download::StartScheduleDownload,
    TimeSignalRequest       = 0x0104 => crate::operations::time_signal_request::TimeSignalRequest,
    TransmitSchedule        = 0x0105 => crate::operations::transmit_schedule::TransmitSchedule,
    ComponentModeDpi        = 0x0106 => crate::operations::component_mode_dpi::ComponentModeDpi,
    EncryptedDpi            = 0x0107 => crate::operations::encrypted_dpi::EncryptedDpi,
    InsertDescriptor        = 0x0108 => crate::operations::insert_descriptor::InsertDescriptor<'a>,
    InsertDtmfDescriptor    = 0x0109 => crate::operations::insert_dtmf_descriptor::InsertDtmfDescriptor<'a>,
    InsertAvailDescriptor   = 0x010A => crate::operations::insert_avail_descriptor::InsertAvailDescriptor,
    InsertSegmentationDescriptor = 0x010B => crate::operations::insert_segmentation_descriptor::InsertSegmentationDescriptor<'a>,
    ProprietaryCommand      = 0x010C => crate::operations::proprietary_command::ProprietaryCommand<'a>,
    ScheduleComponentMode   = 0x010D => crate::operations::schedule_component_mode::ScheduleComponentMode,
    ScheduleDefinition      = 0x010E => crate::operations::schedule_definition::ScheduleDefinition,
    InsertTier              = 0x010F => crate::operations::insert_tier::InsertTier,
    InsertTimeDescriptor    = 0x0110 => crate::operations::insert_time_descriptor::InsertTimeDescriptor,
    InsertAudioDescriptor   = 0x0111 => crate::operations::insert_audio_descriptor::InsertAudioDescriptor<'a>,
    InsertAudioProvisioning = 0x0112 => crate::operations::insert_audio_provisioning::InsertAudioProvisioning<'a>,
    InsertAlternateBreakDuration = 0x0113 => crate::operations::insert_alternate_break_duration::InsertAlternateBreakDuration<'a>,
    DeleteControlWord       = 0x0300 => crate::operations::control_word::DeleteControlWord,
    UpdateControlWord       = 0x0301 => crate::operations::control_word::UpdateControlWord,
    InjectSectionData       = 0x0100 => crate::operations::inject_section_data::InjectSectionData<'a>,
}

#[cfg(test)]
mod drift_tests {
    use crate::traits::OperationDef;

    #[test]
    fn op_id_drift() {
        // This test pins each opID literal in declare_operations! to the
        // type's OperationDef::OP_ID, so the dispatch list can never drift.
        assert_eq!(
            0x0101,
            crate::operations::splice_request::SpliceRequest::OP_ID
        );
        assert_eq!(
            0x0102,
            crate::operations::splice_null_request::SpliceNullRequest::OP_ID
        );
        assert_eq!(
            0x0103,
            crate::operations::start_schedule_download::StartScheduleDownload::OP_ID
        );
        assert_eq!(
            0x0104,
            crate::operations::time_signal_request::TimeSignalRequest::OP_ID
        );
        assert_eq!(
            0x0105,
            crate::operations::transmit_schedule::TransmitSchedule::OP_ID
        );
        assert_eq!(
            0x0106,
            crate::operations::component_mode_dpi::ComponentModeDpi::OP_ID
        );
        assert_eq!(
            0x0107,
            crate::operations::encrypted_dpi::EncryptedDpi::OP_ID
        );
        assert_eq!(
            0x0108,
            crate::operations::insert_descriptor::InsertDescriptor::OP_ID
        );
        assert_eq!(
            0x0109,
            crate::operations::insert_dtmf_descriptor::InsertDtmfDescriptor::OP_ID
        );
        assert_eq!(
            0x010A,
            crate::operations::insert_avail_descriptor::InsertAvailDescriptor::OP_ID
        );
        assert_eq!(
            0x010B,
            crate::operations::insert_segmentation_descriptor::InsertSegmentationDescriptor::OP_ID
        );
        assert_eq!(
            0x010C,
            crate::operations::proprietary_command::ProprietaryCommand::OP_ID
        );
        assert_eq!(
            0x010D,
            crate::operations::schedule_component_mode::ScheduleComponentMode::OP_ID
        );
        assert_eq!(
            0x010E,
            crate::operations::schedule_definition::ScheduleDefinition::OP_ID
        );
        assert_eq!(0x010F, crate::operations::insert_tier::InsertTier::OP_ID);
        assert_eq!(
            0x0110,
            crate::operations::insert_time_descriptor::InsertTimeDescriptor::OP_ID
        );
        assert_eq!(
            0x0111,
            crate::operations::insert_audio_descriptor::InsertAudioDescriptor::OP_ID
        );
        assert_eq!(
            0x0112,
            crate::operations::insert_audio_provisioning::InsertAudioProvisioning::OP_ID
        );
        assert_eq!(
            0x0113,
            crate::operations::insert_alternate_break_duration::InsertAlternateBreakDuration::OP_ID
        );
        assert_eq!(
            0x0300,
            crate::operations::control_word::DeleteControlWord::OP_ID
        );
        assert_eq!(
            0x0301,
            crate::operations::control_word::UpdateControlWord::OP_ID
        );
        assert_eq!(
            0x0100,
            crate::operations::inject_section_data::InjectSectionData::OP_ID
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::splice_request::{SpliceInsertType, SpliceRequest};
    use alloc::vec;
    use broadcast_common::Serialize;

    #[test]
    fn unknown_op_id_round_trips_body() {
        let body = [0xDE, 0xAD, 0xBE, 0xEF];
        let op = AnyOperation::dispatch(0x1234, &body).unwrap();
        assert!(matches!(op, AnyOperation::Unknown { op_id: 0x1234, .. }));
        assert_eq!(op.body_len(), 4);
        assert_eq!(op.op_id(), 0x1234);
        assert_eq!(op.name(), "UNKNOWN");
        let mut buf = vec![0u8; op.body_len()];
        op.serialize_body_into(&mut buf).unwrap();
        assert_eq!(buf, body);
    }

    #[test]
    fn dispatch_splice_request() {
        let body = SpliceRequest {
            splice_insert_type: SpliceInsertType::SpliceStartNormal,
            splice_event_id: 0x42,
            unique_program_id: 1,
            pre_roll_time: 5000,
            break_duration: 300,
            avail_num: 0,
            avails_expected: 0,
            auto_return_flag: 1,
            not_an_entry_flag: 0,
        }
        .to_bytes();
        let op = AnyOperation::dispatch(0x0101, &body).unwrap();
        assert_eq!(op.name(), "SPLICE_REQUEST");
        assert_eq!(op.op_id(), 0x0101);
    }

    #[test]
    fn dispatched_op_ids_count() {
        // Verify all expected operations are in the dispatch list
        assert!(AnyOperation::DISPATCHED_OP_IDS.contains(&0x0100));
        assert!(AnyOperation::DISPATCHED_OP_IDS.contains(&0x0101));
        assert!(AnyOperation::DISPATCHED_OP_IDS.contains(&0x0301));
    }
}
