//! The CI protocol stack — composes the transport + session layers (and, as
//! they land, the resource state machines) into one sans-IO core.
//!
//! [`CiStack::handle`] is the pure entry point: feed it an [`Event`], get back
//! the [`Action`]s the driver must perform. No I/O, threads, or clock here.

use crate::event::{Action, Event, HostRequest, Notification};
use crate::resource::{
    ApplicationInformation, ConditionalAccess, DateTime, Mmi, Resource, ResourceManager,
    ResourceOut,
};
use crate::session::{SessionLayer, SessionOut};
use crate::transport::{Out as TransportOut, Transport};

use dvb_ci::builder::{build_ca_pmt, build_ca_pmt_for_caids};
use dvb_ci::objects::ca_pmt::{CaPmtCmdId, CaPmtListManagement};
use dvb_ci::objects::mmi_high::{Answ, AnswId, MenuAnsw};
use dvb_ci::resource::{ResourceId, CONDITIONAL_ACCESS_SUPPORT, DATE_TIME, MMI, RESOURCE_MANAGER};
use dvb_common::{Parse, Serialize};
use dvb_si::tables::pmt::PmtSection;

/// Serialize an APDU object to owned bytes (buffer is sized exactly).
fn ser_apdu<S: Serialize>(s: &S) -> Vec<u8> {
    let mut b = vec![0u8; s.serialized_len()];
    match s.serialize_into(&mut b) {
        Ok(n) => b.truncate(n),
        Err(_) => b.clear(),
    }
    b
}

/// The composed EN 50221 protocol core.
pub struct CiStack {
    transport: Transport,
    session: SessionLayer,
    /// Resources the host provides (answers incoming `open_session_request`).
    provided: Vec<ResourceId>,
    /// Application-layer resource handlers, dispatched by `ResourceId`.
    resources: Vec<Box<dyn Resource>>,
    /// `CA_system_id`s the CAM advertised in its `ca_info` (the descramble
    /// filter set; empty until `ca_info` arrives).
    cam_caids: Vec<u16>,
    /// PMT section bytes of an in-flight [`HostRequest::Descramble`], awaiting
    /// the `ca_pmt_reply` that triggers the `ok_descrambling` follow-up.
    pending_descramble: Option<Vec<u8>>,
}

impl Default for CiStack {
    fn default() -> Self {
        Self::new()
    }
}

impl CiStack {
    /// New stack on transport connection `t_c_id = 1`. The host advertises the
    /// Resource Manager and registers the RM + application_information +
    /// conditional_access handlers.
    #[must_use]
    pub fn new() -> Self {
        // The host provides Resource Manager + Date-Time; it opens the
        // module-provided application_information + conditional_access.
        let provided = vec![RESOURCE_MANAGER, DATE_TIME];
        Self {
            transport: Transport::new(1),
            session: SessionLayer::new(),
            resources: vec![
                Box::new(ResourceManager::new(provided.clone())),
                Box::new(ApplicationInformation),
                Box::new(ConditionalAccess),
                Box::new(DateTime::new()),
                Box::new(Mmi),
            ],
            provided,
            cam_caids: Vec::new(),
            pending_descramble: None,
        }
    }

    /// Register an additional resource handler.
    pub fn register(&mut self, resource: Box<dyn Resource>) -> &mut Self {
        self.resources.push(resource);
        self
    }

    /// Index of the registered handler for `resource`, if any.
    fn handler_index(&self, resource: ResourceId) -> Option<usize> {
        self.resources.iter().position(|r| r.id() == resource)
    }

