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
//! - **Multi-stream Host Control** (`0x00200081`, base v3 `0x00200043`,
//!   [`multistream_host_control`]) — the tune APDUs `tune_triplet_req` /
//!   `tune_lcn_req` / `tune_ip_req` / `tuner_status_req` / `tuner_status_reply`.
//!   Both resource ids route to the same [`CiPlusResource::MultistreamHostControl`]
//!   kind, carrying the [`multistream_host_control::HostControlMode`] that selects
//!   the `tune_ip_req` reserved-bit budget. `tune_broadcast_req` / `tune_reply` /
//!   `ask_release(_reply)` are deferred to CI Plus V1.3 and not encoded.
//! - **Sample decryption** (`0x00920041`, [`sample_decryption`]) — `sd_info_req` /
//!   `sd_info_reply` / `sd_start(_reply)` / `sd_update(_reply)`, with opaque DRM
//!   metadata / UUID payloads.
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
pub mod multistream_host_control;
pub mod sample_decryption;

// --- Resource identifiers (TS 103 205 resource-summary tables) ---

/// Multi-stream resource — Class 144, Type 1, Version 1 (`0x00900041`).
/// §6.4.2.1 Table 2.
pub const MULTISTREAM: ResourceId = ResourceId(0x0090_0041);
/// Content Control multi-stream resource — Class 140, Type 65, Version 1
/// (`0x008C1041`). §6.4.3.1 Table 6.
pub const CONTENT_CONTROL: ResourceId = ResourceId(0x008C_1041);
/// Multi-stream Host Control resource — Class 32, Type 2, Version 1
/// (`0x00200081`). §6.4.5, Table 17. Based on DVB Host Control v3.
pub const MULTISTREAM_HOST_CONTROL: ResourceId = ResourceId(0x0020_0081);
/// Base DVB Host Control v3 resource — Class 32, Type 1, Version 3
/// (`0x00200043`). §13. The multi-stream tune APDUs derive from this; both ids
/// route to [`CiPlusResource::MultistreamHostControl`], distinguished only by the
/// `tune_ip_req` reserved-bit budget ([`multistream_host_control::HostControlMode`]).
pub const HOST_CONTROL_V3: ResourceId = ResourceId(0x0020_0043);
/// Sample decryption resource — Class 146, Type 1, Version 1 (`0x00920041`).
/// §7.4, Table 30.
pub const SAMPLE_DECRYPTION: ResourceId = ResourceId(0x0092_0041);

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
    /// Multi-stream Host Control — `0x00200081`
    /// ([`MultiStream`](multistream_host_control::HostControlMode::MultiStream))
    /// or the base DVB Host Control v3 `0x00200043`
    /// ([`BaseV3`](multistream_host_control::HostControlMode::BaseV3)). The matched
    /// mode is carried so the `tune_ip_req` reserved-bit budget can be selected.
    MultistreamHostControl(multistream_host_control::HostControlMode),
    /// Sample decryption resource (`0x00920041`).
    SampleDecryption,
}

impl CiPlusResource {
    /// Diagnostic spec token.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Multistream => "multistream",
            Self::ContentControl => "content_control",
            Self::MultistreamHostControl(_) => "multistream_host_control",
            Self::SampleDecryption => "sample_decryption",
        }
    }
}
dvb_common::impl_spec_display!(CiPlusResource);

