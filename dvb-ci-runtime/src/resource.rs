//! The resource layer — application-layer state machines (ETSI EN 50221 §8),
//! one per resource, driven by the session layer's APDUs.
//!
//! Each resource implements [`Resource`]: it reacts to its session opening and
//! to incoming APDUs, producing APDUs to send back, host [`Notification`]s, and
//! requests to open further (module-provided) resources. This module ships the
//! mandatory [`ResourceManager`]; application_information / conditional_access /
//! date_time / mmi land as further `Resource` impls.

use std::time::Duration;

use dvb_ci::objects::application_info::{ApplicationInfo, ApplicationInfoEnq};
use dvb_ci::objects::ca_info::{CaInfo, CaInfoEnq};
use dvb_ci::objects::ca_pmt_reply::{CaEnable, CaPmtReply};
use dvb_ci::objects::resource_manager::{Profile, ProfileEnq};
use dvb_ci::resource::{
    ResourceId, APPLICATION_INFORMATION, CONDITIONAL_ACCESS_SUPPORT, MMI, RESOURCE_MANAGER,
};
use dvb_ci::tag::{self, ApduTag};
use dvb_common::{Parse, Serialize};

use crate::event::Notification;

pub(crate) fn ser<S: Serialize>(s: &S) -> Vec<u8> {
    let mut b = vec![0u8; s.serialized_len()];
    match s.serialize_into(&mut b) {
        Ok(n) => b.truncate(n),
        Err(_) => b.clear(),
    }
    b
}

/// The 3-byte `apdu_tag` at the start of an APDU, if present.
pub(crate) fn peek_tag(apdu: &[u8]) -> Option<ApduTag> {
    (apdu.len() >= 3).then(|| ApduTag::from_bytes(apdu[0], apdu[1], apdu[2]))
}

/// What a resource wants done after reacting to an input.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ResourceOut {
    /// APDUs to send on this resource's session.
    pub apdus: Vec<Vec<u8>>,
    /// Host-facing notifications.
    pub notify: Vec<Notification>,
    /// Module-provided resources the host should now open (`create_session`).
    pub open: Vec<ResourceId>,
}

/// An EN 50221 application-layer resource.
pub trait Resource {
    /// The resource this handler serves.
    fn id(&self) -> ResourceId;
    /// The session for this resource just opened.
    fn on_open(&mut self) -> ResourceOut {
        ResourceOut::default()
    }
    /// An APDU arrived on this resource's session.
    fn on_apdu(&mut self, apdu: &[u8]) -> ResourceOut;
    /// Logical time advanced (for resources with timers, e.g. date_time).
    fn tick(&mut self, _elapsed: Duration) -> ResourceOut {
        ResourceOut::default()
    }
}

/// Resource Manager (§8.4.1) — host-provided. Drives the profile exchange and,
/// once complete, reports [`Notification::CamReady`] and asks the host to open
/// the module-provided resources it understands.
#[derive(Debug)]
pub struct ResourceManager {
    host_resources: Vec<ResourceId>,
    module_resources: Vec<ResourceId>,
    host_profiled: bool,
    module_profiled: bool,
    ready: bool,
}

impl ResourceManager {
    /// New RM advertising `host_resources` in its profile reply.
    #[must_use]
    pub fn new(host_resources: Vec<ResourceId>) -> Self {
        Self {
            host_resources,
            module_resources: Vec::new(),
            host_profiled: false,
            module_profiled: false,
            ready: false,
        }
    }

    /// Resources the module advertised (valid once the profile exchange ran).
    #[must_use]
    pub fn module_resources(&self) -> &[ResourceId] {
        &self.module_resources
    }
}

impl Resource for ResourceManager {
    fn id(&self) -> ResourceId {
        RESOURCE_MANAGER
    }

    fn on_open(&mut self) -> ResourceOut {
        // Kick off the handshake: ask the module for its profile.
        ResourceOut {
            apdus: vec![ser(&ProfileEnq)],
            ..ResourceOut::default()
        }
    }