    /// The pure sans-IO entry point.
    pub fn handle(&mut self, event: Event<'_>) -> Vec<Action> {
        match event {
            Event::Host(HostRequest::Init) => {
                let mut actions = vec![Action::Reset, Action::QuerySlot];
                let out = self.transport.init();
                actions.extend(self.emit_transport(out));
                actions
            }
            Event::Tick { elapsed } => {
                let out = self.transport.tick(elapsed);
                let mut actions = self.emit_transport(out);
                // Advance each open resource's timers (e.g. date_time resend).
                for (session_nb, resource) in self.session.sessions() {
                    if let Some(i) = self.handler_index(resource) {
                        let out = self.resources[i].tick(elapsed);
                        actions.extend(self.process_resource_out(session_nb, out));
                    }
                }
                actions
            }
            Event::Readable(frame) => {
                let out = self.transport.on_frame(frame);
                self.emit_transport(out)
            }
            Event::Host(HostRequest::SendCaPmt(apdu)) => {
                self.send_to_resource(CONDITIONAL_ACCESS_SUPPORT, apdu)
            }
            Event::Host(HostRequest::Descramble(pmt)) => self.descramble(pmt),
            Event::Host(HostRequest::MmiMenuAnswer(choice_ref)) => {
                let apdu = ser_apdu(&MenuAnsw { choice_ref });
                self.send_to_resource(MMI, &apdu)
            }
            Event::Host(HostRequest::MmiEnquiryAnswer(text)) => {
                let apdu = ser_apdu(&Answ {
                    answ_id: AnswId::Answer,
                    text_chars: text,
                });
                self.send_to_resource(MMI, &apdu)
            }
            Event::Host(HostRequest::MmiCancel) => {
                let apdu = ser_apdu(&Answ {
                    answ_id: AnswId::Cancel,
                    text_chars: &[],
                });
                self.send_to_resource(MMI, &apdu)
            }
            Event::Host(HostRequest::Shutdown) => Vec::new(),
        }
    }

