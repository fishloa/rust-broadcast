//! The resource layer — application-layer state machines (ETSI EN 50221 §8),
//! one per resource, driven by the session layer's APDUs.
//!
//! Each resource implements [`Resource`]: it reacts to its session opening and
//! to incoming APDUs, producing APDUs to send back, host [`Notification`]s, and
//! requests to open further (module-provided) resources. This module ships the
//! mandatory [`ResourceManager`]; application_information / conditional_access /
//! date_time / mmi land as further `Resource` impls.

use std::time::Duration;

use broadcast_common::{Parse, Serialize};
use dvb_ci::objects::application_info::{ApplicationInfo, ApplicationInfoEnq};
use dvb_ci::objects::ca_info::{CaInfo, CaInfoEnq};
use dvb_ci::objects::ca_pmt_reply::{CaEnable, CaPmtReply};
use dvb_ci::objects::date_time::{DateTime as CiDateTime, DateTimeEnq, UTC_TIME_LEN};
use dvb_ci::objects::host_control::{AskRelease, ClearReplace, Replace, Tune};
use dvb_ci::objects::mmi_display::{
    DisplayControl, DisplayControlCmd, DisplayReply, DisplayReplyBody, DisplayReplyId, MmiMode,
};
use dvb_ci::objects::mmi_high::{Enq, List, Menu};
use dvb_ci::objects::resource_manager::{Profile, ProfileChange, ProfileEnq};
use dvb_ci::resource::{
    APPLICATION_INFORMATION, CONDITIONAL_ACCESS_SUPPORT, DATE_TIME, HOST_CONTROL, MMI,
    RESOURCE_MANAGER, ResourceId,
};
use dvb_ci::tag::{self, ApduTag};

use crate::event::{HostControlEvent, MmiEvent, MmiMenu, Notification};

/// Decode MMI `text_char` bytes to a `String` (lossy; full EN 300 468 Annex A
/// decoding is the application's concern).
fn text(chars: &[u8]) -> String {
    String::from_utf8_lossy(chars).into_owned()
}

/// Project a parsed high-level [`Menu`] (also the body of a `list()`) onto the
/// host-facing [`MmiMenu`] — the three header lines and the choice list kept
/// distinct for display.
fn to_menu(m: &Menu<'_>) -> MmiMenu {
    MmiMenu {
        title: text(m.title.text_chars),
        subtitle: text(m.subtitle.text_chars),
        bottom: text(m.bottom.text_chars),
        choices: m.choices.iter().map(|c| text(c.text_chars)).collect(),
    }
}

