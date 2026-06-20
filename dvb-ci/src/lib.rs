//! DVB Common Interface (EN 50221) — the host↔CICAM wire protocol.
//!
//! Parses + builds the EN 50221 protocol objects: resource APDUs (Resource
//! Manager, Application Information, CA support incl. `ca_pmt`/`ca_pmt_reply`,
//! Date-Time, MMI), the session-layer SPDUs and transport-layer TPDUs, plus a
//! `CA_PMT` builder that turns a `dvb-si` PMT into the object handed to a CAM.
//!
//! Scope: the wire/protocol layer only. The physical PC-Card transport and CI+
//! crypto (the CC resource) are out of scope. `#![no_std]` (+ `alloc`).
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

extern crate alloc;

// Implemented by story #268 (subagent-built): apdu tags + resource objects
// (ca_pmt/ca_pmt_reply/ca_info/application_info/resource_manager/date_time/mmi),
// spdu session, tpdu framing, and the CA_PMT builder.