    /// React to a CA notification as it is surfaced: cache the CAM's CAIDs from
    /// `ca_info`, and complete a pending [`HostRequest::Descramble`] by sending
    /// `ok_descrambling` when the `ca_pmt_reply` says descrambling is possible.
    fn on_ca_notification(&mut self, note: &Notification) -> Vec<Action> {
        match note {
            Notification::CaInfo { ca_system_ids } => {
                self.cam_caids = ca_system_ids.clone();
                Vec::new()
            }
            Notification::CaPmtReply {
                descrambling_ok, ..
            } => match self.pending_descramble.take() {
                Some(pmt) if *descrambling_ok => {
                    match self.build_ca_pmt_bytes(&pmt, CaPmtCmdId::OkDescrambling) {
                        Ok(bytes) => self.send_to_resource(CONDITIONAL_ACCESS_SUPPORT, &bytes),
                        Err(detail) => vec![Action::Notify(Notification::Error { detail })],
                    }
                }
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// Begin a [`HostRequest::Descramble`]: build a CAID-filtered `ca_pmt` with
    /// `cmd_id = query` and send it, recording the PMT so the `ok_descrambling`
    /// follow-up can be built when the reply arrives.
    fn descramble(&mut self, pmt: &[u8]) -> Vec<Action> {
        let bytes = match self.build_ca_pmt_bytes(pmt, CaPmtCmdId::Query) {
            Ok(b) => b,
            Err(detail) => return vec![Action::Notify(Notification::Error { detail })],
        };
        self.pending_descramble = Some(pmt.to_vec());
        self.send_to_resource(CONDITIONAL_ACCESS_SUPPORT, &bytes)
    }

    /// Build a CAID-filtered `ca_pmt` APDU (`list_management = only`) for `pmt`
    /// with the given command id. Filters to the CAM's advertised CAIDs once
    /// `ca_info` is known; falls back to all `CA_descriptor`s before then.
    fn build_ca_pmt_bytes(&self, pmt: &[u8], cmd_id: CaPmtCmdId) -> Result<Vec<u8>, String> {
        let parsed = PmtSection::parse(pmt).map_err(|e| format!("invalid PMT: {e}"))?;
        let lm = CaPmtListManagement::Only;
        let built = if self.cam_caids.is_empty() {
            build_ca_pmt(&parsed, lm, cmd_id)
        } else {
            build_ca_pmt_for_caids(&parsed, &self.cam_caids, lm, cmd_id)
        };
        Ok(built.to_bytes())
    }

    /// Send an APDU to the open session bound to `resource` (if any).
    fn send_to_resource(&mut self, resource: ResourceId, apdu: &[u8]) -> Vec<Action> {
        // Find the session_nb for the resource (linear scan over the small set).
        let nb = (1u16..=u16::MAX).find(|&n| self.session.resource_of(n) == Some(resource));
        match nb {
            Some(nb) => {
                let spdu = self.session.send_apdu(nb, apdu);
                let out = self.transport.send_spdu(&spdu);
                self.emit_transport(out)
            }
            None => vec![Action::Notify(Notification::Error {
                detail: format!("no open session for resource {}", resource.name()),
            })],
        }
    }

    /// Convert a transport [`Out`](TransportOut) into actions, driving any
    /// reassembled SPDUs up through the session layer.
    fn emit_transport(&mut self, out: TransportOut) -> Vec<Action> {
        let mut actions = Vec::new();
        for w in out.writes {
            actions.push(Action::Write(w));
        }
        if let Some(after) = out.timer {
            actions.push(Action::SetTimer { after });
        }
        if let Some(err) = out.error {
            actions.push(Action::Notify(Notification::Error {
                detail: err.to_string(),
            }));
        }
        for spdu in out.spdus {
            actions.extend(self.drive_session(&spdu));
        }
        actions
    }

    /// Feed one SPDU to the session layer and convert its output to actions.
    fn drive_session(&mut self, spdu: &[u8]) -> Vec<Action> {
        let provided = self.provided.clone();
        let SessionOut {
            spdus,
            apdus,
            opened,
            closed,
        } = self.session.on_spdu(spdu, |r| provided.contains(&r));

        let mut actions = Vec::new();
        // Session-layer SPDUs (e.g. open_session_response) go down the transport.
        for s in spdus {
            actions.extend(self.send_spdu_actions(&s));
        }
        for (session_nb, resource) in opened {
            actions.push(Action::Notify(Notification::SessionOpened { resource }));
            // Drive the resource handler's on_open (e.g. RM sends profile_enq).
            if let Some(i) = self.handler_index(resource) {
                let out = self.resources[i].on_open();
                actions.extend(self.process_resource_out(session_nb, out));
            }
        }
        for session_nb in closed {
            actions.push(Action::Notify(Notification::SessionClosed { session_nb }));
        }
        // Route each APDU to the resource handler bound to its session.
        for (session_nb, apdu) in apdus {
            if let Some(resource) = self.session.resource_of(session_nb) {
                if let Some(i) = self.handler_index(resource) {
                    let out = self.resources[i].on_apdu(&apdu);
                    actions.extend(self.process_resource_out(session_nb, out));
                }
            }
        }
        actions
    }

    /// Wrap an SPDU as a `T_Data_Last` and collect the resulting actions.
    fn send_spdu_actions(&mut self, spdu: &[u8]) -> Vec<Action> {
        let t = self.transport.send_spdu(spdu);
        let mut actions = Vec::new();
        for w in t.writes {
            actions.push(Action::Write(w));
        }
        if let Some(after) = t.timer {
            actions.push(Action::SetTimer { after });
        }
        actions
    }

    /// Convert a [`ResourceOut`] into actions: send its APDUs on `session_nb`,
    /// surface its notifications, and open any module resources it requested.
    fn process_resource_out(&mut self, session_nb: u16, out: ResourceOut) -> Vec<Action> {
        let mut actions = Vec::new();
        for apdu in out.apdus {
            let spdu = self.session.send_apdu(session_nb, &apdu);
            actions.extend(self.send_spdu_actions(&spdu));
        }
        for note in out.notify {
            // Drive the auto-descramble sequence off the CA notifications.
            let follow = self.on_ca_notification(&note);
            actions.push(Action::Notify(note));
            actions.extend(follow);
        }
        for resource in out.open {
            let spdu = self.session.create_session(resource);
            actions.extend(self.send_spdu_actions(&spdu));
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::DEFAULT_POLL_INTERVAL;
    use dvb_ci::resource::RESOURCE_MANAGER;
    use dvb_ci::spdu::{tags as spdu_tags, OpenSessionRequest};
    use dvb_ci::tpdu::{tags as tpdu_tags, SbValue};
    use dvb_common::Serialize;

    fn ser<S: Serialize>(s: &S) -> Vec<u8> {
        let mut b = vec![0u8; s.serialized_len()];
        match s.serialize_into(&mut b) {
            Ok(n) => b.truncate(n),
            Err(_) => b.clear(),
        }
        b
    }

    /// Wrap an SPDU as a module→host `T_Data_Last` R_TPDU (+ T_SB, DA clear).
    fn r_data(tcid: u8, spdu: &[u8]) -> Vec<u8> {
        let mut v = vec![tpdu_tags::DATA_LAST, (1 + spdu.len()) as u8, tcid];
        v.extend_from_slice(spdu);
        v.extend_from_slice(&[tpdu_tags::SB, 0x02, tcid, SbValue::new(false).0]);
        v
    }

    #[test]
    fn init_resets_and_opens_transport() {
        let mut s = CiStack::new();
        let a = s.handle(Event::Host(HostRequest::Init));
        assert_eq!(a[0], Action::Reset);
        assert_eq!(a[1], Action::QuerySlot);
        assert!(matches!(&a[2], Action::Write(w) if w[0] == tpdu_tags::CREATE_T_C));
    }

    #[test]
    fn full_pipeline_opens_a_session_for_a_provided_resource() {
        let mut s = CiStack::new();
        s.handle(Event::Host(HostRequest::Init));
        // module accepts the transport connection
        s.handle(Event::Readable(&[tpdu_tags::C_T_C_REPLY, 0x01, 0x01]));
        // module opens a session to the host's resource_manager (carried in an
        // R_TPDU data block)
        let osr = ser(&OpenSessionRequest {
            resource: RESOURCE_MANAGER,
        });
        let actions = s.handle(Event::Readable(&r_data(1, &osr)));

        // a SessionOpened notification surfaced...
        assert!(actions.iter().any(|x| matches!(
            x,
            Action::Notify(Notification::SessionOpened {
                resource
            }) if *resource == RESOURCE_MANAGER
        )));
        // ...and an open_session_response was written back down (inside a TPDU).
        let wrote_osr = actions.iter().any(|x| match x {
            Action::Write(w) => w
                .windows(1)
                .any(|_| w.contains(&spdu_tags::OPEN_SESSION_RESPONSE)),
            _ => false,
        });
        assert!(wrote_osr, "open_session_response must be sent down");

        // and the session is tracked + a valid response decodes
        let nb = (1u16..16).find(|&n| s.session.resource_of(n).is_some());
        assert!(nb.is_some());
    }

    #[test]
    fn tick_drives_poll_when_active() {
        let mut s = CiStack::new();
        s.handle(Event::Host(HostRequest::Init));
        s.handle(Event::Readable(&[tpdu_tags::C_T_C_REPLY, 0x01, 0x01]));
        let a = s.handle(Event::Tick {
            elapsed: DEFAULT_POLL_INTERVAL,
        });
        assert!(a
            .iter()
            .any(|x| matches!(x, Action::Write(w) if w.first() == Some(&tpdu_tags::DATA_LAST))));
    }

    // --- #334: the auto-descramble (query -> reply -> ok) sequence ---

    /// Feed standalone `T_SB`s (data_available = 0) — the module acking each host
    /// block — until the stack stops writing, collecting every action. This
    /// drains the transport's one-block-per-turn outbound queue (#337).
    fn pump_sbs(s: &mut CiStack) -> Vec<Action> {
        let mut all = Vec::new();
        for _ in 0..16 {
            let a = s.handle(Event::Readable(&[
                tpdu_tags::SB,
                0x02,
                0x01,
                SbValue::new(false).0,
            ]));
            let wrote = a.iter().any(|x| matches!(x, Action::Write(_)));
            all.extend(a);
            if !wrote {
                break;
            }
        }
        all
    }

    /// Wrap an APDU for delivery on `session_nb` (session_number prefix), then as
    /// a module→host R_TPDU.
    fn r_apdu(session_nb: u16, apdu: &[u8]) -> Vec<u8> {
        use dvb_ci::spdu::SessionNumber;
        let mut spdu = ser(&SessionNumber { session_nb });
        spdu.extend_from_slice(apdu);
        r_data(1, &spdu)
    }

    /// Minimal PMT: program_info has one CA_descriptor (CAID 0x0B00) + a non-CA
    /// descriptor; one clear ES. Mirrors the dvb-ci builder fixture.
    fn build_pmt() -> Vec<u8> {
        let prog_ca = [0x09u8, 0x04, 0x0B, 0x00, 0xE1, 0x00];
        let reg = [0x05u8, 0x04, b'H', b'D', b'M', b'V'];
        let mut program_info = Vec::new();
        program_info.extend_from_slice(&prog_ca);
        program_info.extend_from_slice(&reg);
        let lang = [0x0Au8, 0x04, b'e', b'n', b'g', 0x00];

        let mut body = Vec::new();
        body.push(0x02); // table_id
        body.push(0);
        body.push(0); // section_length placeholder
        body.extend_from_slice(&[0x00, 0x01]); // program_number 1
        body.push(0xC3); // version 1, current_next 1
        body.push(0x00);
        body.push(0x00);
        body.push(0xE0 | 0x02); // PCR_PID 0x0200
        body.push(0x00);
        let pil = program_info.len();
        body.push(0xF0 | ((pil >> 8) as u8 & 0x0F));
        body.push(pil as u8);
        body.extend_from_slice(&program_info);
        // one clear ES
        body.push(0x03);
        body.push(0xE0 | 0x02);
        body.push(0x01);
        body.push(0xF0 | ((lang.len() >> 8) as u8 & 0x0F));
        body.push(lang.len() as u8);
        body.extend_from_slice(&lang);

        let section_length = body.len() - 3 + 4;
        body[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        body[2] = section_length as u8;
        let crc = dvb_common::crc32_mpeg2::compute(&body);
        body.extend_from_slice(&crc.to_be_bytes());
        body
    }

    /// Drive the full handshake to an open conditional-access session with the
    /// CAM's CAIDs learned. Returns the stack with CA on session 2.
    fn stack_with_ca_session() -> CiStack {
        use dvb_ci::objects::ca_info::CaInfo;
        use dvb_ci::objects::resource_manager::{Profile, ProfileEnq};
        use dvb_ci::resource::{APPLICATION_INFORMATION, CONDITIONAL_ACCESS_SUPPORT, MMI};
        use dvb_ci::spdu::{CreateSessionResponse, OpenSessionRequest, SessionStatus};

        let mut s = CiStack::new();
        s.handle(Event::Host(HostRequest::Init));
        s.handle(Event::Readable(&[tpdu_tags::C_T_C_REPLY, 0x01, 0x01]));
        // module opens the host's resource_manager → RM session 1
        s.handle(Event::Readable(&r_data(
            1,
            &ser(&OpenSessionRequest {
                resource: RESOURCE_MANAGER,
            }),
        )));
        // module enquires our profile (we answer) and sends its own profile
        s.handle(Event::Readable(&r_apdu(1, &ser(&ProfileEnq))));
        s.handle(Event::Readable(&r_apdu(
            1,
            &ser(&Profile {
                resources: vec![
                    RESOURCE_MANAGER,
                    APPLICATION_INFORMATION,
                    CONDITIONAL_ACCESS_SUPPORT,
                    MMI,
                ],
            }),
        )));
        // RM is ready → it opened the module resources via create_session.
        // application_information got session 2, conditional_access session 3
        // (alloc order matches the open list). Accept both.
        for (nb, res) in [
            (2u16, APPLICATION_INFORMATION),
            (3, CONDITIONAL_ACCESS_SUPPORT),
            (4, MMI),
        ] {
            s.handle(Event::Readable(&r_data(
                1,
                &ser(&CreateSessionResponse {
                    status: SessionStatus::Ok,
                    resource: res,
                    session_nb: nb,
                }),
            )));
        }
        // module advertises its CAIDs on the CA session
        let ca_nb = s
            .session
            .sessions()
            .into_iter()
            .find(|&(_, r)| r == CONDITIONAL_ACCESS_SUPPORT)
            .map(|(n, _)| n)
            .expect("CA session open");
        s.handle(Event::Readable(&r_apdu(
            ca_nb,
            &ser(&CaInfo {
                ca_system_ids: vec![0x0B00, 0x1800],
            }),
        )));
        s
    }

    #[test]
    fn descramble_filters_then_queries_then_oks() {
        use dvb_ci::objects::ca_pmt::CaPmtCmdId;
        use dvb_ci::objects::ca_pmt_reply::{CaEnable, CaPmtReply};
        use dvb_ci::resource::CONDITIONAL_ACCESS_SUPPORT;

        let mut s = stack_with_ca_session();
        let ca_nb = s
            .session
            .sessions()
            .into_iter()
            .find(|&(_, r)| r == CONDITIONAL_ACCESS_SUPPORT)
            .map(|(n, _)| n)
            .unwrap();

        let pmt = build_pmt();
        let mut query_actions = s.handle(Event::Host(HostRequest::Descramble(&pmt)));
        // The query is queued behind the in-flight link; the module's SB flushes
        // it (one block per turn — #337).
        query_actions.extend(pump_sbs(&mut s));
        // A ca_pmt with cmd_id = query was sent, filtered to the CAM's CAIDs.
        let q = first_ca_pmt(&query_actions).expect("ca_pmt query sent");
        assert_eq!(q.cmd_id, CaPmtCmdId::Query);
        assert_eq!(
            q.program_ca_descriptors.as_slice(),
            &[0x09, 0x04, 0x0B, 0x00, 0xE1, 0x00]
        );

        // Module replies that descrambling is possible → stack auto-sends OK.
        let mut ok_actions = s.handle(Event::Readable(&r_apdu(
            ca_nb,
            &ser(&CaPmtReply {
                program_number: 1,
                version_number: 1,
                current_next_indicator: true,
                ca_enable: Some(CaEnable::Possible),
                streams: vec![],
            }),
        )));
        assert!(ok_actions.iter().any(|a| matches!(
            a,
            Action::Notify(Notification::CaPmtReply {
                descrambling_ok: true,
                ..
            })
        )));
        ok_actions.extend(pump_sbs(&mut s));
        assert!(
            all_ca_pmts(&ok_actions)
                .iter()
                .any(|c| c.cmd_id == CaPmtCmdId::OkDescrambling),
            "ca_pmt ok_descrambling sent after a positive reply"
        );
    }

    #[test]
    fn descramble_reply_not_possible_sends_no_ok() {
        use dvb_ci::objects::ca_pmt::CaPmtCmdId;
        use dvb_ci::objects::ca_pmt_reply::CaPmtReply;
        use dvb_ci::resource::CONDITIONAL_ACCESS_SUPPORT;

        let mut s = stack_with_ca_session();
        let ca_nb = s
            .session
            .sessions()
            .into_iter()
            .find(|&(_, r)| r == CONDITIONAL_ACCESS_SUPPORT)
            .map(|(n, _)| n)
            .unwrap();
        let pmt = build_pmt();
        let mut actions = s.handle(Event::Host(HostRequest::Descramble(&pmt)));
        // ca_enable = None → descrambling not possible → no OK follow-up.
        actions.extend(s.handle(Event::Readable(&r_apdu(
            ca_nb,
            &ser(&CaPmtReply {
                program_number: 1,
                version_number: 1,
                current_next_indicator: true,
                ca_enable: None,
                streams: vec![],
            }),
        ))));
        actions.extend(pump_sbs(&mut s));
        // The query may be flushed, but no ok_descrambling is ever sent.
        assert!(
            all_ca_pmts(&actions)
                .iter()
                .all(|c| c.cmd_id != CaPmtCmdId::OkDescrambling),
            "no ok_descrambling without a positive reply"
        );
    }

    /// Whether any written frame carries the 3-byte APDU tag `want`.
    fn wrote_apdu(actions: &[Action], want: [u8; 3]) -> bool {
        actions
            .iter()
            .any(|a| matches!(a, Action::Write(w) if w.windows(3).any(|x| x == want)))
    }

    #[test]
    fn mmi_menu_answer_sends_menu_answ() {
        let mut s = stack_with_ca_session();
        let mut acts = s.handle(Event::Host(HostRequest::MmiMenuAnswer(2)));
        acts.extend(pump_sbs(&mut s));
        // menu_answ APDU (9F 88 0B) reaches the wire.
        assert!(wrote_apdu(&acts, [0x9F, 0x88, 0x0B]));
    }

    #[test]
    fn mmi_enquiry_answer_sends_answ() {
        let mut s = stack_with_ca_session();
        let mut acts = s.handle(Event::Host(HostRequest::MmiEnquiryAnswer(b"1234")));
        acts.extend(pump_sbs(&mut s));
        // answ APDU (9F 88 08) reaches the wire.
        assert!(wrote_apdu(&acts, [0x9F, 0x88, 0x08]));
    }

    /// Parse every `ca_pmt` (tag `9F 80 32`) found in the written frames,
    /// returning each one's `cmd_id` + programme CA-descriptor bytes (owned).
    fn all_ca_pmts(actions: &[Action]) -> Vec<CaPmtSummary> {
        use dvb_ci::objects::ca_pmt::CaPmt;
        use dvb_common::Parse;
        let tag = [0x9F, 0x80, 0x32];
        let mut out = Vec::new();
        for a in actions {
            if let Action::Write(w) = a {
                if let Some(pos) = w.windows(3).position(|x| x == tag) {
                    if let Ok(p) = CaPmt::parse(&w[pos..]) {
                        out.push(CaPmtSummary {
                            cmd_id: p.cmd_id.expect("programme cmd_id present"),
                            program_ca_descriptors: p.program_ca_descriptors.to_vec(),
                        });
                    }
                }
            }
        }
        out
    }

    /// The first `ca_pmt` in the written frames.
    fn first_ca_pmt(actions: &[Action]) -> Option<CaPmtSummary> {
        all_ca_pmts(actions).into_iter().next()
    }

    struct CaPmtSummary {
        cmd_id: dvb_ci::objects::ca_pmt::CaPmtCmdId,
        program_ca_descriptors: Vec<u8>,
    }
}
