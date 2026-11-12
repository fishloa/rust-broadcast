//! TS byte-stream resynchronisation helpers for `dvb-stream`.
//!
//! Re-exported from [`mpeg_ts::resync`], which now owns the stateless
//! datagram resync (including the lone-sync-byte acceptance mode) alongside
//! the stateful [`mpeg_ts::resync::TsResync`].

pub use mpeg_ts::ts::{TS_PACKET_SIZE, TS_SYNC_BYTE};
pub use mpeg_ts::resync::{resync, resync_confirmed, unconfirmed_candidate};
