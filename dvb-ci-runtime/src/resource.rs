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
use dvb_ci::objects::date_time::{DateTime as CiDateTime, DateTimeEnq, UTC_TIME_LEN};
use dvb_ci::objects::mmi_high::{Enq, Menu};
use dvb_ci::objects::resource_manager::{Profile, ProfileChange, ProfileEnq};
use dvb_ci::resource::{
    ResourceId, APPLICATION_INFORMATION, CONDITIONAL_ACCESS_SUPPORT, DATE_TIME, MMI,
    RESOURCE_MANAGER,
};
use dvb_ci::tag::{self, ApduTag};
use dvb_common::{Parse, Serialize};

use crate::event::{MmiEvent, Notification};

/// Decode MMI `text_char` bytes to a `String` (lossy; full EN 300 468 Annex A
/// decoding is the application's concern).
fn text(chars: &[u8]) -> String {
    String::from_utf8_lossy(chars).into_owned()
}

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
        // Once we have the module's profile, the host builds its resource list
        // and sends `profile_change` (§8.4.1.1). That is the gate the module is
        // waiting on: until it arrives the module can neither open nor accept
        // sessions, so it idles after its `profile` reply (#340). After it, the
        // module opens its own sessions (application_information, conditional_
        // access, mmi) — the host does NOT `create_session` for them (§7.2.3;
        // create_session is inter-module routing only).
        if self.module_profiled && !self.ready {
            self.ready = true;
            out.apdus.push(ser(&ProfileChange));
            out.notify.push(Notification::CamReady);
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

const SECS_PER_DAY: u64 = 86_400;
/// Modified Julian Date of the Unix epoch (1970-01-01).
const MJD_UNIX_EPOCH: u64 = 40_587;

fn bcd(v: u64) -> u8 {
    (((v / 10) << 4) | (v % 10)) as u8
}

/// Encode a Unix timestamp as the 5-byte DVB `UTC_time` (MJD `[15:0]` + BCD
/// HH:MM:SS), per EN 300 468 Annex C.
fn unix_to_mjd_bcd(unix_secs: u64) -> [u8; UTC_TIME_LEN] {
    let mjd = (MJD_UNIX_EPOCH + unix_secs / SECS_PER_DAY) as u16;
    let sod = unix_secs % SECS_PER_DAY;
    [
        (mjd >> 8) as u8,
        mjd as u8,
        bcd(sod / 3600),
        bcd((sod % 3600) / 60),
        bcd(sod % 60),
    ]
}

fn system_utc() -> [u8; UTC_TIME_LEN] {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    unix_to_mjd_bcd(secs)
}

/// Date-Time (§8.5.2) — host-provided. On `date_time_enq` replies with the
/// current UTC; if the enquiry's `response_interval` is non-zero, re-sends every
/// `response_interval` seconds (driven by [`tick`](Resource::tick)).
pub struct DateTime {
    clock: fn() -> [u8; UTC_TIME_LEN],
    interval: u8,
    since: Duration,
}

impl Default for DateTime {
    fn default() -> Self {
        Self::new()
    }
}

impl DateTime {
    /// New handler using the system clock.
    #[must_use]
    pub fn new() -> Self {
        Self {
            clock: system_utc,
            interval: 0,
            since: Duration::ZERO,
        }
    }

    /// New handler with an injected clock (for tests / a host-supplied source).
    #[must_use]
    pub fn with_clock(clock: fn() -> [u8; UTC_TIME_LEN]) -> Self {
        Self {
            clock,
            interval: 0,
            since: Duration::ZERO,
        }
    }

    fn reply(&self) -> Vec<u8> {
        ser(&CiDateTime {
            utc_time: (self.clock)(),
            local_offset: None,
        })
    }
}

impl Resource for DateTime {
    fn id(&self) -> ResourceId {
        DATE_TIME
    }

    fn on_apdu(&mut self, apdu: &[u8]) -> ResourceOut {
        let mut out = ResourceOut::default();
        if peek_tag(apdu) == Some(tag::DATE_TIME_ENQ) {
            if let Ok(enq) = DateTimeEnq::parse(apdu) {
                self.interval = enq.response_interval;
                self.since = Duration::ZERO;
                out.apdus.push(self.reply());
            }
        }
        out
    }

    fn tick(&mut self, elapsed: Duration) -> ResourceOut {
        let mut out = ResourceOut::default();
        if self.interval > 0 {
            self.since += elapsed;
            if self.since >= Duration::from_secs(u64::from(self.interval)) {
                self.since = Duration::ZERO;
                out.apdus.push(self.reply());
            }
        }
        out
    }
}

/// MMI (§8.6) — module-provided. Surfaces the module's menus/enquiries and the
/// close as [`Notification::Mmi`] events for the application to display. (The
/// module drives the dialog; answering — `menu_answ`/`answ` — is a later
/// addition.)
#[derive(Debug, Default)]
pub struct Mmi;

impl Resource for Mmi {
    fn id(&self) -> ResourceId {
        MMI
    }

    fn on_apdu(&mut self, apdu: &[u8]) -> ResourceOut {
        let mut out = ResourceOut::default();
        match peek_tag(apdu) {
            Some(t) if t == tag::ENQ => {
                if let Ok(e) = Enq::parse(apdu) {
                    out.notify.push(Notification::Mmi(MmiEvent::Enquiry {
                        prompt: text(e.text_chars),
                        blind: e.blind_answer,
                        answer_len: e.answer_text_length,
                    }));
                }
            }
            Some(t) if t == tag::MENU_LAST => {
                if let Ok(m) = Menu::parse(apdu) {
                    out.notify.push(Notification::Mmi(MmiEvent::Menu {
                        title: text(m.title.text_chars),
                        items: m.choices.iter().map(|c| text(c.text_chars)).collect(),
                    }));
                }
            }
            Some(t) if t == tag::CLOSE_MMI => {
                out.notify.push(Notification::Mmi(MmiEvent::Close));
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
    fn module_profile_triggers_profile_change_and_camready() {
        // #340: after the module's `profile`, the host fires CamReady and sends
        // `profile_change` — the gate that unblocks the module to open its own
        // resource sessions (§8.4.1.1). The host does NOT open them itself.
        let mut rm = ResourceManager::new(vec![RESOURCE_MANAGER]);
        rm.on_open();
        let module_profile = ser(&Profile {
            resources: vec![APPLICATION_INFORMATION, CONDITIONAL_ACCESS_SUPPORT],
        });
        let o = rm.on_apdu(&module_profile);
        assert!(o.notify.contains(&Notification::CamReady));
        assert_eq!(o.apdus.len(), 1, "host sends profile_change");
        assert_eq!(peek_tag(&o.apdus[0]), Some(tag::PROFILE_CHANGE));
        assert!(
            o.open.is_empty(),
            "the module opens its own sessions, not the host"
        );
    }

    #[test]
    fn answers_a_module_profile_enquiry_without_re_readying() {
        let mut rm = ResourceManager::new(vec![RESOURCE_MANAGER]);
        rm.on_open();
        rm.on_apdu(&ser(&Profile {
            resources: vec![APPLICATION_INFORMATION],
        }));
        // A later module profile_enq → reply with our profile, no second CamReady.
        let o = rm.on_apdu(&ser(&ProfileEnq));
        assert_eq!(o.apdus.len(), 1);
        assert_eq!(peek_tag(&o.apdus[0]), Some(tag::PROFILE));
        assert!(!o.notify.contains(&Notification::CamReady));
    }

    #[test]
    fn mmi_surfaces_enquiry_and_close() {
        let mut h = Mmi;
        // enquiry
        let enq = ser(&Enq {
            blind_answer: true,
            answer_text_length: 4,
            text_chars: b"PIN?",
        });
        assert_eq!(
            h.on_apdu(&enq).notify,
            vec![Notification::Mmi(MmiEvent::Enquiry {
                prompt: "PIN?".to_string(),
                blind: true,
                answer_len: 4,
            })]
        );
        // close_mmi (tag 9F 88 00) — surfaced as Close
        let close = [0x9F, 0x88, 0x00, 0x01, 0x00];
        assert_eq!(
            h.on_apdu(&close).notify,
            vec![Notification::Mmi(MmiEvent::Close)]
        );
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
    fn mjd_bcd_encoding_is_correct() {
        // Unix epoch 1970-01-01 00:00:00 → MJD 40587 (0x9E8B), 00:00:00.
        assert_eq!(unix_to_mjd_bcd(0), [0x9E, 0x8B, 0x00, 0x00, 0x00]);
        // 1970-01-02 13:45:09 → MJD 40588 (0x9E8C), BCD 13 45 09.
        let secs = SECS_PER_DAY + 13 * 3600 + 45 * 60 + 9;
        assert_eq!(unix_to_mjd_bcd(secs), [0x9E, 0x8C, 0x13, 0x45, 0x09]);
    }

    #[test]
    fn date_time_replies_to_enq_and_resends_on_interval() {
        let fixed = || [0x9E, 0x7B, 0x00, 0x00, 0x00];
        let mut h = DateTime::with_clock(fixed);
        // enquiry with a 5s response interval → immediate reply
        let enq = ser(&DateTimeEnq {
            response_interval: 5,
        });
        let out = h.on_apdu(&enq);
        assert_eq!(out.apdus.len(), 1);
        assert_eq!(peek_tag(&out.apdus[0]), Some(tag::DATE_TIME));
        // before the interval: no resend
        assert!(h.tick(Duration::from_secs(3)).apdus.is_empty());
        // crossing the interval: resend
        assert_eq!(h.tick(Duration::from_secs(3)).apdus.len(), 1);
    }

    #[test]
    fn date_time_interval_zero_does_not_resend() {
        let mut h = DateTime::with_clock(|| [0u8; UTC_TIME_LEN]);
        h.on_apdu(&ser(&DateTimeEnq {
            response_interval: 0,
        }));
        assert!(h.tick(Duration::from_secs(60)).apdus.is_empty());
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
