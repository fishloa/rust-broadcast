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

use crate::resource::ser;
use broadcast_common::Parse;
use dvb_ci::resource::ResourceId;
use dvb_ci::spdu::{
    CloseSessionRequest, CloseSessionResponse, CreateSessionResponse, OpenSessionRequest,
    OpenSessionResponse, SessionNumber, SessionStatus, tags,
};

/// Most sessions this layer keeps open at once.
///
/// One CI slot's resources are a handful — the host advertises six, plus
/// whatever the module opens — and nothing here needs anywhere near the 65 535
/// representable `session_nb`s. Unbounded, a module repeating
/// `open_session_request` for one resource grew the table to that size and had
/// its resource handler ticked once per duplicate, running (for example) the
/// `date_time` resend timer N times as fast (audit a20-44). A request past
/// this bound, and a request for a resource that already has an open session,
/// are answered `session_status = resource_busy` (EN 50221 Table 7).
const MAX_SESSIONS: usize = 32;

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

    /// The lowest open `session_nb` bound to `resource`, if any.
    ///
    /// Walks only the open sessions (a handful per slot), where the caller
    /// used to probe every one of the 65 535 representable numbers through
    /// [`resource_of`](Self::resource_of) (audit r10-O-9).
    #[must_use]
    pub fn session_for(&self, resource: ResourceId) -> Option<u16> {
        self.sessions
            .iter()
            .find(|&(_, &r)| r == resource)
            .map(|(&n, _)| n)
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
    fn alloc(&mut self) -> Option<u16> {
        let start = self.next;
        loop {
            let nb = self.next;
            self.next = self.next.checked_add(1).filter(|&n| n != 0).unwrap_or(1);
            if !self.sessions.contains_key(&nb) {
                return Some(nb);
            }
            // Every one of the 65535 representable numbers is in use. Callers
            // hold `MAX_SESSIONS`, so this is unreachable in practice; report
            // the exhaustion instead of aliasing a live session.
            if self.next == start {
                return None;
            }
        }
    }

    /// Bind `session_nb` to `resource` directly, bypassing the open-request
    /// policy — a test needs the several-sessions-for-one-resource state that
    /// policy now refuses, to show the resource layer still ticks it once.
    #[cfg(test)]
    pub(crate) fn bind_for_test(&mut self, session_nb: u16, resource: ResourceId) {
        self.sessions.insert(session_nb, resource);
    }

    /// Whether `resource` already has an open session.
    fn resource_is_open(&self, resource: ResourceId) -> bool {
        self.sessions.values().any(|&r| r == resource)
    }

    /// Open a session to a **module-provided** resource (host-initiated):
    /// returns the `open_session_request` SPDU to send. Sessions are opened the
    /// same way in both directions (§8.4.1) — the host sends
    /// `open_session_request`, and the module (the resource provider) assigns the
    /// `session_nb` in its `open_session_response`. (`create_session`/0x93 is a
    /// resource-manager-internal primitive; a real CAM rejects it with
    /// `status=0xF0` — verified live against an AlphaCrypt.) The session is
    /// recorded once the module's `open_session_response(ok)` arrives.
    pub fn create_session(&mut self, resource: ResourceId) -> dvb_ci::Result<Vec<u8>> {
        ser(&OpenSessionRequest {
            length_form: dvb_ci::length::LengthForm::Minimal,
            resource,
        })
    }

    /// Wrap an APDU for sending on `session_nb` (`session_number` + body).
    pub fn send_apdu(&self, session_nb: u16, apdu: &[u8]) -> dvb_ci::Result<Vec<u8>> {
        let mut v = ser(&SessionNumber {
            length_form: dvb_ci::length::LengthForm::Minimal,
            session_nb,
        })?;
        v.extend_from_slice(apdu);
        Ok(v)
    }

    /// Begin closing `session_nb`: returns the `close_session_request` SPDU.
    pub fn close(&mut self, session_nb: u16) -> dvb_ci::Result<Vec<u8>> {
        self.sessions.remove(&session_nb);
        ser(&CloseSessionRequest {
            length_form: dvb_ci::length::LengthForm::Minimal,
            session_nb,
        })
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
                if self.resource_is_open(resource) || self.sessions.len() >= MAX_SESSIONS {
                    return false;
                }
                self.sessions.insert(session_nb, resource);
                true
            }
        }
    }

    /// Handle one inbound SPDU. `provides` answers "does the host provide this
    /// resource?" for an incoming `open_session_request`.
    pub fn on_spdu(
        &mut self,
        spdu: &[u8],
        provides: impl Fn(ResourceId) -> bool,
    ) -> dvb_ci::Result<SessionOut> {
        let mut out = SessionOut::default();
        match spdu.first().copied() {
            // Module wants a host-provided resource.
            Some(tags::OPEN_SESSION_REQUEST) => {
                // A malformed CAM SPDU is surfaced (`?` -> Notification::Error),
                // never dropped silently.
                let req = OpenSessionRequest::parse(spdu)?;
                let granted = provides(req.resource)
                    && !self.resource_is_open(req.resource)
                    && self.sessions.len() < MAX_SESSIONS;
                let session_nb = if granted { self.alloc() } else { None };
                let Some(session_nb) = session_nb else {
                    out.spdus.push(ser(&OpenSessionResponse {
                        length_form: dvb_ci::length::LengthForm::Minimal,
                        status: if provides(req.resource) {
                            SessionStatus::ResourceBusy
                        } else {
                            SessionStatus::ResourceNonExistent
                        },
                        resource: req.resource,
                        session_nb: 0,
                    })?);
                    return Ok(out);
                };
                self.sessions.insert(session_nb, req.resource);
                out.spdus.push(ser(&OpenSessionResponse {
                    length_form: dvb_ci::length::LengthForm::Minimal,
                    status: SessionStatus::Ok,
                    resource: req.resource,
                    session_nb,
                })?);
                out.opened.push((session_nb, req.resource));
            }
            // Module's reply to our open_session_request (host opened a
            // module-provided resource); the module assigns the session_nb.
            // r10-W-18: the assignment is not trusted blindly — see
            // `try_bind_module_chosen`.
            Some(tags::OPEN_SESSION_RESPONSE) => {
                let resp = OpenSessionResponse::parse(spdu)?;
                if resp.status == SessionStatus::Ok {
                    out.opened.extend(
                        self.try_bind_module_chosen(resp.session_nb, resp.resource)
                            .then_some((resp.session_nb, resp.resource)),
                    );
                }
            }
            // (Legacy) module's reply to a create_session, if any module uses it.
            Some(tags::CREATE_SESSION_RESPONSE) => {
                let resp = CreateSessionResponse::parse(spdu)?;
                if resp.status == SessionStatus::Ok {
                    out.opened.extend(
                        self.try_bind_module_chosen(resp.session_nb, resp.resource)
                            .then_some((resp.session_nb, resp.resource)),
                    );
                }
            }
            // Peer closes a session.
            Some(tags::CLOSE_SESSION_REQUEST) => {
                let req = CloseSessionRequest::parse(spdu)?;
                let known = self.sessions.remove(&req.session_nb).is_some();
                out.spdus.push(ser(&CloseSessionResponse {
                    length_form: dvb_ci::length::LengthForm::Minimal,
                    status: if known {
                        SessionStatus::Ok
                    } else {
                        SessionStatus::ResourceNonExistent
                    },
                    session_nb: req.session_nb,
                })?);
                if known {
                    out.closed.push(req.session_nb);
                }
            }
            // Ack of a close we initiated.
            Some(tags::CLOSE_SESSION_RESPONSE) => {
                let resp = CloseSessionResponse::parse(spdu)?;
                self.sessions.remove(&resp.session_nb);
                out.closed.push(resp.session_nb);
            }
            // Data: session_number(nb) + APDU body.
            Some(tags::SESSION_NUMBER) => {
                let sn = SessionNumber::parse(spdu)?;
                // The header is `sn.header_len()` long: 4 bytes when the
                // `length_field` is minimally coded, longer for a CAM that
                // codes it in a long form (`90 82 00 02 ..`).
                let header_len = sn.header_len();
                if spdu.len() > header_len {
                    out.apdus.push((sn.session_nb, spdu[header_len..].to_vec()));
                }
            }
            _ => {}
        }
        Ok(out)
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
    fn session_for_returns_the_lowest_open_session_of_a_resource() {
        let mut s = SessionLayer::new();
        assert_eq!(s.session_for(RESOURCE_MANAGER), None);
        let rm_req = ser(&OpenSessionRequest {
            length_form: dvb_ci::length::LengthForm::Minimal,
            resource: RESOURCE_MANAGER,
        })
        .unwrap();
        let ai_req = ser(&OpenSessionRequest {
            length_form: dvb_ci::length::LengthForm::Minimal,
            resource: APPLICATION_INFORMATION,
        })
        .unwrap();
        let all = |_: ResourceId| true;
        let rm = s.on_spdu(&rm_req, all).unwrap().opened[0].0;
        let ai = s.on_spdu(&ai_req, all).unwrap().opened[0].0;
        assert_ne!(rm, ai);
        assert_eq!(s.session_for(RESOURCE_MANAGER), Some(rm));
        assert_eq!(s.session_for(APPLICATION_INFORMATION), Some(ai));
        // Closing the session removes it from the lookup.
        s.close(rm).unwrap();
        assert_eq!(s.session_for(RESOURCE_MANAGER), None);
    }

    #[test]
    fn open_request_for_provided_resource_grants_and_tracks() {
        let mut s = SessionLayer::new();
        let req = ser(&OpenSessionRequest {
            length_form: dvb_ci::length::LengthForm::Minimal,
            resource: RESOURCE_MANAGER,
        })
        .unwrap();
        let out = s.on_spdu(&req, provides_rm).unwrap();
        assert_eq!(out.opened.len(), 1);
        let (nb, res) = out.opened[0];
        assert_eq!(res, RESOURCE_MANAGER);
        assert_eq!(s.resource_of(nb), Some(RESOURCE_MANAGER));
        // reply is an open_session_response with status ok
        let resp = OpenSessionResponse::parse(&out.spdus[0]).unwrap();
        assert_eq!(resp.status, SessionStatus::Ok);
        assert_eq!(resp.session_nb, nb);
    }

    /// A module repeating `open_session_request` for one resource must not
    /// grow the session table (audit a20-44): the resource already has its
    /// session, so the repeat is refused `resource_busy` and nothing new is
    /// opened.
    #[test]
    fn repeated_open_session_request_for_one_resource_is_refused_as_busy() {
        let mut s = SessionLayer::new();
        let req = ser(&OpenSessionRequest {
            length_form: dvb_ci::length::LengthForm::Minimal,
            resource: RESOURCE_MANAGER,
        })
        .unwrap();
        let first = s.on_spdu(&req, provides_rm).unwrap();
        assert_eq!(first.opened.len(), 1);

        for _ in 0..16 {
            let out = s.on_spdu(&req, provides_rm).unwrap();
            assert!(
                out.opened.is_empty(),
                "a duplicate open_session_request must not open another session"
            );
            let resp = OpenSessionResponse::parse(&out.spdus[0]).unwrap();
            assert_eq!(resp.status, SessionStatus::ResourceBusy);
        }
        assert_eq!(s.len(), 1, "the table must still hold exactly one session");
    }

    /// The table is bounded: past `MAX_SESSIONS` distinct resources every
    /// further request is refused `resource_busy` instead of being granted a
    /// 65 535th session (audit a20-44).
    #[test]
    fn session_table_is_capped_at_max_sessions() {
        let mut s = SessionLayer::new();
        let open = |resource: ResourceId| {
            ser(&OpenSessionRequest {
                length_form: dvb_ci::length::LengthForm::Minimal,
                resource,
            })
            .unwrap()
        };

        for resource in 0..MAX_SESSIONS as u32 {
            let out = s
                .on_spdu(&open(ResourceId(0x0100_0000 + resource)), |_| true)
                .unwrap();
            assert_eq!(out.opened.len(), 1, "session {resource} must be granted");
        }
        assert_eq!(s.len(), MAX_SESSIONS);

        let out = s
            .on_spdu(&open(ResourceId(0x0100_0000 + MAX_SESSIONS as u32)), |_| {
                true
            })
            .unwrap();
        assert!(
            out.opened.is_empty(),
            "the cap must refuse the next session"
        );
        let resp = OpenSessionResponse::parse(&out.spdus[0]).unwrap();
        assert_eq!(resp.status, SessionStatus::ResourceBusy);
        assert_eq!(s.len(), MAX_SESSIONS);
    }

    /// #2946: the cap also holds on the module-chosen bind path — the one a
    /// CAM drives by sending `open_session_response`s of its own.
    #[test]
    fn module_chosen_sessions_are_capped_at_max_sessions() {
        let response = |session_nb: u16, resource: u32, status: SessionStatus| {
            ser(&OpenSessionResponse {
                length_form: dvb_ci::length::LengthForm::Minimal,
                status,
                resource: ResourceId(0x0100_0000 + resource),
                session_nb,
            })
            .unwrap()
        };

        let mut s = SessionLayer::new();
        for i in 0..MAX_SESSIONS as u16 {
            let out = s
                .on_spdu(&response(i + 1, u32::from(i), SessionStatus::Ok), |_| true)
                .unwrap();
            assert_eq!(
                out.opened,
                vec![(i + 1, ResourceId(0x0100_0000 + u32::from(i)))]
            );
        }
        assert_eq!(s.len(), MAX_SESSIONS);

        let out = s
            .on_spdu(&response(100, 0x0000_00FF, SessionStatus::Ok), |_| true)
            .unwrap();
        assert!(
            out.opened.is_empty(),
            "the cap must refuse a further module-chosen session"
        );
        assert_eq!(s.len(), MAX_SESSIONS);

        let out = s
            .on_spdu(&response(101, 0, SessionStatus::Ok), |_| true)
            .unwrap();
        assert!(
            out.opened.is_empty(),
            "a second session for an already-open resource is refused"
        );
        assert_eq!(s.resource_of(101), None);
        assert_eq!(s.len(), MAX_SESSIONS);
    }

    /// #2946: only an `ok` status binds a module-chosen session.
    #[test]
    fn a_non_ok_open_session_response_never_binds() {
        let mut s = SessionLayer::new();
        let resp = ser(&OpenSessionResponse {
            length_form: dvb_ci::length::LengthForm::Minimal,
            status: SessionStatus::ResourceBusy,
            resource: ResourceId(0x0004_1001),
            session_nb: 7,
        })
        .unwrap();
        let out = s.on_spdu(&resp, |_| true).unwrap();
        assert!(out.opened.is_empty());
        assert_eq!(s.resource_of(7), None);
        assert!(s.is_empty());
    }

    /// #2946: the host-initiated open request is the tag-`0x91` SPDU with the
    /// resource identifier, hand-derived from ETSI EN 50221 §7.2.6.1.
    #[test]
    fn create_session_emits_the_open_session_request_spdu() {
        let mut s = SessionLayer::new();
        assert_eq!(
            s.create_session(ResourceId(0x0004_1001)).unwrap(),
            vec![0x91, 0x04, 0x00, 0x04, 0x10, 0x01]
        );
    }

    /// #2946: a module acknowledging a close the host initiated removes the
    /// session and reports it closed.
    #[test]
    fn a_close_session_response_removes_the_session() {
        let mut s = SessionLayer::new();
        for i in 0..5u32 {
            let req = ser(&OpenSessionRequest {
                length_form: dvb_ci::length::LengthForm::Minimal,
                resource: ResourceId(0x0100_0000 + i),
            })
            .unwrap();
            let out = s.on_spdu(&req, |_| true).unwrap();
            assert_eq!(out.opened[0].0, u16::try_from(i).unwrap() + 1);
        }
        assert_eq!(s.len(), 5);

        let resp = ser(&CloseSessionResponse {
            length_form: dvb_ci::length::LengthForm::Minimal,
            status: SessionStatus::Ok,
            session_nb: 5,
        })
        .unwrap();
        let out = s.on_spdu(&resp, |_| false).unwrap();
        assert_eq!(out.closed, vec![5]);
        assert_eq!(s.resource_of(5), None);
        assert_eq!(s.len(), 4);
    }

    /// #2946: a `session_number` SPDU carries an APDU only when a body follows
    /// the header.
    #[test]
    fn a_session_number_spdu_without_a_body_yields_no_apdu() {
        let mut s = SessionLayer::new();
        let header_only = ser(&SessionNumber {
            length_form: dvb_ci::length::LengthForm::Minimal,
            session_nb: 3,
        })
        .unwrap();
        assert_eq!(header_only.len(), 4);
        assert!(s.on_spdu(&header_only, |_| false).unwrap().apdus.is_empty());

        let mut with_body = header_only;
        with_body.push(0xAA);
        assert_eq!(
            s.on_spdu(&with_body, |_| false).unwrap().apdus,
            vec![(3, vec![0xAA])]
        );
    }

    #[test]
    fn open_request_for_absent_resource_denied() {
        let mut s = SessionLayer::new();
        let req = ser(&OpenSessionRequest {
            length_form: dvb_ci::length::LengthForm::Minimal,
            resource: APPLICATION_INFORMATION,
        })
        .unwrap();
        let out = s.on_spdu(&req, provides_rm).unwrap();
        assert!(out.opened.is_empty());
        let resp = OpenSessionResponse::parse(&out.spdus[0]).unwrap();
        assert_eq!(resp.status, SessionStatus::ResourceNonExistent);
        assert!(s.is_empty());
    }

    #[test]
    fn create_session_tracked_on_ok_response() {
        let mut s = SessionLayer::new();
        let _spdu = s.create_session(APPLICATION_INFORMATION).unwrap();
        // module replies ok for session 1
        let resp = ser(&CreateSessionResponse {
            length_form: dvb_ci::length::LengthForm::Minimal,
            status: SessionStatus::Ok,
            resource: APPLICATION_INFORMATION,
            session_nb: 1,
        })
        .unwrap();
        let out = s.on_spdu(&resp, |_| false).unwrap();
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
            length_form: dvb_ci::length::LengthForm::Minimal,
            resource: RESOURCE_MANAGER,
        })
        .unwrap();
        let opened = s.on_spdu(&open, provides_rm).unwrap().opened;
        let nb = opened[0].0;
        assert_eq!(s.resource_of(nb), Some(RESOURCE_MANAGER));

        // The module now claims that SAME session_nb for a totally
        // different resource via create_session_response.
        let colliding = ser(&CreateSessionResponse {
            length_form: dvb_ci::length::LengthForm::Minimal,
            status: SessionStatus::Ok,
            resource: APPLICATION_INFORMATION,
            session_nb: nb,
        })
        .unwrap();
        let out = s.on_spdu(&colliding, |_| false).unwrap();
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
            length_form: dvb_ci::length::LengthForm::Minimal,
            status: SessionStatus::Ok,
            resource: APPLICATION_INFORMATION,
            session_nb: 0,
        })
        .unwrap();
        let out = s.on_spdu(&resp, |_| false).unwrap();
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
        let first = s.alloc().expect("65535 is free");
        assert_eq!(first, 65535);
        s.sessions.insert(first, APPLICATION_INFORMATION);
        // Next candidate would be `65535 -> checked_add overflows -> 1`.
        // Occupy 1 directly (as if it were opened earlier and never closed).
        s.sessions.insert(1, RESOURCE_MANAGER);
        let second = s.alloc().expect("65535 and 1 are the only sessions open");
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
        let mut spdu = ser(&SessionNumber {
            length_form: dvb_ci::length::LengthForm::Minimal,
            session_nb: 7,
        })
        .unwrap();
        spdu.extend_from_slice(&apdu);
        let out = s.on_spdu(&spdu, |_| false).unwrap();
        assert_eq!(out.apdus, vec![(7, apdu.to_vec())]);
    }

    #[test]
    fn close_request_acks_and_removes() {
        let mut s = SessionLayer::new();
        // open one first
        let req = ser(&OpenSessionRequest {
            length_form: dvb_ci::length::LengthForm::Minimal,
            resource: RESOURCE_MANAGER,
        })
        .unwrap();
        let nb = s.on_spdu(&req, provides_rm).unwrap().opened[0].0;
        // peer closes it
        let close = ser(&CloseSessionRequest {
            length_form: dvb_ci::length::LengthForm::Minimal,
            session_nb: nb,
        })
        .unwrap();
        let out = s.on_spdu(&close, |_| false).unwrap();
        assert_eq!(out.closed, vec![nb]);
        assert!(s.is_empty());
        // reply is a close_session_response
        assert_eq!(out.spdus[0][0], tags::CLOSE_SESSION_RESPONSE);
    }

    #[test]
    fn send_apdu_prefixes_session_number() {
        let s = SessionLayer::new();
        let wire = s.send_apdu(3, &[0xAA, 0xBB]).unwrap();
        let sn = SessionNumber::parse(&wire).unwrap();
        assert_eq!(sn.session_nb, 3);
        assert_eq!(&wire[SessionNumber::HEADER_LEN..], &[0xAA, 0xBB]);
    }

    /// r10-W-4: a CAM may code the `session_number` `length_field` in a long
    /// form (`90 82 00 02 ..`, the `82 00 NN` shape captured on hardware in
    /// #337 at the TPDU layer). The APDU body then starts after the longer
    /// header; slicing at the fixed 4-byte `HEADER_LEN` handed the APDU layer
    /// two stray session bytes.
    #[test]
    fn session_number_in_a_long_length_form_still_yields_the_apdu() {
        let mut s = SessionLayer::new();
        let wire = [0x90, 0x82, 0x00, 0x02, 0x00, 0x03, 0xAA, 0xBB];
        let out = s.on_spdu(&wire, |_| false).unwrap();
        assert_eq!(out.apdus, vec![(3, vec![0xAA, 0xBB])]);
    }

    #[test]
    fn closing_an_unallocated_session_is_answered_not_allocated() {
        let mut s = SessionLayer::new();
        let req = ser(&CloseSessionRequest {
            length_form: dvb_ci::length::LengthForm::Minimal,
            session_nb: 77,
        })
        .unwrap();
        let out = s.on_spdu(&req, |_| true).unwrap();
        let resp = CloseSessionResponse::parse(&out.spdus[0]).unwrap();
        assert_eq!(resp.status, SessionStatus::ResourceNonExistent);
        assert!(out.closed.is_empty(), "no session was closed");
    }
}
