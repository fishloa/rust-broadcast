//! RIST Simple Profile (VSF TR-06-1) — reliable internet stream transport.
//!
//! RIST wraps standard RTP/RTCP with ARQ-based retransmission for
//! reliable media delivery over lossy networks. The Simple Profile
//! provides:
//!
//! - RTP payload transport with sequence-number tracking
//! - RTCP-based NACK for selective retransmission (ARQ)
//! - Configurable retransmission buffer depth
//! - Bonding across multiple network paths
//!
//! Both ingest (receiver) and egress (sender) roles are supported.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
