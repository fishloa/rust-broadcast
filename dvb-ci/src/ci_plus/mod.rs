//! CI Plus extensions (ETSI TS 103 205) — the resource-scoped APDU layer.
//!
//! Like the TS 101 699 extensions ([`crate::ci_ext`]), the CI Plus resources
//! defined by TS 103 205 have **their own apdu_tag namespace** that collides
//! with EN 50221 and TS 101 699 — e.g. the Multi-stream resource's tags live in
//! `0x9F92xx`, Content Control's in `0x9F90xx`, and the extended CA Support
//! `ca_pmt`/`ca_pmt_reply` reuse EN 50221's `0x9F8032`/`0x9F8033` verbatim. The
//! same tag bytes therefore denote different objects depending on which
//! resource's session they arrive on, so they cannot join the global
//! [`crate::AnyApdu`]. This module provides a *resource-scoped* dispatch
//! ([`CiPlusApdu`]): parsing keys on the `resource_identifier()` first
//! ([`classify`]), then the leading `apdu_tag` selects the object within that
//! resource.
//!
//! Spec: ETSI TS 103 205 V1.4.1 — resource IDs in `docs/ts_103_205/resource-ids.md`;
//! per-resource layouts cited in their own module docs.
//!
//! ## Resources implemented this pass
//!
//! - **Multi-stream** (`0x00900041`, [`multistream`]) — `CICAM_multistream_capability`
//!   / `PID_select_req` / `PID_select_reply`.
//! - **Content Control multi-stream** (`0x008C1041`, [`content_control`]) — the
//!   printed-syntax extended APDUs `cc_PIN_reply` / `cc_PIN_event`, plus the SAC
//!   protocol datatype model.
//!
//! ## Not dispatched here
//!
//! - **CA Support multi-stream** ([`ca_support`]) — TS 103 205 does not print a
//!   resource_id for the `resource_type = 2` variant (it defers to CI Plus V1.3),
//!   so its extended `ca_pmt`/`ca_pmt_reply` are standalone typed structs,
//!   directly constructible/parseable but **not** wired into [`CiPlusApdu`].
//! - **CI Plus descriptors** ([`descriptors`]) — Sample-Mode TLV descriptors, not
//!   APDUs, so they are standalone too.

use crate::error::{Error, Result};
use crate::resource::ResourceId;

pub mod ca_support;
pub mod content_control;
pub mod descriptors;
pub mod multistream;

// --- Resource identifiers (TS 103 205 resource-summary tables) ---

/// Multi-stream resource — Class 144, Type 1, Version 1 (`0x00900041`).
/// §6.4.2.1 Table 2.
pub const MULTISTREAM: ResourceId = ResourceId(0x0090_0041);
/// Content Control multi-stream resource — Class 140, Type 65, Version 1
/// (`0x008C1041`). §6.4.3.1 Table 6.
pub const CONTENT_CONTROL: ResourceId = ResourceId(0x008C_1041);

/// The CI Plus resource a [`ResourceId`] denotes, for the resources dispatched by
/// [`CiPlusApdu`]. Returns `None` for any other resource (including the
/// deferred-resource_id CA Support multi-stream type).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CiPlusResource {
    /// Multi-stream resource (`0x00900041`).
    Multistream,
    /// Content Control multi-stream resource (`0x008C1041`).
    ContentControl,
}

impl CiPlusResource {
    /// Diagnostic spec token.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Multistream => "multistream",
            Self::ContentControl => "content_control",
        }
    }
}
dvb_common::impl_spec_display!(CiPlusResource);

/// Map a [`ResourceId`] to the CI Plus resource it denotes, or `None` for any
/// resource not dispatched by [`CiPlusApdu`].
#[must_use]
pub fn classify(id: ResourceId) -> Option<CiPlusResource> {
    match id {
        MULTISTREAM => Some(CiPlusResource::Multistream),
        CONTENT_CONTROL => Some(CiPlusResource::ContentControl),
        _ => None,
    }
}

/// A parsed CI Plus APDU, scoped to the resource it arrived on.
///
/// One variant per resource dispatched this pass; each wraps that resource's own
/// object enum (which dispatches on the leading `apdu_tag`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CiPlusApdu {
    /// Multi-stream resource object (`0x00900041`).
    Multistream(multistream::MultistreamApdu),
    /// Content Control multi-stream object (`0x008C1041`).
    ContentControl(content_control::ContentControlApdu),
}

impl CiPlusApdu {
    /// Parse a CI Plus APDU, selecting the resource from `resource_id`
    /// ([`classify`]) and then delegating to that resource's object dispatch on
    /// the leading `apdu_tag`.
    ///
    /// Errors with [`Error::UnknownResource`] if `resource_id` is not a CI Plus
    /// resource handled this pass.
    pub fn parse(resource_id: ResourceId, body: &[u8]) -> Result<Self> {
        match classify(resource_id) {
            Some(CiPlusResource::Multistream) => Ok(Self::Multistream(
                multistream::MultistreamApdu::parse(body)?,
            )),
            Some(CiPlusResource::ContentControl) => Ok(Self::ContentControl(
                content_control::ContentControlApdu::parse(body)?,
            )),
            None => Err(Error::UnknownResource {
                resource_id: resource_id.0,
            }),
        }
    }
}