/// Map a [`ResourceId`] to the CI Plus resource it denotes, or `None` for any
/// resource not dispatched by [`CiPlusApdu`].
#[must_use]
pub fn classify(id: ResourceId) -> Option<CiPlusResource> {
    use multistream_host_control::HostControlMode;
    match id {
        MULTISTREAM => Some(CiPlusResource::Multistream),
        CONTENT_CONTROL => Some(CiPlusResource::ContentControl),
        MULTISTREAM_HOST_CONTROL => Some(CiPlusResource::MultistreamHostControl(
            HostControlMode::MultiStream,
        )),
        HOST_CONTROL_V3 => Some(CiPlusResource::MultistreamHostControl(
            HostControlMode::BaseV3,
        )),
        SAMPLE_DECRYPTION => Some(CiPlusResource::SampleDecryption),
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
pub enum CiPlusApdu<'a> {
    /// Multi-stream resource object (`0x00900041`).
    Multistream(multistream::MultistreamApdu),
    /// Content Control multi-stream object (`0x008C1041`).
    ContentControl(content_control::ContentControlApdu),
    /// Multi-stream Host Control object (`0x00200081` / base v3 `0x00200043`).
    MultistreamHostControl(
        #[cfg_attr(feature = "serde", serde(borrow))]
        multistream_host_control::MultistreamHostControlApdu<'a>,
    ),
    /// Sample decryption object (`0x00920041`).
    SampleDecryption(
        #[cfg_attr(feature = "serde", serde(borrow))] sample_decryption::SampleDecryptionApdu<'a>,
    ),
}

impl<'a> CiPlusApdu<'a> {
    /// Parse a CI Plus APDU, selecting the resource from `resource_id`
    /// ([`classify`]) and then delegating to that resource's object dispatch on
    /// the leading `apdu_tag`.
    ///
    /// Errors with [`Error::UnknownResource`] if `resource_id` is not a CI Plus
    /// resource handled this pass.
    pub fn parse(resource_id: ResourceId, body: &'a [u8]) -> Result<Self> {
        match classify(resource_id) {
            Some(CiPlusResource::Multistream) => Ok(Self::Multistream(
                multistream::MultistreamApdu::parse(body)?,
            )),
            Some(CiPlusResource::ContentControl) => Ok(Self::ContentControl(
                content_control::ContentControlApdu::parse(body)?,
            )),
            Some(CiPlusResource::MultistreamHostControl(mode)) => Ok(Self::MultistreamHostControl(
                multistream_host_control::MultistreamHostControlApdu::parse_mode(body, mode)?,
            )),
            Some(CiPlusResource::SampleDecryption) => Ok(Self::SampleDecryption(
                sample_decryption::SampleDecryptionApdu::parse(body)?,
            )),
            None => Err(Error::UnknownResource {
                resource_id: resource_id.0,
            }),
        }
    }
}

impl dvb_common::Serialize for CiPlusApdu<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::Multistream(o) => o.serialized_len(),
            Self::ContentControl(o) => o.serialized_len(),
            Self::MultistreamHostControl(o) => o.serialized_len(),
            Self::SampleDecryption(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Multistream(o) => o.serialize_into(buf),
            Self::ContentControl(o) => o.serialize_into(buf),
            Self::MultistreamHostControl(o) => o.serialize_into(buf),
            Self::SampleDecryption(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvb_common::Serialize;

    #[test]
    fn classify_fixed_ids() {
        use multistream_host_control::HostControlMode;
        assert_eq!(classify(MULTISTREAM), Some(CiPlusResource::Multistream));
        assert_eq!(
            classify(CONTENT_CONTROL),
            Some(CiPlusResource::ContentControl)
        );
        // The two Host Control ids both map to MultistreamHostControl, carrying
        // the layout mode.
        assert_eq!(
            classify(MULTISTREAM_HOST_CONTROL),
            Some(CiPlusResource::MultistreamHostControl(
                HostControlMode::MultiStream
            ))
        );
        assert_eq!(
            classify(HOST_CONTROL_V3),
            Some(CiPlusResource::MultistreamHostControl(
                HostControlMode::BaseV3
            ))
        );
        assert_eq!(
            classify(SAMPLE_DECRYPTION),
            Some(CiPlusResource::SampleDecryption)
        );
        assert_eq!(classify(ResourceId(0xDEAD_BEEF)), None);
        // The deferred CA Support multi-stream type is intentionally not classified.
        assert_eq!(classify(ResourceId(0x000C_0041)), None);
    }

    #[test]
    fn dispatch_routes_host_control_and_sample_decryption() {
        // tune_lcn_req under the multi-stream host control resource.
        let lcn = [0x9F, 0x84, 0x07, 0x03, 0x01, 0x81, 0x23];
        let hc = CiPlusApdu::parse(MULTISTREAM_HOST_CONTROL, &lcn).unwrap();
        assert!(matches!(
            hc,
            CiPlusApdu::MultistreamHostControl(
                multistream_host_control::MultistreamHostControlApdu::TuneLcnReq(_)
            )
        ));
        assert_eq!(hc.to_bytes(), lcn);

        // The same tune tag arrives on the base-v3 resource id and still routes.
        let hc_v3 = CiPlusApdu::parse(HOST_CONTROL_V3, &lcn).unwrap();
        assert!(matches!(hc_v3, CiPlusApdu::MultistreamHostControl(_)));

        // sd_update_reply under the sample decryption resource.
        let sd = [0x9F, 0x98, 0x05, 0x02, 0x03, 0x01];
        let parsed = CiPlusApdu::parse(SAMPLE_DECRYPTION, &sd).unwrap();
        assert!(matches!(
            parsed,
            CiPlusApdu::SampleDecryption(sample_decryption::SampleDecryptionApdu::SdUpdateReply(_))
        ));
        assert_eq!(parsed.to_bytes(), sd);
    }

    #[test]
    fn tune_tag_9f8409_routes_resource_scoped_not_via_anyapdu() {
        // 0x9F8409 (tune_triplet_req) is a multi-stream host-control tag. The
        // EN 50221 Host Control resource owns 0x9F8400 (tune) / 0x9F8403
        // (ask_release) — distinct values. The resource-scoped CiPlusApdu parses
        // 0x9F8409 only under a host-control resource, proving it routes
        // independently of the global AnyApdu (which has no 0x9F8409 member).
        let triplet = [
            0x9F, 0x84, 0x09, 0x09, 0x05, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x44, 0x00,
        ];
        assert!(matches!(
            CiPlusApdu::parse(MULTISTREAM_HOST_CONTROL, &triplet).unwrap(),
            CiPlusApdu::MultistreamHostControl(_)
        ));
        // Under an unrelated resource (Multistream PID resource), the same tag is
        // not a member and is rejected.
        assert!(matches!(
            CiPlusApdu::parse(MULTISTREAM, &triplet),
            Err(Error::UnexpectedApduTag { .. })
        ));
        // A global EN 50221 tune tag (0x9F8400) is not a member of the
        // host-control resource's tune set either (different object namespace).
        let en_tune = [0x9F, 0x84, 0x00, 0x00];
        assert!(matches!(
            CiPlusApdu::parse(MULTISTREAM_HOST_CONTROL, &en_tune),
            Err(Error::UnexpectedApduTag { .. })
        ));
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
