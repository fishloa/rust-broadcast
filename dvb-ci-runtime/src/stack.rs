//! The CI protocol stack — composes the transport + session layers (and, as
//! they land, the resource state machines) into one sans-IO core.
//!
//! [`CiStack::handle`] is the pure entry point: feed it an [`Event`], get back
//! the [`Action`]s the driver must perform. No I/O, threads, or clock here.

use crate::event::{Action, Event, HostRequest, Notification};
use crate::resource::{
    ApplicationInformation, ConditionalAccess, DateTime, HostControl, Mmi, Resource,
    ResourceManager, ResourceOut,
};
use crate::session::{SessionLayer, SessionOut};
use crate::transport::{Out as TransportOut, Transport};

use broadcast_common::{Parse, Serialize};
use dvb_ci::builder::{build_ca_pmt, build_ca_pmt_for_caids};
use dvb_ci::objects::ca_pmt::{CaPmtCmdId, CaPmtListManagement};
use dvb_ci::objects::mmi_high::{Answ, AnswId, MenuAnsw};
use dvb_ci::resource::{
    ResourceId, APPLICATION_INFORMATION, CONDITIONAL_ACCESS_SUPPORT, DATE_TIME, HOST_CONTROL, MMI,
    RESOURCE_MANAGER,
};
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
    /// Application-layer resource handlers, dispatched by `ResourceId`.
    resources: Vec<Box<dyn Resource>>,
    /// Resources the host **provides** — the module opens sessions to these, so
    /// the host accepts an incoming `open_session_request` for them
    /// (resource_manager, date_time). Module-provided resources are opened the
    /// other way, by the host's `create_session` (#340).
    host_provided: Vec<ResourceId>,
    /// `CA_system_id`s the CAM advertised in its `ca_info` (the descramble
    /// filter set; empty until `ca_info` arrives).
    cam_caids: Vec<u16>,
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
        // The host *provides* all six resources it implements; the module opens
        // a session to each (module → host `open_session_request`, host accepts),
        // and this is the list the RM advertises in its `profile` reply. Verified
        // on hardware (#340, live AlphaCrypt): the module opens sessions only to
        // resources the host advertises here — so application_information,
        // conditional_access and mmi MUST be advertised, or the module never
        // opens them and `ca_info` never arrives. (The earlier host-initiated
        // `create_session`/`open_session_request` for these was wrong: the module
        // rejects/ignores it.)
        let host_provided = vec![
            RESOURCE_MANAGER,
            APPLICATION_INFORMATION,
            CONDITIONAL_ACCESS_SUPPORT,
            DATE_TIME,
            MMI,
            HOST_CONTROL,
        ];
        Self {
            transport: Transport::new(1),
            session: SessionLayer::new(),
            resources: vec![
                Box::new(ResourceManager::new(host_provided.clone())),
                Box::new(ApplicationInformation),
                Box::new(ConditionalAccess),
                Box::new(DateTime::new()),
                Box::new(Mmi),
                Box::new(HostControl),
            ],
            host_provided,
            cam_caids: Vec::new(),
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
            Event::Host(HostRequest::DescramblePrograms(pmts)) => self.descramble_programs(pmts),
            Event::Host(HostRequest::AddProgram(pmt)) => self.add_program(pmt),
            Event::Host(HostRequest::RemoveProgram(pmt)) => self.remove_program(pmt),
            Event::Host(HostRequest::EnterMenu) => {
                let apdu = ser_apdu(&dvb_ci::objects::application_info::EnterMenu);
                self.send_to_resource(APPLICATION_INFORMATION, &apdu)
            }
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
        // Cache the CAM's advertised CAIDs so a later `descramble` can filter the
        // `ca_pmt` to them. (The `ca_pmt_reply` outcome is surfaced to the host as
        // `Notification::CaPmtReply`; no follow-up SPDU is needed — we send
        // `ok_descrambling` up front.)
        if let Notification::CaInfo { ca_system_ids } = note {
            self.cam_caids = ca_system_ids.clone();
        }
        Vec::new()
    }

    /// Begin a [`HostRequest::Descramble`]: build a CAID-filtered `ca_pmt` with
    /// `cmd_id = ok_descrambling` and send it.
    ///
    /// We do NOT send a `query` first: a real AlphaCrypt/Irdeto module does not
    /// reply to a `ca_pmt` query (verified live — the query was sent and the
    /// module stayed silent, so a query→reply→ok flow stalls forever). The
    /// module descrambles directly on `ok_descrambling` and reports the outcome
    /// via `ca_pmt_reply` (surfaced as `Notification::CaPmtReply`). This matches
    /// what oscam / libdvben50221 do in practice.
    fn descramble(&mut self, pmt: &[u8]) -> Vec<Action> {
        self.send_ca_pmt_for(pmt, CaPmtListManagement::Only, CaPmtCmdId::OkDescrambling)
    }

    /// Descramble a **set** of programmes in one CA-PMT list (§8.4.3.4): the
    /// first programme is sent `list_management = first` (or `only` if it is the
    /// sole programme), the interior ones `more`, the last `last`; all with
    /// `cmd_id = ok_descrambling`. Replaces any previously selected set. The
    /// per-programme `ca_pmt`s are serialised one-per-module-turn by the
    /// transport queue.
    fn descramble_programs(&mut self, pmts: &[&[u8]]) -> Vec<Action> {
        let mut actions = Vec::new();
        let n = pmts.len();
        for (i, pmt) in pmts.iter().enumerate() {
            let lm = match (n, i) {
                (1, _) => CaPmtListManagement::Only,
                (_, 0) => CaPmtListManagement::First,
                (_, i) if i == n - 1 => CaPmtListManagement::Last,
                _ => CaPmtListManagement::More,
            };
            actions.extend(self.send_ca_pmt_for(pmt, lm, CaPmtCmdId::OkDescrambling));
        }
        actions
    }

    /// Add one programme to the descrambled set without re-listing the rest
    /// (`list_management = add`, `cmd_id = ok_descrambling`).
    fn add_program(&mut self, pmt: &[u8]) -> Vec<Action> {
        self.send_ca_pmt_for(pmt, CaPmtListManagement::Add, CaPmtCmdId::OkDescrambling)
    }

    /// Remove one programme from the descrambled set (`list_management = update`,
    /// `cmd_id = not_selected` — tells the CAM to stop descrambling it).
    fn remove_program(&mut self, pmt: &[u8]) -> Vec<Action> {
        self.send_ca_pmt_for(pmt, CaPmtListManagement::Update, CaPmtCmdId::NotSelected)
    }

    /// Build a CAID-filtered `ca_pmt` for `pmt` with the given list-management +
    /// command id and send it on the conditional-access session.
    fn send_ca_pmt_for(
        &mut self,
        pmt: &[u8],
        list_management: CaPmtListManagement,
        cmd_id: CaPmtCmdId,
    ) -> Vec<Action> {
        match self.build_ca_pmt_bytes(pmt, list_management, cmd_id) {
            Ok(bytes) => self.send_to_resource(CONDITIONAL_ACCESS_SUPPORT, &bytes),
            Err(detail) => vec![Action::Notify(Notification::Error { detail })],
        }
    }

    /// Build a CAID-filtered `ca_pmt` APDU for `pmt` with the given
    /// list-management + command id. Filters to the CAM's advertised CAIDs once
    /// `ca_info` is known; falls back to all `CA_descriptor`s before then.
    fn build_ca_pmt_bytes(
        &self,
        pmt: &[u8],
        list_management: CaPmtListManagement,
        cmd_id: CaPmtCmdId,
    ) -> Result<Vec<u8>, String> {
        let parsed = PmtSection::parse(pmt).map_err(|e| format!("invalid PMT: {e}"))?;
        let built = if self.cam_caids.is_empty() {
            build_ca_pmt(&parsed, list_management, cmd_id)
        } else {
            build_ca_pmt_for_caids(&parsed, &self.cam_caids, list_management, cmd_id)
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
        // The module opens sessions to **host-provided** resources
        // (resource_manager, date_time); the host accepts those. Module-provided
        // resources (application_information, conditional_access, mmi) are opened
        // the other way — by the host's `create_session` (#340) — so an incoming
        // `open_session_request` for them is *not* accepted here.
        let host_provided = self.host_provided.clone();
        let SessionOut {
            spdus,
            apdus,
            opened,
            closed,
        } = self.session.on_spdu(spdu, |r| host_provided.contains(&r));

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
    use broadcast_common::Serialize;
    use dvb_ci::resource::RESOURCE_MANAGER;
    use dvb_ci::spdu::{tags as spdu_tags, OpenSessionRequest};
    use dvb_ci::tpdu::{tags as tpdu_tags, SbValue};

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
        let crc = broadcast_common::crc32_mpeg2::compute(&body);
        body.extend_from_slice(&crc.to_be_bytes());
        body
    }

    /// Drive the full handshake to open conditional-access + mmi sessions with
    /// the CAM's CAIDs learned, following the real flow (#340): module opens RM →
    /// host sends `profile_change` and `create_session`s the module-provided
    /// resources → module accepts each with `create_session_response`.
    fn stack_with_ca_session() -> CiStack {
        use dvb_ci::objects::ca_info::CaInfo;
        use dvb_ci::objects::resource_manager::Profile;
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
        // module sends its profile → host: CamReady + profile_change +
        // create_session for each module-provided resource (alloc nb 2,3,4).
        s.handle(Event::Readable(&r_apdu(
            1,
            &ser(&Profile {
                resources: vec![APPLICATION_INFORMATION, CONDITIONAL_ACCESS_SUPPORT, MMI],
            }),
        )));
        pump_sbs(&mut s); // flush profile_change + the first create_session
                          // The module accepts each create_session → its session opens (+ on_open
                          // enq); each acceptance frees the link so the next create_session flushes.
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
            pump_sbs(&mut s);
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
    fn descramble_sends_ok_descrambling_filtered() {
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

        // descramble() sends ca_pmt with cmd_id = ok_descrambling directly (no
        // query first — a real CAM doesn't reply to a query; verified live).
        let pmt = build_pmt();
        let mut actions = s.handle(Event::Host(HostRequest::Descramble(&pmt)));
        // Queued behind the in-flight link; the module's SB flushes it (one block
        // per turn — #337).
        actions.extend(pump_sbs(&mut s));
        let c = first_ca_pmt(&actions).expect("ca_pmt sent");
        assert_eq!(c.cmd_id, CaPmtCmdId::OkDescrambling);
        // CA descriptors filtered to the CAM's advertised CAIDs.
        assert_eq!(
            c.program_ca_descriptors.as_slice(),
            &[0x09, 0x04, 0x0B, 0x00, 0xE1, 0x00]
        );

        // The module's ca_pmt_reply is surfaced to the host (no follow-up SPDU).
        let reply = s.handle(Event::Readable(&r_apdu(
            ca_nb,
            &ser(&CaPmtReply {
                program_number: 1,
                version_number: 1,
                current_next_indicator: true,
                ca_enable: Some(CaEnable::Possible),
                streams: vec![],
            }),
        )));
        assert!(reply.iter().any(|a| matches!(
            a,
            Action::Notify(Notification::CaPmtReply {
                descrambling_ok: true,
                ..
            })
        )));
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
        use broadcast_common::Parse;
        use dvb_ci::objects::ca_pmt::CaPmt;
        let tag = [0x9F, 0x80, 0x32];
        let mut out = Vec::new();
        for a in actions {
            if let Action::Write(w) = a {
                if let Some(pos) = w.windows(3).position(|x| x == tag) {
                    if let Ok(p) = CaPmt::parse(&w[pos..]) {
                        out.push(CaPmtSummary {
                            list_management: p.list_management,
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
        list_management: dvb_ci::objects::ca_pmt::CaPmtListManagement,
        cmd_id: dvb_ci::objects::ca_pmt::CaPmtCmdId,
        program_ca_descriptors: Vec<u8>,
    }

    #[test]
    fn descramble_programs_emits_first_more_last() {
        use dvb_ci::objects::ca_pmt::{CaPmtCmdId, CaPmtListManagement};

        let mut s = stack_with_ca_session();
        let pmt = build_pmt();
        // Three programmes → first / more / last, all ok_descrambling.
        let mut acts = s.handle(Event::Host(HostRequest::DescramblePrograms(&[
            &pmt, &pmt, &pmt,
        ])));
        acts.extend(pump_sbs(&mut s));
        let lms: Vec<_> = all_ca_pmts(&acts)
            .iter()
            .map(|c| c.list_management)
            .collect();
        assert_eq!(
            lms,
            vec![
                CaPmtListManagement::First,
                CaPmtListManagement::More,
                CaPmtListManagement::Last,
            ]
        );
        assert!(all_ca_pmts(&acts)
            .iter()
            .all(|c| c.cmd_id == CaPmtCmdId::OkDescrambling));
    }

    #[test]
    fn add_and_remove_program_use_add_update() {
        use dvb_ci::objects::ca_pmt::{CaPmtCmdId, CaPmtListManagement};

        let mut s = stack_with_ca_session();
        let pmt = build_pmt();

        let mut add = s.handle(Event::Host(HostRequest::AddProgram(&pmt)));
        add.extend(pump_sbs(&mut s));
        let a = first_ca_pmt(&add).expect("add ca_pmt");
        assert_eq!(a.list_management, CaPmtListManagement::Add);
        assert_eq!(a.cmd_id, CaPmtCmdId::OkDescrambling);

        let mut rm = s.handle(Event::Host(HostRequest::RemoveProgram(&pmt)));
        rm.extend(pump_sbs(&mut s));
        let r = first_ca_pmt(&rm).expect("remove ca_pmt");
        assert_eq!(r.list_management, CaPmtListManagement::Update);
        assert_eq!(r.cmd_id, CaPmtCmdId::NotSelected);
    }
}