impl dvb_common::Serialize for CiPlusApdu {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::Multistream(o) => o.serialized_len(),
            Self::ContentControl(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Multistream(o) => o.serialize_into(buf),
            Self::ContentControl(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvb_common::Serialize;

    #[test]
    fn classify_fixed_ids() {
        assert_eq!(classify(MULTISTREAM), Some(CiPlusResource::Multistream));
        assert_eq!(
            classify(CONTENT_CONTROL),
            Some(CiPlusResource::ContentControl)
        );
        assert_eq!(classify(ResourceId(0xDEAD_BEEF)), None);
        // The deferred CA Support multi-stream type is intentionally not classified.
        assert_eq!(classify(ResourceId(0x000C_0041)), None);
    }

    #[test]
    fn dispatch_routes_multistream_and_content_control() {
        // Multistream PID_select_reply (whole-TS).
        let ms_body = [0x9F, 0x92, 0x02, 0x03, 0x01, 0x00, 0x00];
        let ms = CiPlusApdu::parse(MULTISTREAM, &ms_body).unwrap();
        assert!(matches!(
            ms,
            CiPlusApdu::Multistream(multistream::MultistreamApdu::PidSelectReply(_))
        ));
        assert_eq!(ms.to_bytes(), ms_body);

        // Content Control cc_PIN_reply (unbound).
        let cc_body = [0x9F, 0x90, 0x14, 0x03, 0x00, 0x00, 0x42];
        let cc = CiPlusApdu::parse(CONTENT_CONTROL, &cc_body).unwrap();
        assert!(matches!(
            cc,
            CiPlusApdu::ContentControl(content_control::ContentControlApdu::CcPinReply(_))
        ));
        assert_eq!(cc.to_bytes(), cc_body);
    }

    #[test]
    fn unknown_resource_errors() {
        let body = [0x9F, 0x92, 0x00, 0x00];
        assert!(matches!(
            CiPlusApdu::parse(ResourceId(0x1234_5678), &body),
            Err(Error::UnknownResource { .. })
        ));
    }

    #[test]
    fn ca_pmt_tag_collision_routes_per_resource_independently() {
        // 0x9F8032 (ca_pmt) is an EN 50221 tag AND a CI Plus CA-support tag, AND
        // a TS 101 699-style 0x9F80xx value. Under the Multistream / Content
        // Control resources it is simply *not* a member tag, so resource-scoped
        // dispatch rejects it — proving CiPlusApdu routes independently of the
        // global AnyApdu (which WOULD parse 0x9F8032 as ca_pmt).
        let ca_pmt_like = [0x9F, 0x80, 0x32, 0x00];
        assert!(matches!(
            CiPlusApdu::parse(MULTISTREAM, &ca_pmt_like),
            Err(Error::UnexpectedApduTag { .. })
        ));
        assert!(matches!(
            CiPlusApdu::parse(CONTENT_CONTROL, &ca_pmt_like),
            Err(Error::UnexpectedApduTag { .. })
        ));
        // The global EN 50221 dispatch, by contrast, recognizes 0x9F8032.
        // (We don't construct a full ca_pmt here; the point is the tag space is
        // shared, so only resource-scoping disambiguates.)
    }

    #[test]
    fn tag_9f9200_under_multistream_vs_9f9014_under_content_control() {
        // 9F9200 is a Multistream tag; under Content Control it is unknown.
        let cap = [0x9F, 0x92, 0x00, 0x03, 0x04, 0x00, 0x01];
        assert!(matches!(
            CiPlusApdu::parse(MULTISTREAM, &cap).unwrap(),
            CiPlusApdu::Multistream(multistream::MultistreamApdu::CicamMultistreamCapability(_))
        ));
        assert!(matches!(
            CiPlusApdu::parse(CONTENT_CONTROL, &cap),
            Err(Error::UnexpectedApduTag { .. })
        ));
        // And 9F9014 is a Content Control tag; under Multistream it is unknown.
        let pin = [0x9F, 0x90, 0x14, 0x03, 0x00, 0x00, 0x42];
        assert!(matches!(
            CiPlusApdu::parse(CONTENT_CONTROL, &pin).unwrap(),
            CiPlusApdu::ContentControl(content_control::ContentControlApdu::CcPinReply(_))
        ));
        assert!(matches!(
            CiPlusApdu::parse(MULTISTREAM, &pin),
            Err(Error::UnexpectedApduTag { .. })
        ));
    }
}
