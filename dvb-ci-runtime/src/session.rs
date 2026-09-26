//! SPDU session layer — a sans-IO mechanism over the transport layer
//! (ETSI EN 50221 §7.2).
//!
//! Multiplexes logical sessions (one per resource in use) over the transport
//! connection: allocates/tracks `session_nb`s, answers `open_session_request`
//! for resources the host advertises, opens `create_session` on demand, and
//! routes `session_number`+APDU to/from the resource bound to a session. It is
//! mechanism only — *which* resources the host provides is the caller's policy,
//! supplied as the `provides` predicate to [`SessionLayer::on_spdu`].

use std::collections::BTreeMap;

use broadcast_common::{Parse, Serialize};
use dvb_ci::resource::ResourceId;
use dvb_ci::spdu::{
    CloseSessionRequest, CloseSessionResponse, CreateSessionResponse, OpenSessionRequest,
    OpenSessionResponse, SessionNumber, SessionStatus, tags,
};

/// r10-W-20: previously matched (not `expect`ed) specifically to avoid a
/// `Debug` bound, silently emitting an empty `Vec` on error — which then
/// went out to real CI hardware as an indistinguishable-from-deliberate
/// empty SPDU rather than surfacing the failure at all. Panicking is
/// louder and safer than silently corrupting the wire exchange.
fn ser<S: Serialize>(s: &S) -> Vec<u8>
where
    S::Error: core::fmt::Debug,
{
    let mut b = vec![0u8; s.serialized_len()];
    let n = s
        .serialize_into(&mut b)
        .expect("value must satisfy every wire constraint of its own type");
    b.truncate(n);
    b
}

/// What the session layer wants done after handling one SPDU.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SessionOut {
    /// SPDUs to hand down to the transport layer (each becomes a `T_Data_Last`).
    pub spdus: Vec<Vec<u8>>,
    /// `(session_nb, apdu_bytes)` to pass up to the resource layer.
    pub apdus: Vec<(u16, Vec<u8>)>,
    /// Sessions newly opened (`session_nb`, bound resource).
    pub opened: Vec<(u16, ResourceId)>,
    /// `session_nb`s that closed.
    pub closed: Vec<u16>,
}

/// The session table + `session_nb` allocator.
#[derive(Debug, Default)]
pub struct SessionLayer {
    sessions: BTreeMap<u16, ResourceId>,
    next: u16,
}

