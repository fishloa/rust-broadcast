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

use dvb_ci::resource::ResourceId;
use dvb_ci::spdu::{
    tags, CloseSessionRequest, CloseSessionResponse, CreateSession, CreateSessionResponse,
    OpenSessionRequest, OpenSessionResponse, SessionNumber, SessionStatus,
};
use dvb_common::{Parse, Serialize};

fn ser<S: Serialize>(s: &S) -> Vec<u8> {
    let mut b = vec![0u8; s.serialized_len()];
    // The buffer is sized to `serialized_len()`, so serialization cannot fail;
    // matched (not `expect`ed) to avoid a `Debug` bound on `S::Error`.
    match s.serialize_into(&mut b) {
        Ok(n) => b.truncate(n),
        Err(_) => b.clear(),
    }
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

    fn alloc(&mut self) -> u16 {
        let nb = self.next;
        self.next = self.next.checked_add(1).filter(|&n| n != 0).unwrap_or(1);
        nb
    }

    /// Open a session to a **module-provided** resource (host-initiated):
    /// returns the `create_session` SPDU to send. The session is recorded once
    /// the module's `create_session_response(ok)` arrives.
    pub fn create_session(&mut self, resource: ResourceId) -> Vec<u8> {
        let session_nb = self.alloc();
        ser(&CreateSession {
            resource,
            session_nb,
        })
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

    /// Handle one inbound SPDU. `provides` answers "does the host provide this
    /// resource?" for an incoming `open_session_request`.
    pub fn on_spdu(&mut self, spdu: &[u8], provides: impl Fn(ResourceId) -> bool) -> SessionOut {
        let mut out = SessionOut::default();
        match spdu.first().copied() {
            // Module wants a host-provided resource.
            Some(tags::OPEN_SESSION_REQUEST) => {
                if let Ok(req) = OpenSessionRequest::parse(spdu) {
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
            }
            // Module's reply to our create_session.
            Some(tags::CREATE_SESSION_RESPONSE) => {
                if let Ok(resp) = CreateSessionResponse::parse(spdu) {
                    if resp.status == SessionStatus::Ok {
                        self.sessions.insert(resp.session_nb, resp.resource);
                        out.opened.push((resp.session_nb, resp.resource));
                    }
                }
            }
            // Peer closes a session.
            Some(tags::CLOSE_SESSION_REQUEST) => {
                if let Ok(req) = CloseSessionRequest::parse(spdu) {
                    self.sessions.remove(&req.session_nb);
                    out.spdus.push(ser(&CloseSessionResponse {
                        status: SessionStatus::Ok,
                        session_nb: req.session_nb,
                    }));
                    out.closed.push(req.session_nb);
                }
            }
            // Ack of a close we initiated.
            Some(tags::CLOSE_SESSION_RESPONSE) => {
                if let Ok(resp) = CloseSessionResponse::parse(spdu) {
                    self.sessions.remove(&resp.session_nb);
                    out.closed.push(resp.session_nb);
                }
            }
            // Data: session_number(nb) + APDU body.
            Some(tags::SESSION_NUMBER) => {
                if let Ok(sn) = SessionNumber::parse(spdu) {
                    if spdu.len() > SessionNumber::HEADER_LEN {
                        out.apdus
                            .push((sn.session_nb, spdu[SessionNumber::HEADER_LEN..].to_vec()));
                    }
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
