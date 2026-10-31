//! Source-agnostic fetch bookkeeping shared by the segment-pull adapters: small passive state
//! machines with no knowledge of what is being fetched. Each adapter keeps its own failure routing.

pub(crate) mod body;
pub(crate) mod conn;
pub(crate) mod deadlines;
pub(crate) mod failures;
pub(crate) mod inactivity;
pub(crate) mod resume;
pub(crate) mod retry;
pub(crate) mod wall;
pub(crate) mod window;