/// Serialize an APDU/SPDU object to owned bytes — the one helper the resource,
/// session and stack layers share (audit r10-O-12: it was copied three times).
///
/// r10-W-20: `serialize_into` failing here used to be swallowed into a
/// silently-empty `Vec` (and, in a later pass, turned into a panic) — either
/// way the failure never reached the caller as a value it could act on. These
/// layers react to APDUs that ultimately originate from the CAM/card, so a
/// serialize error must propagate as `Err`, not corrupt the wire exchange or
/// crash the driver.
pub(crate) fn ser<S: Serialize<Error = dvb_ci::Error>>(s: &S) -> dvb_ci::Result<Vec<u8>> {
    s.try_to_bytes()
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
    fn on_open(&mut self) -> dvb_ci::Result<ResourceOut> {
        Ok(ResourceOut::default())
    }
    /// An APDU arrived on this resource's session.
    fn on_apdu(&mut self, apdu: &[u8]) -> dvb_ci::Result<ResourceOut>;
    /// Logical time advanced (for resources with timers, e.g. date_time).
    fn tick(&mut self, _elapsed: Duration) -> dvb_ci::Result<ResourceOut> {
        Ok(ResourceOut::default())
    }
    /// Reset this resource's internal state (r10-W-19): called on
    /// [`HostRequest::Init`](crate::event::HostRequest::Init) and
    /// [`HostRequest::Shutdown`](crate::event::HostRequest::Shutdown) so a
    /// re-inserted CAM starts a fresh handshake against clean state rather
    /// than one latched by whatever the previous module left behind (e.g. a
    /// `ready` flag that never re-opens the `profile_change` gate on a
    /// second `profile` exchange). The default no-op is only correct for a
    /// resource that carries no state between calls.
    fn reset(&mut self) {}
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

    fn on_open(&mut self) -> dvb_ci::Result<ResourceOut> {
        // Kick off the handshake: ask the module for its profile.
        Ok(ResourceOut {
            apdus: vec![ser(&ProfileEnq)?],
            ..ResourceOut::default()
        })
    }

    fn on_apdu(&mut self, apdu: &[u8]) -> dvb_ci::Result<ResourceOut> {
        let mut out = ResourceOut::default();
        match peek_tag(apdu) {
            // Module asks for the host's profile → reply with our resource list.
            Some(t) if t == tag::PROFILE_ENQ => {
                out.apdus.push(ser(&Profile {
                    resources: self.host_resources.clone(),
                })?);
            }
            // Module's profile → record its resources.
            Some(t) if t == tag::PROFILE => {
                // A CAM-originated APDU that fails to parse is a protocol error to
                // surface, not to drop silently (audit #1092 interop).
                let p = Profile::parse(apdu)?;
                self.module_resources = p.resources;
                self.module_profiled = true;
            }
            // Resource set changed → re-enquire.
            Some(t) if t == tag::PROFILE_CHANGE => {
                out.apdus.push(ser(&ProfileEnq)?);
                self.module_profiled = false;
                self.ready = false;
            }
            _ => {}
        }
        // Once we have the module's profile, the host sends `profile_change`
        // (§8.4.1.1) — the gate the module waits on; until it arrives the module
        // idles after its `profile` reply (#340 round 1).
        //
        // The host does NOT open application_information / conditional_access /
        // mmi itself. Confirmed on hardware (#340, live AlphaCrypt): the module
        // ignores a host `open_session_request` for them and rejects a
        // `create_session` (`status=0xF0`). Those resources are **host-provided**
        // — the host advertises them in its `profile` reply (see
        // `CiStack::host_provided`), and the *module* opens sessions to them
        // (module → host `open_session_request`), exactly as it does for
        // resource_manager / date_time. The host just accepts. Each session's
        // `on_open` then drives its enquiry (app_info_enq, ca_info_enq).
        if self.module_profiled && !self.ready {
            self.ready = true;
            out.apdus.push(ser(&ProfileChange)?);
            out.notify.push(Notification::CamReady);
        }
        Ok(out)
    }

    /// r10-W-19: without this, `ready`/`module_profiled` stay latched from a
    /// prior handshake, so a second `profile` (e.g. after a hot-plug
    /// re-insert routed through a fresh `Init`) never re-opens the
    /// `profile_change`/`CamReady` gate — the module then idles forever
    /// (`on_apdu`'s doc comment above).
    fn reset(&mut self) {
        self.module_resources.clear();
        self.module_profiled = false;
        self.ready = false;
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

    fn on_open(&mut self) -> dvb_ci::Result<ResourceOut> {
        Ok(ResourceOut {
            apdus: vec![ser(&ApplicationInfoEnq)?],
            ..ResourceOut::default()
        })
    }

    fn on_apdu(&mut self, apdu: &[u8]) -> dvb_ci::Result<ResourceOut> {
        let mut out = ResourceOut::default();
        if peek_tag(apdu) == Some(tag::APPLICATION_INFO) {
            let ai = ApplicationInfo::parse(apdu)?;
            out.notify.push(Notification::ApplicationInfo {
                application_type: ai.application_type.to_u8(),
                manufacturer: ai.application_manufacturer,
                code: ai.manufacturer_code,
                menu: String::from_utf8_lossy(ai.menu_string).into_owned(),
            });
        }
        Ok(out)
    }
    // No state carried between calls — the default `reset()` no-op applies.
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

    fn on_open(&mut self) -> dvb_ci::Result<ResourceOut> {
        Ok(ResourceOut {
            apdus: vec![ser(&CaInfoEnq)?],
            ..ResourceOut::default()
        })
    }

    fn on_apdu(&mut self, apdu: &[u8]) -> dvb_ci::Result<ResourceOut> {
        let mut out = ResourceOut::default();
        match peek_tag(apdu) {
            Some(t) if t == tag::CA_INFO => {
                // A CAM-originated APDU that fails to parse is a protocol error to
                // surface, not to drop silently (audit #1092 interop).
                let ci = CaInfo::parse(apdu)?;
                out.notify.push(Notification::CaInfo {
                    ca_system_ids: ci.ca_system_ids,
                });
            }
            Some(t) if t == tag::CA_PMT_REPLY => {
                // A CAM-originated APDU that fails to parse is a protocol error to
                // surface, not to drop silently (audit #1092 interop).
                let r = CaPmtReply::parse(apdu)?;
                // EN 50221 §8.4.3.5 Table 26: programme-level `CA_enable`.
                // Plumb the object's own `Option<CaEnable>` straight
                // through — `None` means the programme
                // `CA_enable_flag` bit was clear (no programme-level
                // status given), which is distinct from a genuine
                // flag-set reserved code and must not be collapsed to a
                // sentinel.
                let descrambling_ok = matches!(
                    r.ca_enable,
                    Some(
                        CaEnable::Possible
                            | CaEnable::PossiblePurchaseDialogue
                            | CaEnable::PossibleTechnicalDialogue
                    )
                );
                out.notify.push(Notification::CaPmtReply {
                    program_number: r.program_number,
                    ca_enable: r.ca_enable,
                    descrambling_ok,
                });
            }
            _ => {}
        }
        Ok(out)
    }
    // No state carried between calls — the default `reset()` no-op applies.
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

    fn reply(&self) -> dvb_ci::Result<Vec<u8>> {
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

    fn on_apdu(&mut self, apdu: &[u8]) -> dvb_ci::Result<ResourceOut> {
        let mut out = ResourceOut::default();
        if peek_tag(apdu) == Some(tag::DATE_TIME_ENQ) {
            let enq = DateTimeEnq::parse(apdu)?;
            self.interval = enq.response_interval;
            self.since = Duration::ZERO;
            out.apdus.push(self.reply()?);
        }
        Ok(out)
    }

    fn tick(&mut self, elapsed: Duration) -> dvb_ci::Result<ResourceOut> {
        let mut out = ResourceOut::default();
        if self.interval > 0 {
            self.since += elapsed;
            if self.since >= Duration::from_secs(u64::from(self.interval)) {
                self.since = Duration::ZERO;
                out.apdus.push(self.reply()?);
            }
        }
        Ok(out)
    }

    /// r10-W-19: without this, a stale `interval`/`since` from a previous
    /// module survives into the next connection, so the resend cadence
    /// negotiated with the OLD CAM keeps firing (or a leftover partial
    /// `since` fires early) before the new module ever sends its own
    /// `date_time_enq`.
    fn reset(&mut self) {
        self.interval = 0;
        self.since = Duration::ZERO;
    }
}

/// MMI (§8.6) — module-provided. Surfaces the module's menus/enquiries and the
/// close as [`Notification::Mmi`] events for the application to display, and
/// answers the module's `display_control` mode negotiation. The host drives the
/// dialog back through [`Driver::mmi_menu_answer`](crate::Driver::mmi_menu_answer)
/// / [`mmi_enquiry_answer`](crate::Driver::mmi_enquiry_answer) /
/// [`mmi_cancel`](crate::Driver::mmi_cancel) (sent by [`CiStack`](crate::CiStack)
/// on the open MMI session).
#[derive(Debug, Default)]
pub struct Mmi;

impl Resource for Mmi {
    fn id(&self) -> ResourceId {
        MMI
    }

    fn on_apdu(&mut self, apdu: &[u8]) -> dvb_ci::Result<ResourceOut> {
        let mut out = ResourceOut::default();
        match peek_tag(apdu) {
            Some(t) if t == tag::ENQ => {
                // A CAM-originated APDU that fails to parse is a protocol error to
                // surface, not to drop silently (audit #1092 interop).
                let e = Enq::parse(apdu)?;
                out.notify.push(Notification::Mmi(MmiEvent::Enquiry {
                    prompt: text(e.text_chars),
                    blind: e.blind_answer,
                    answer_len: e.answer_text_length,
                }));
            }
            Some(t) if t == tag::MENU_LAST => {
                // A CAM-originated APDU that fails to parse is a protocol error to
                // surface, not to drop silently (audit #1092 interop).
                let m = Menu::parse(apdu)?;
                out.notify
                    .push(Notification::Mmi(MmiEvent::Menu(to_menu(&m))));
            }
            Some(t) if t == tag::LIST_LAST => {
                // A CAM-originated APDU that fails to parse is a protocol error to
                // surface, not to drop silently (audit #1092 interop).
                let l = List::parse(apdu)?;
                out.notify
                    .push(Notification::Mmi(MmiEvent::List(to_menu(&l.0))));
            }
            Some(t) if t == tag::CLOSE_MMI => {
                dvb_ci::objects::mmi_close::CloseMmi::parse(apdu)?;
                out.notify.push(Notification::Mmi(MmiEvent::Close));
            }
            // High-level MMI mode negotiation (§8.6.1): the module opens an MMI
            // session and sends `display_control`. The host MUST answer
            // `display_reply` or the module aborts the MMI — verified live: a
            // real AlphaCrypt opens MMI after `ca_pmt` and, with no reply, closes
            // the session and never descrambles (an Enigma2 box answers it).
            Some(t) if t == tag::DISPLAY_CONTROL => {
                // A CAM-originated APDU that fails to parse is a protocol error to
                // surface, not to drop silently (audit #1092 interop).
                let dc = DisplayControl::parse(apdu)?;
                let reply = match dc.cmd {
                    // Acknowledge the requested MMI mode (echo it back).
                    DisplayControlCmd::SetMmiMode => DisplayReply {
                        reply_id: DisplayReplyId::MmiModeAck,
                        body: DisplayReplyBody::MmiModeAck(
                            dc.mmi_mode.unwrap_or(MmiMode::HighLevel),
                        ),
                    },
                    // We don't implement the character-table / graphics
                    // queries; tell the module so per Table 35.
                    _ => DisplayReply {
                        reply_id: DisplayReplyId::UnknownDisplayControlCmd,
                        body: DisplayReplyBody::None,
                    },
                };
                out.apdus.push(ser(&reply)?);
            }
            _ => {}
        }
        Ok(out)
    }
    // No state carried between calls — the default `reset()` no-op applies.
}

/// Host Control (§8.5.1) — host-provided. The module opens a host_control
/// session and issues `tune` / `replace` / `clear_replace` / `ask_release`
/// objects; this handler decodes each and surfaces it as
/// [`Notification::HostControl`]. The host acts on the request out of band (it
/// retunes / replaces PIDs itself) — the runtime does not re-tune, so there is
/// no reply APDU.
#[derive(Debug, Default)]
pub struct HostControl;

impl Resource for HostControl {
    fn id(&self) -> ResourceId {
        HOST_CONTROL
    }

    fn on_apdu(&mut self, apdu: &[u8]) -> dvb_ci::Result<ResourceOut> {
        let mut out = ResourceOut::default();
        // A CAM-originated host-control APDU that fails to parse (e.g. a padded
        // `tune`) is a protocol error: `?` surfaces it as `Notification::Error`
        // through the stack instead of dropping it silently.
        let event = match peek_tag(apdu) {
            Some(t) if t == tag::TUNE => {
                let t = Tune::parse(apdu)?;
                Some(HostControlEvent::Tune {
                    network_id: t.network_id,
                    original_network_id: t.original_network_id,
                    transport_stream_id: t.transport_stream_id,
                    service_id: t.service_id,
                })
            }
            Some(t) if t == tag::REPLACE => {
                let r = Replace::parse(apdu)?;
                Some(HostControlEvent::Replace {
                    replacement_ref: r.replacement_ref,
                    replaced_pid: r.replaced_pid,
                    replacement_pid: r.replacement_pid,
                })
            }
            Some(t) if t == tag::CLEAR_REPLACE => {
                let c = ClearReplace::parse(apdu)?;
                Some(HostControlEvent::ClearReplace {
                    replacement_ref: c.replacement_ref,
                })
            }
            Some(t) if t == tag::ASK_RELEASE => {
                AskRelease::parse(apdu)?;
                Some(HostControlEvent::AskRelease)
            }
            _ => None,
        };
        if let Some(event) = event {
            out.notify.push(Notification::HostControl(event));
        }
        Ok(out)
    }
    // No state carried between calls — the default `reset()` no-op applies.
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvb_ci::objects::resource_manager::Profile;

    /// Local test helper: unwrap a serialize that must succeed for the
    /// fixed/bounded values these tests construct.
    fn s<S: Serialize<Error = dvb_ci::Error>>(v: &S) -> Vec<u8> {
        ser(v).expect("test value must serialize")
    }

    #[test]
    fn on_open_sends_profile_enq() {
        let mut rm = ResourceManager::new(vec![RESOURCE_MANAGER]);
        let out = rm.on_open().unwrap();
        assert_eq!(out.apdus, vec![s(&ProfileEnq)]);
    }

    #[test]
    fn module_profile_triggers_profile_change_and_camready() {
        // #340: after the module's `profile`, the host fires CamReady and sends
        // `profile_change` (the §8.4.1.1 gate) — and nothing else. It does NOT
        // open application_information / conditional_access / mmi itself: those
        // are host-provided resources the module opens sessions to (verified on
        // a live AlphaCrypt, which rejects/ignores host-initiated opens).
        let mut rm = ResourceManager::new(vec![RESOURCE_MANAGER]);
        rm.on_open().unwrap();
        let empty_profile = s(&Profile { resources: vec![] });
        let o = rm.on_apdu(&empty_profile).unwrap();
        assert!(o.notify.contains(&Notification::CamReady));
        assert_eq!(o.apdus.len(), 1, "host sends profile_change");
        assert_eq!(peek_tag(&o.apdus[0]), Some(tag::PROFILE_CHANGE));
        assert!(o.open.is_empty(), "host opens no sessions itself");
    }

    #[test]
    fn answers_a_module_profile_enquiry_without_re_readying() {
        let mut rm = ResourceManager::new(vec![RESOURCE_MANAGER]);
        rm.on_open().unwrap();
        rm.on_apdu(&s(&Profile {
            resources: vec![APPLICATION_INFORMATION],
        }))
        .unwrap();
        // A later module profile_enq → reply with our profile, no second CamReady.
        let o = rm.on_apdu(&s(&ProfileEnq)).unwrap();
        assert_eq!(o.apdus.len(), 1);
        assert_eq!(peek_tag(&o.apdus[0]), Some(tag::PROFILE));
        assert!(!o.notify.contains(&Notification::CamReady));
    }

    /// r10-W-19: `reset()` must clear every piece of the RM's latched state
    /// (module resources, `module_profiled`, `ready`) so a fresh handshake
    /// after a re-init behaves exactly like a brand-new `ResourceManager` —
    /// in particular so `ready` does not stay latched and silently suppress
    /// the next `CamReady`.
    #[test]
    fn resource_manager_reset_clears_latched_handshake_state() {
        let mut rm = ResourceManager::new(vec![RESOURCE_MANAGER]);
        rm.on_open().unwrap();
        let o = rm
            .on_apdu(&s(&Profile {
                resources: vec![APPLICATION_INFORMATION],
            }))
            .unwrap();
        assert!(o.notify.contains(&Notification::CamReady), "dirtied: ready");
        assert!(
            !rm.module_resources().is_empty(),
            "dirtied: module_resources"
        );

        rm.reset();
        assert!(
            rm.module_resources().is_empty(),
            "reset must clear module_resources"
        );

        // Pre-fix (no reset()), `ready` stayed latched `true` and this second
        // profile exchange would NOT re-fire CamReady.
        rm.on_open().unwrap();
        let o2 = rm.on_apdu(&s(&Profile { resources: vec![] })).unwrap();
        assert!(
            o2.notify.contains(&Notification::CamReady),
            "a fresh handshake after reset() must fire CamReady again"
        );
    }

    #[test]
    fn mmi_surfaces_enquiry_and_close() {
        let mut h = Mmi;
        // enquiry
        let enq = s(&Enq {
            blind_answer: true,
            answer_text_length: 4,
            text_chars: b"PIN?",
        });
        assert_eq!(
            h.on_apdu(&enq).unwrap().notify,
            vec![Notification::Mmi(MmiEvent::Enquiry {
                prompt: "PIN?".to_string(),
                blind: true,
                answer_len: 4,
            })]
        );
        // close_mmi (tag 9F 88 00) — surfaced as Close
        let close = [0x9F, 0x88, 0x00, 0x01, 0x00];
        assert_eq!(
            h.on_apdu(&close).unwrap().notify,
            vec![Notification::Mmi(MmiEvent::Close)]
        );
    }

    #[test]
    fn mmi_surfaces_structured_menu_and_list() {
        use dvb_ci::objects::mmi_high::{List, Menu, Text};
        let txt = |s: &'static [u8]| Text {
            more: false,
            text_chars: s,
        };
        let mut h = Mmi;
        // A `menu()` → MmiEvent::Menu with header lines and choices kept distinct.
        let menu = s(&Menu {
            more: false,
            choice_nb: 2,
            title: txt(b"AlphaCrypt"),
            subtitle: txt(b"Module Mainmenu"),
            bottom: txt(b"Select item and press OK"),
            choices: vec![txt(b"Smartcard"), txt(b"Quit")],
        });
        assert_eq!(
            h.on_apdu(&menu).unwrap().notify,
            vec![Notification::Mmi(MmiEvent::Menu(MmiMenu {
                title: "AlphaCrypt".to_string(),
                subtitle: "Module Mainmenu".to_string(),
                bottom: "Select item and press OK".to_string(),
                choices: vec!["Smartcard".to_string(), "Quit".to_string()],
            }))]
        );
        // A `list()` (same body) → MmiEvent::List.
        let list = s(&List(Menu {
            more: false,
            choice_nb: 0xFF,
            title: txt(b"Entitlements"),
            subtitle: txt(b""),
            bottom: txt(b""),
            choices: vec![txt(b"ORF AUT")],
        }));
        assert_eq!(
            h.on_apdu(&list).unwrap().notify,
            vec![Notification::Mmi(MmiEvent::List(MmiMenu {
                title: "Entitlements".to_string(),
                subtitle: String::new(),
                bottom: String::new(),
                choices: vec!["ORF AUT".to_string()],
            }))]
        );
    }

    #[test]
    fn mmi_answers_display_control_set_mmi_mode() {
        use dvb_ci::objects::mmi_display::{DisplayControl, DisplayControlCmd, MmiMode};
        let mut h = Mmi;
        let dc = s(&DisplayControl {
            cmd: DisplayControlCmd::SetMmiMode,
            mmi_mode: Some(MmiMode::HighLevel),
        });
        let out = h.on_apdu(&dc).unwrap();
        // mmi_mode_ack(high_level): 9F 88 02 02 01 01.
        assert_eq!(out.apdus, vec![vec![0x9F, 0x88, 0x02, 0x02, 0x01, 0x01]]);
        assert!(out.notify.is_empty());
    }

    #[test]
    fn profile_change_re_enquires() {
        let mut rm = ResourceManager::new(vec![RESOURCE_MANAGER]);
        let out = rm
            .on_apdu(&s(&dvb_ci::objects::resource_manager::ProfileChange))
            .unwrap();
        assert_eq!(out.apdus, vec![s(&ProfileEnq)]);
    }

    #[test]
    fn application_information_surfaces_notification() {
        use dvb_ci::objects::application_info::ApplicationType;
        let mut h = ApplicationInformation;
        assert_eq!(h.on_open().unwrap().apdus, vec![s(&ApplicationInfoEnq)]);
        let ai = s(&ApplicationInfo {
            application_type: ApplicationType::ConditionalAccess,
            application_manufacturer: 0x1234,
            manufacturer_code: 0x5678,
            menu_string: b"Acme CAM",
        });
        let out = h.on_apdu(&ai).unwrap();
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
    fn host_control_surfaces_tune_replace_clear_and_ask_release() {
        let mut h = HostControl;
        assert_eq!(h.id(), HOST_CONTROL);

        // tune() → HostControlEvent::Tune with the four 16-bit identifiers.
        let tune = s(&Tune {
            network_id: 0x1122,
            original_network_id: 0x3344,
            transport_stream_id: 0x5566,
            service_id: 0x7788,
        });
        assert_eq!(
            h.on_apdu(&tune).unwrap().notify,
            vec![Notification::HostControl(HostControlEvent::Tune {
                network_id: 0x1122,
                original_network_id: 0x3344,
                transport_stream_id: 0x5566,
                service_id: 0x7788,
            })]
        );

        // replace() → HostControlEvent::Replace with the 13-bit PIDs decoded.
        let replace = s(&Replace {
            replacement_ref: 0x07,
            replaced_pid: 0x0123,
            replacement_pid: 0x01FF,
        });
        assert_eq!(
            h.on_apdu(&replace).unwrap().notify,
            vec![Notification::HostControl(HostControlEvent::Replace {
                replacement_ref: 0x07,
                replaced_pid: 0x0123,
                replacement_pid: 0x01FF,
            })]
        );

        // clear_replace() → HostControlEvent::ClearReplace.
        let clear = s(&ClearReplace {
            replacement_ref: 0x42,
        });
        assert_eq!(
            h.on_apdu(&clear).unwrap().notify,
            vec![Notification::HostControl(HostControlEvent::ClearReplace {
                replacement_ref: 0x42,
            })]
        );

        // ask_release() → HostControlEvent::AskRelease (header-only).
        let ask = s(&AskRelease);
        assert_eq!(
            h.on_apdu(&ask).unwrap().notify,
            vec![Notification::HostControl(HostControlEvent::AskRelease)]
        );

        // The host acts out of band: no reply APDU is produced.
        assert!(h.on_apdu(&tune).unwrap().apdus.is_empty());
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
        let enq = s(&DateTimeEnq {
            response_interval: 5,
        });
        let out = h.on_apdu(&enq).unwrap();
        assert_eq!(out.apdus.len(), 1);
        assert_eq!(peek_tag(&out.apdus[0]), Some(tag::DATE_TIME));
        // before the interval: no resend
        assert!(h.tick(Duration::from_secs(3)).unwrap().apdus.is_empty());
        // crossing the interval: resend
        assert_eq!(h.tick(Duration::from_secs(3)).unwrap().apdus.len(), 1);
    }

    #[test]
    fn date_time_interval_zero_does_not_resend() {
        let mut h = DateTime::with_clock(|| [0u8; UTC_TIME_LEN]);
        h.on_apdu(&s(&DateTimeEnq {
            response_interval: 0,
        }))
        .unwrap();
        assert!(h.tick(Duration::from_secs(60)).unwrap().apdus.is_empty());
    }

    /// r10-W-19: `reset()` must clear the resend timer (`interval`/`since`)
    /// so a stale cadence negotiated with a PREVIOUS module does not keep
    /// firing (or fire early on a leftover partial `since`) before the new
    /// module ever sends its own `date_time_enq`.
    #[test]
    fn date_time_reset_clears_interval_and_resend_timer() {
        let mut h = DateTime::with_clock(|| [0u8; UTC_TIME_LEN]);
        h.on_apdu(&s(&DateTimeEnq {
            response_interval: 5,
        }))
        .unwrap();
        // Dirty: interval = 5, partway through the resend window.
        assert!(h.tick(Duration::from_secs(3)).unwrap().apdus.is_empty());

        h.reset();
        // Pre-fix, the leftover `since` (3s) plus this tick would cross the
        // still-latched 5s interval and resend; post-reset, interval is 0 so
        // nothing fires no matter how much time passes.
        assert!(
            h.tick(Duration::from_secs(60)).unwrap().apdus.is_empty(),
            "reset must clear the resend cadence from the previous module"
        );
    }

    #[test]
    fn conditional_access_surfaces_ca_info_and_pmt_reply() {
        let mut h = ConditionalAccess;
        assert_eq!(h.on_open().unwrap().apdus, vec![s(&CaInfoEnq)]);
        // ca_info -> CaInfo notification
        let ci = s(&CaInfo {
            ca_system_ids: vec![0x0B00, 0x1800],
        });
        assert_eq!(
            h.on_apdu(&ci).unwrap().notify,
            vec![Notification::CaInfo {
                ca_system_ids: vec![0x0B00, 0x1800],
            }]
        );
        // ca_pmt_reply (descrambling possible) -> CaPmtReply notification
        let reply = s(&CaPmtReply {
            program_number: 0x0042,
            version_number: 0,
            current_next_indicator: true,
            ca_enable: Some(CaEnable::Possible),
            streams: vec![],
        });
        assert_eq!(
            h.on_apdu(&reply).unwrap().notify,
            vec![Notification::CaPmtReply {
                program_number: 0x0042,
                ca_enable: Some(CaEnable::Possible),
                descrambling_ok: true,
            }]
        );
    }

    #[test]
    fn conditional_access_ca_pmt_reply_flag_clear_surfaces_none() {
        let mut h = ConditionalAccess;
        h.on_open().unwrap();
        // Programme `CA_enable_flag` clear -> no programme-level status given.
        let reply = s(&CaPmtReply {
            program_number: 0x0007,
            version_number: 0,
            current_next_indicator: true,
            ca_enable: None,
            streams: vec![],
        });
        assert_eq!(
            h.on_apdu(&reply).unwrap().notify,
            vec![Notification::CaPmtReply {
                program_number: 0x0007,
                ca_enable: None,
                descrambling_ok: false,
            }]
        );
    }
}
