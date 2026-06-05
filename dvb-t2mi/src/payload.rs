//! T2-MI payload types (§5.2.1 - §5.2.12).

pub mod any;
pub mod arbitrary_cells;
pub mod aux_iq;
pub mod bbframe;
pub mod fef_composite;
pub mod fef_iq;
pub mod fef_null;
pub mod fef_subpart;
pub mod individual_addressing;
pub mod l1_current;
pub mod l1_future;
pub mod p2_bias;
pub mod timestamp;

pub use any::AnyPayload;
pub use arbitrary_cells::ArbitraryCellsPayload;
pub use aux_iq::AuxIqPayload;
pub use bbframe::BbframePayload;
pub use fef_composite::FefCompositePayload;
pub use fef_iq::FefIqPayload;
pub use fef_null::FefNullPayload;
pub use fef_null::S1Field;
pub use fef_subpart::{FefSubPartPayload, PrbsType, SubpartVariety};
pub use individual_addressing::{
    AddressingFunctionTag, FunctionEntry, IndividualAddressingPayload,
};
pub use l1_current::{FrequencySource, L1CurrentPayload};
pub use l1_future::L1FuturePayload;
pub use p2_bias::P2BiasPayload;
pub use timestamp::{Bandwidth, T2TimestampPayload};
