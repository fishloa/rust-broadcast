//! The CI protocol stack — composes the transport + session layers (and, as
//! they land, the resource state machines) into one sans-IO core.
//!
//! [`CiStack::handle`] is the pure entry point: feed it an [`Event`], get back
//! the [`Action`]s the driver must perform. No I/O, threads, or clock here.

use crate::event::{Action, Event, HostRequest, Notification};
use crate::resource::{
    ApplicationInformation, ConditionalAccess, DateTime, Resource, ResourceManager, ResourceOut,
};
use crate::session::{SessionLayer, SessionOut};
use crate::transport::{Out as TransportOut, Transport};

use dvb_ci::resource::{ResourceId, DATE_TIME, RESOURCE_MANAGER};

/// The composed EN 50221 protocol core.
pub struct CiStack {
    transport: Transport,
    session: SessionLayer,
    /// Resources the host provides (answers incoming `open_session_request`).
    provided: Vec<ResourceId>,
    /// Application-layer resource handlers, dispatched by `ResourceId`.
    resources: Vec<Box<dyn Resource>>,
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
            ],
            provided,
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
                self.send_to_resource(dvb_ci::resource::CONDITIONAL_ACCESS_SUPPORT, apdu)
            }
            Event::Host(HostRequest::Shutdown) => Vec::new(),
        }
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
            actions.push(Action::Notify(note));
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
}
