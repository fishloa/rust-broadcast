//! Source-agnostic fetch bookkeeping shared by the segment-pull adapters: small passive state
//! machines with no knowledge of what is being fetched. Each adapter keeps its own failure routing.

pub(crate) mod conn;
pub(crate) mod retry;