    fn on_apdu(&mut self, apdu: &[u8]) -> ResourceOut {
        let mut out = ResourceOut::default();
        match peek_tag(apdu) {
            // Module asks for the host's profile → reply with our resource list.
            Some(t) if t == tag::PROFILE_ENQ => {
                out.apdus.push(ser(&Profile {
                    resources: self.host_resources.clone(),
                }));
                self.host_profiled = true;
            }
            // Module's profile → record its resources.
            Some(t) if t == tag::PROFILE => {
                if let Ok(p) = Profile::parse(apdu) {
                    self.module_resources = p.resources;
                    self.module_profiled = true;
                }
            }
            // Resource set changed → re-enquire.
            Some(t) if t == tag::PROFILE_CHANGE => {
                out.apdus.push(ser(&ProfileEnq));
                self.module_profiled = false;
                self.ready = false;
            }
            _ => {}
        }
        if self.module_profiled && self.host_profiled && !self.ready {
            self.ready = true;
            out.notify.push(Notification::CamReady);
            // Open the module-provided resources we understand.
            for r in [APPLICATION_INFORMATION, CONDITIONAL_ACCESS_SUPPORT, MMI] {
                if self.module_resources.contains(&r) {
                    out.open.push(r);
                }
            }
        }
        out
    }
}

/// Application Information (§8.4.2) — module-provided. On open, enquires the
/// module's application info; surfaces it as [`Notification::ApplicationInfo`].
#[derive(Debug, Default)]
pub struct ApplicationInformation;

impl Resource for ApplicationInformation {
    fn id(&self) -> ResourceId {
        APPLICATION_INFORMATION
    }

    fn on_open(&mut self) -> ResourceOut {
        ResourceOut {
            apdus: vec![ser(&ApplicationInfoEnq)],
            ..ResourceOut::default()
        }
    }

    fn on_apdu(&mut self, apdu: &[u8]) -> ResourceOut {
        let mut out = ResourceOut::default();
        if peek_tag(apdu) == Some(tag::APPLICATION_INFO) {
            if let Ok(ai) = ApplicationInfo::parse(apdu) {
                out.notify.push(Notification::ApplicationInfo {
                    application_type: ai.application_type.to_u8(),
                    manufacturer: ai.application_manufacturer,
                    code: ai.manufacturer_code,
                    menu: String::from_utf8_lossy(ai.menu_string).into_owned(),
                });
            }
        }
        out
    }
}

/// Conditional Access Support (§8.4.3) — module-provided. On open, enquires the
/// module's supported `CA_system_id`s ([`Notification::CaInfo`]); decodes
/// `ca_pmt_reply` ([`Notification::CaPmtReply`]). The host sends `ca_pmt` via
/// [`HostRequest::SendCaPmt`](crate::event::HostRequest::SendCaPmt).
#[derive(Debug, Default)]
pub struct ConditionalAccess;

impl Resource for ConditionalAccess {
    fn id(&self) -> ResourceId {
        CONDITIONAL_ACCESS_SUPPORT
    }

    fn on_open(&mut self) -> ResourceOut {
        ResourceOut {
            apdus: vec![ser(&CaInfoEnq)],
            ..ResourceOut::default()
        }
    }

