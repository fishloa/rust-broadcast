//! Source-agnostic fetch bookkeeping shared by the segment-pull adapters: small passive state
//! machines with no knowledge of what is being fetched. Each adapter keeps its own failure routing.
//!
//! ```rust
//! use broadcast_client::fetch::window::Window;
//! let mut window = Window::<u32>::new(2);
//! assert!(window.is_open());
//! ```

pub mod body;
/// Fetch bookkeeping helper.
pub mod conn;
pub mod deadlines;
pub mod failures;
pub mod inactivity;
pub mod inflight;
pub mod resume;
pub mod retry;
pub mod wall;
pub mod window;
