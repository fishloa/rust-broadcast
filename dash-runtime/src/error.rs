//! Typed errors for MPD parsing.
//!
//! The error type itself is the manifest reader's ([`broadcast_dash::error`]);
//! it is re-exported here under the names players already import.

pub use broadcast_dash::error::{Limit, MpdError, Unsupported};