impl SessionLayer {
    /// New, empty session layer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: BTreeMap::new(),
            next: 1, // session_nb 0 is reserved
        }
    }

    /// Resource bound to `session_nb`, if open.
    #[must_use]
    pub fn resource_of(&self, session_nb: u16) -> Option<ResourceId> {
        self.sessions.get(&session_nb).copied()
    }

    /// All open `(session_nb, resource)` pairs, ascending by `session_nb`.
    #[must_use]
    pub fn sessions(&self) -> Vec<(u16, ResourceId)> {
        self.sessions.iter().map(|(&n, &r)| (n, r)).collect()
    }

    /// Number of open sessions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Whether there are no open sessions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// r10-W-18: probe past any `session_nb` still in `self.sessions` —
    /// `next` wraps around after 65535 allocations (a long-running slot with
    /// enough session churn, e.g. repeated MMI dialogues, can reach this),
    /// and without the probe a wrapped-around number could alias a session
    /// that is still open, silently corrupting its `resource_of` mapping.
    fn alloc(&mut self) -> u16 {
        let start = self.next;
        loop {
            let nb = self.next;
            self.next = self.next.checked_add(1).filter(|&n| n != 0).unwrap_or(1);
            if !self.sessions.contains_key(&nb) {
                return nb;
            }
            // Every one of the 65535 representable numbers is in use — an
            // exhaustion this extreme is not realistic for one CI slot's
            // handful of concurrent resource sessions, but return the
            // original candidate rather than loop forever.
            if self.next == start {
                return nb;
            }
        }
    }

    /// Open a session to a **module-provided** resource (host-initiated):
    /// returns the `open_session_request` SPDU to send. Sessions are opened the
    /// same way in both directions (§8.4.1) — the host sends
    /// `open_session_request`, and the module (the resource provider) assigns the
    /// `session_nb` in its `open_session_response`. (`create_session`/0x93 is a
    /// resource-manager-internal primitive; a real CAM rejects it with
    /// `status=0xF0` — verified live against an AlphaCrypt.) The session is
    /// recorded once the module's `open_session_response(ok)` arrives.
    pub fn create_session(&mut self, resource: ResourceId) -> Vec<u8> {
        ser(&OpenSessionRequest { resource })
    }

    /// Wrap an APDU for sending on `session_nb` (`session_number` + body).
    #[must_use]
    pub fn send_apdu(&self, session_nb: u16, apdu: &[u8]) -> Vec<u8> {
        let mut v = ser(&SessionNumber { session_nb });
        v.extend_from_slice(apdu);
        v
    }

    /// Begin closing `session_nb`: returns the `close_session_request` SPDU.
    pub fn close(&mut self, session_nb: u16) -> Vec<u8> {
        self.sessions.remove(&session_nb);
        ser(&CloseSessionRequest { session_nb })
    }

    /// Bind a module-chosen `session_nb` to `resource` (from
    /// `open_session_response`/`create_session_response`), returning
    /// whether the binding is now in effect.
    ///
    /// r10-W-18: `session_nb` is the MODULE's own choice, not ours — a
    /// misbehaving (or hostile) module reusing a number already bound to a
    /// DIFFERENT resource would otherwise silently alias that resource's
    /// session, misrouting its APDUs from then on. `0` is also rejected: it
    /// is the reserved value `alloc` never hands out. Re-asserting the SAME
    /// resource on an already-open `session_nb` is treated as an idempotent
    /// success, not a collision (a module may legitimately repeat its own
    /// prior response).
    fn try_bind_module_chosen(&mut self, session_nb: u16, resource: ResourceId) -> bool {
        if session_nb == 0 {
            return false;
        }
        match self.sessions.get(&session_nb) {
            Some(&existing) => existing == resource,
            None => {
                self.sessions.insert(session_nb, resource);
                true
            }
        }
    }

    /// Handle one inbound SPDU. `provides` answers "does the host provide this
    /// resource?" for an incoming `open_session_request`.
    pub fn on_spdu(&mut self, spdu: &[u8], provides: impl Fn(ResourceId) -> bool) -> SessionOut {
        let mut out = SessionOut::default();
        match spdu.first().copied() {
            // Module wants a host-provided resource.
            Some(tags::OPEN_SESSION_REQUEST) if let Ok(req) = OpenSessionRequest::parse(spdu) => {
                if provides(req.resource) {
                    let session_nb = self.alloc();
                    self.sessions.insert(session_nb, req.resource);
                    out.spdus.push(ser(&OpenSessionResponse {
                        status: SessionStatus::Ok,
                        resource: req.resource,
                        session_nb,
                    }));
                    out.opened.push((session_nb, req.resource));
                } else {
                    out.spdus.push(ser(&OpenSessionResponse {
                        status: SessionStatus::ResourceNonExistent,
                        resource: req.resource,
                        session_nb: 0,
                    }));
                }
            }
            // Module's reply to our open_session_request (host opened a
            // module-provided resource); the module assigns the session_nb.
            // r10-W-18: the assignment is not trusted blindly — see
            // `try_bind_module_chosen`.
            Some(tags::OPEN_SESSION_RESPONSE)
                if let Ok(resp) = OpenSessionResponse::parse(spdu)
                    && resp.status == SessionStatus::Ok =>
            {
                out.opened.extend(
                    self.try_bind_module_chosen(resp.session_nb, resp.resource)
                        .then_some((resp.session_nb, resp.resource)),
                );
            }
            // (Legacy) module's reply to a create_session, if any module uses it.
            Some(tags::CREATE_SESSION_RESPONSE)
                if let Ok(resp) = CreateSessionResponse::parse(spdu)
                    && resp.status == SessionStatus::Ok =>
            {
                out.opened.extend(
                    self.try_bind_module_chosen(resp.session_nb, resp.resource)
                        .then_some((resp.session_nb, resp.resource)),
                );
            }
            // Peer closes a session.
            Some(tags::CLOSE_SESSION_REQUEST) if let Ok(req) = CloseSessionRequest::parse(spdu) => {
                self.sessions.remove(&req.session_nb);
                out.spdus.push(ser(&CloseSessionResponse {
                    status: SessionStatus::Ok,
                    session_nb: req.session_nb,
                }));
                out.closed.push(req.session_nb);
            }
            // Ack of a close we initiated.
            Some(tags::CLOSE_SESSION_RESPONSE)
                if let Ok(resp) = CloseSessionResponse::parse(spdu) =>
            {
                self.sessions.remove(&resp.session_nb);
                out.closed.push(resp.session_nb);
            }
            // Data: session_number(nb) + APDU body.
            Some(tags::SESSION_NUMBER)
                if let Ok(sn) = SessionNumber::parse(spdu)
                    && spdu.len() > SessionNumber::HEADER_LEN =>
            {
                out.apdus
                    .push((sn.session_nb, spdu[SessionNumber::HEADER_LEN..].to_vec()));
            }
            _ => {}
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvb_ci::resource::{APPLICATION_INFORMATION, RESOURCE_MANAGER};

    fn provides_rm(r: ResourceId) -> bool {
        r == RESOURCE_MANAGER
    }

    #[test]
    fn open_request_for_provided_resource_grants_and_tracks() {
        let mut s = SessionLayer::new();
        let req = ser(&OpenSessionRequest {
            resource: RESOURCE_MANAGER,
        });
        let out = s.on_spdu(&req, provides_rm);
        assert_eq!(out.opened.len(), 1);
        let (nb, res) = out.opened[0];
        assert_eq!(res, RESOURCE_MANAGER);
        assert_eq!(s.resource_of(nb), Some(RESOURCE_MANAGER));
        // reply is an open_session_response with status ok
        let resp = OpenSessionResponse::parse(&out.spdus[0]).unwrap();
        assert_eq!(resp.status, SessionStatus::Ok);
        assert_eq!(resp.session_nb, nb);
    }

    #[test]
    fn open_request_for_absent_resource_denied() {
        let mut s = SessionLayer::new();
        let req = ser(&OpenSessionRequest {
            resource: APPLICATION_INFORMATION,
        });
        let out = s.on_spdu(&req, provides_rm);
        assert!(out.opened.is_empty());
        let resp = OpenSessionResponse::parse(&out.spdus[0]).unwrap();
        assert_eq!(resp.status, SessionStatus::ResourceNonExistent);
        assert!(s.is_empty());
    }

    #[test]
    fn create_session_tracked_on_ok_response() {
        let mut s = SessionLayer::new();
        let _spdu = s.create_session(APPLICATION_INFORMATION);
        // module replies ok for session 1
        let resp = ser(&CreateSessionResponse {
            status: SessionStatus::Ok,
            resource: APPLICATION_INFORMATION,
            session_nb: 1,
        });
        let out = s.on_spdu(&resp, |_| false);
        assert_eq!(out.opened, vec![(1, APPLICATION_INFORMATION)]);
        assert_eq!(s.resource_of(1), Some(APPLICATION_INFORMATION));
    }

    /// r10-W-18: a module-chosen `session_nb` that already maps to a
    /// DIFFERENT resource must be rejected, not silently overwritten —
    /// otherwise a misbehaving module aliases an existing session and its
    /// APDUs are misrouted from then on.
    #[test]
    fn module_chosen_session_nb_colliding_with_a_different_resource_is_rejected() {
        let mut s = SessionLayer::new();
        // Session 1 is already RESOURCE_MANAGER (opened via the ordinary path).
        let open = ser(&OpenSessionRequest {
            resource: RESOURCE_MANAGER,
        });
        let opened = s.on_spdu(&open, provides_rm).opened;
        let nb = opened[0].0;
        assert_eq!(s.resource_of(nb), Some(RESOURCE_MANAGER));

        // The module now claims that SAME session_nb for a totally
        // different resource via create_session_response.
        let colliding = ser(&CreateSessionResponse {
            status: SessionStatus::Ok,
            resource: APPLICATION_INFORMATION,
            session_nb: nb,
        });
        let out = s.on_spdu(&colliding, |_| false);
        assert!(
            out.opened.is_empty(),
            "a colliding module-chosen session_nb must not be reported as opened"
        );
        assert_eq!(
            s.resource_of(nb),
            Some(RESOURCE_MANAGER),
            "the original binding must survive the collision attempt"
        );
    }

    /// r10-W-18: `session_nb = 0` is reserved (`alloc` never hands it out)
    /// and must never be accepted as a module-chosen binding.
    #[test]
    fn module_chosen_session_nb_zero_is_rejected() {
        let mut s = SessionLayer::new();
        let resp = ser(&OpenSessionResponse {
            status: SessionStatus::Ok,
            resource: APPLICATION_INFORMATION,
            session_nb: 0,
        });
        let out = s.on_spdu(&resp, |_| false);
        assert!(out.opened.is_empty());
        assert_eq!(s.resource_of(0), None);
    }

    /// r10-W-18: `alloc` must skip any `session_nb` still open, even across
    /// the 65535 wraparound — otherwise a long-running slot with enough
    /// session churn eventually reissues a number that aliases a session
    /// that never closed.
    #[test]
    fn alloc_probes_past_a_still_open_session_across_wraparound() {
        let mut s = SessionLayer::new();
        // Force the allocator right up to the wraparound boundary.
        s.next = 65535;
        let first = s.alloc();
        assert_eq!(first, 65535);
        s.sessions.insert(first, APPLICATION_INFORMATION);
        // Next candidate would be `65535 -> checked_add overflows -> 1`.
        // Occupy 1 directly (as if it were opened earlier and never closed).
        s.sessions.insert(1, RESOURCE_MANAGER);
        let second = s.alloc();
        assert_ne!(
            second, 1,
            "alloc must not hand out a session_nb that is still open"
        );
        assert_eq!(s.resource_of(1), Some(RESOURCE_MANAGER), "untouched");
    }

    #[test]
    fn session_number_routes_apdu_up() {
        let mut s = SessionLayer::new();
        let apdu = [0x9F, 0x80, 0x21, 0x00];
        let mut spdu = ser(&SessionNumber { session_nb: 7 });
        spdu.extend_from_slice(&apdu);
        let out = s.on_spdu(&spdu, |_| false);
        assert_eq!(out.apdus, vec![(7, apdu.to_vec())]);
    }

    #[test]
    fn close_request_acks_and_removes() {
        let mut s = SessionLayer::new();
        // open one first
        let req = ser(&OpenSessionRequest {
            resource: RESOURCE_MANAGER,
        });
        let nb = s.on_spdu(&req, provides_rm).opened[0].0;
        // peer closes it
        let close = ser(&CloseSessionRequest { session_nb: nb });
        let out = s.on_spdu(&close, |_| false);
        assert_eq!(out.closed, vec![nb]);
        assert!(s.is_empty());
        // reply is a close_session_response
        assert_eq!(out.spdus[0][0], tags::CLOSE_SESSION_RESPONSE);
    }

    #[test]
    fn send_apdu_prefixes_session_number() {
        let s = SessionLayer::new();
        let wire = s.send_apdu(3, &[0xAA, 0xBB]);
        let sn = SessionNumber::parse(&wire).unwrap();
        assert_eq!(sn.session_nb, 3);
        assert_eq!(&wire[SessionNumber::HEADER_LEN..], &[0xAA, 0xBB]);
    }
}