    fn on_apdu(&mut self, apdu: &[u8]) -> ResourceOut {
        let mut out = ResourceOut::default();
        match peek_tag(apdu) {
            Some(t) if t == tag::CA_INFO => {
                if let Ok(ci) = CaInfo::parse(apdu) {
                    out.notify.push(Notification::CaInfo {
                        ca_system_ids: ci.ca_system_ids,
                    });
                }
            }
            Some(t) if t == tag::CA_PMT_REPLY => {
                if let Ok(r) = CaPmtReply::parse(apdu) {
                    let descrambling_ok = r.ca_enable.is_some_and(|e| {
                        matches!(
                            e,
                            CaEnable::Possible
                                | CaEnable::PossiblePurchaseDialogue
                                | CaEnable::PossibleTechnicalDialogue
                        )
                    });
                    out.notify.push(Notification::CaPmtReply {
                        program_number: r.program_number,
                        descrambling_ok,
                    });
                }
            }
            _ => {}
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvb_ci::objects::resource_manager::Profile;

    #[test]
    fn on_open_sends_profile_enq() {
        let mut rm = ResourceManager::new(vec![RESOURCE_MANAGER]);
        let out = rm.on_open();
        assert_eq!(out.apdus, vec![ser(&ProfileEnq)]);
    }

    #[test]
    fn handshake_completes_and_opens_module_resources() {
        let mut rm = ResourceManager::new(vec![RESOURCE_MANAGER]);
        rm.on_open();
        // module sends its profile (it provides app_info + ca)
        let module_profile = ser(&Profile {
            resources: vec![APPLICATION_INFORMATION, CONDITIONAL_ACCESS_SUPPORT],
        });
        let o1 = rm.on_apdu(&module_profile);
        assert!(
            o1.notify.is_empty(),
            "not ready until host profile sent too"
        );
        // module enquires our profile → we reply + handshake completes
        let o2 = rm.on_apdu(&ser(&ProfileEnq));
        // replied with a profile
        assert_eq!(o2.apdus.len(), 1);
        assert_eq!(peek_tag(&o2.apdus[0]), Some(tag::PROFILE));
        // CamReady + open the module resources we understand
        assert!(o2.notify.contains(&Notification::CamReady));
        assert!(o2.open.contains(&APPLICATION_INFORMATION));
        assert!(o2.open.contains(&CONDITIONAL_ACCESS_SUPPORT));
        assert!(!o2.open.contains(&MMI), "module didn't advertise MMI");
    }

    #[test]
    fn profile_change_re_enquires() {
        let mut rm = ResourceManager::new(vec![RESOURCE_MANAGER]);
        let out = rm.on_apdu(&ser(&dvb_ci::objects::resource_manager::ProfileChange));
        assert_eq!(out.apdus, vec![ser(&ProfileEnq)]);
    }

    #[test]
    fn application_information_surfaces_notification() {
        use dvb_ci::objects::application_info::ApplicationType;
        let mut h = ApplicationInformation;
        assert_eq!(h.on_open().apdus, vec![ser(&ApplicationInfoEnq)]);
        let ai = ser(&ApplicationInfo {
            application_type: ApplicationType::ConditionalAccess,
            application_manufacturer: 0x1234,
            manufacturer_code: 0x5678,
            menu_string: b"Acme CAM",
        });
        let out = h.on_apdu(&ai);
        assert_eq!(
            out.notify,
            vec![Notification::ApplicationInfo {
                application_type: 0x01,
                manufacturer: 0x1234,
                code: 0x5678,
                menu: "Acme CAM".to_string(),
            }]
        );
    }

    #[test]
    fn conditional_access_surfaces_ca_info_and_pmt_reply() {
        let mut h = ConditionalAccess;
        assert_eq!(h.on_open().apdus, vec![ser(&CaInfoEnq)]);
        // ca_info -> CaInfo notification
        let ci = ser(&CaInfo {
            ca_system_ids: vec![0x0B00, 0x1800],
        });
        assert_eq!(
            h.on_apdu(&ci).notify,
            vec![Notification::CaInfo {
                ca_system_ids: vec![0x0B00, 0x1800],
            }]
        );
        // ca_pmt_reply (descrambling possible) -> CaPmtReply notification
        let reply = ser(&CaPmtReply {
            program_number: 0x0042,
            version_number: 0,
            current_next_indicator: true,
            ca_enable: Some(CaEnable::Possible),
            streams: vec![],
        });
        assert_eq!(
            h.on_apdu(&reply).notify,
            vec![Notification::CaPmtReply {
                program_number: 0x0042,
                descrambling_ok: true,
            }]
        );
    }
}
