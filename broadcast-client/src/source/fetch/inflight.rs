//! The in-flight request table shared by the HLS and DASH adapters: request records, per-class
//! timeouts, deadline and inactivity bookkeeping, and the connection-lease dance of sending one.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::string::ToString;
use alloc::vec::Vec;
use std::collections::HashMap;
use url::Url;

use crate::ConnId;
use crate::events::{Action, ErrorKind, Event, ResumeMode, Status};
use crate::redirect;
use crate::source::fetch::body::BodyAcc;
use crate::source::fetch::conn::{ConnAlloc, ConnLease};
use crate::source::fetch::deadlines::Deadlines;
use crate::source::fetch::inactivity::{Expired, Inactivity};
use crate::source::fetch::resume::{Basis, Parked, Stash};
use crate::source::fetch::retry::RefusedCapture;
use broadcast_auth::{AuthDecision, AuthState};
use broadcast_common::stage::Timestamp;
use http::{HeaderMap, StatusCode};

#[derive(Debug, Clone, Copy)]
pub(crate) enum TimeoutClass {
    Manifest,
    Segment,
    Aux,
}

pub(crate) trait PurposeKind: Copy {
    fn is_media(self) -> bool;
    fn timeout_class(self) -> TimeoutClass;
    fn oversize_field(self) -> Option<&'static str>;
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Timeouts {
    pub(crate) manifest_us: u64,
    pub(crate) segment_us: u64,
    pub(crate) aux_us: u64,
}

impl Timeouts {
    fn of(&self, class: TimeoutClass) -> u64 {
        match class {
            TimeoutClass::Manifest => self.manifest_us,
            TimeoutClass::Segment => self.segment_us,
            TimeoutClass::Aux => self.aux_us,
        }
    }
}

pub(crate) struct Req<P, X> {
    pub(crate) purpose: P,
    pub(crate) original_url: alloc::string::String,
    pub(crate) url: Url,
    pub(crate) byte_range: Option<broadcast_common::range::ByteSpan>,
    pub(crate) redirects: u32,
    pub(crate) authed: bool,
    pub(crate) stale_replayed: bool,
    pub(crate) lease: Option<ConnLease>,
    pub(crate) started_us: u64,
    pub(crate) last_activity_us: u64,
    pub(crate) responded_ok: bool,
    pub(crate) basis: Option<Basis>,
    pub(crate) resume_from: Option<u64>,
    pub(crate) body: BodyAcc,
    pub(crate) extra: X,
}

impl<P: PurposeKind, X> Req<P, X> {
    pub(crate) fn new(
        purpose: P,
        url: Url,
        byte_range: Option<broadcast_common::range::ByteSpan>,
        body_limit: usize,
        extra: X,
    ) -> Self {
        Self {
            purpose,
            original_url: url.to_string(),
            url,
            byte_range,
            redirects: 0,
            authed: false,
            stale_replayed: false,
            lease: None,
            started_us: 0,
            last_activity_us: 0,
            responded_ok: false,
            basis: None,
            resume_from: None,
            body: BodyAcc::new(body_limit),
            extra,
        }
    }

    fn deadline(&self, timeouts: &Timeouts) -> Option<u64> {
        self.started_us
            .checked_add(timeouts.of(self.purpose.timeout_class()))
    }
}

pub(crate) struct InFlight<P, X> {
    pub(crate) reqs: HashMap<ConnId, Req<P, X>>,
    pub(crate) deadlines: Deadlines<ConnId>,
    inactivity: Inactivity,
    conn_alloc: ConnAlloc,
    pub(crate) timeouts: Timeouts,
}

impl<P: PurposeKind + PartialEq, X> InFlight<P, X> {
    pub(crate) fn new(timeouts: Timeouts, inactivity_us: u64, conn_alloc: ConnAlloc) -> Self {
        Self {
            reqs: HashMap::new(),
            deadlines: Deadlines::new(),
            inactivity: Inactivity::new(inactivity_us),
            conn_alloc,
            timeouts,
        }
    }

    pub(crate) fn start(
        &mut self,
        mut req: Req<P, X>,
        mut headers: Vec<(alloc::string::String, alloc::string::String)>,
        method: &str,
        now: u64,
        actions: &mut VecDeque<Action>,
    ) {
        req.started_us = now;
        req.last_activity_us = now;
        req.responded_ok = false;
        let resume = req
            .resume_from
            .zip(req.basis.as_ref())
            .and_then(|(received, basis)| basis.request_headers(received));
        match &resume {
            Some(resume) => headers.extend(resume.iter().cloned()),
            None => {
                req.resume_from = None;
                req.basis = None;
                req.body.reset();
            }
        }
        let lease = self.conn_alloc.lease();
        let conn = lease.id();
        req.lease = Some(lease);
        let timeout = self.timeouts.of(req.purpose.timeout_class());
        actions.push_back(Action::HttpRequest {
            conn,
            method: http::Method::from_bytes(method.as_bytes()).unwrap_or(http::Method::GET),
            url: req.url.clone(),
            headers: crate::headers::to_header_map(&headers),
            byte_range: req.byte_range.filter(|_| resume.is_none()),
            identity: None,
        });
        let deadline = now.checked_add(timeout);
        if let Some(deadline) = deadline {
            actions.push_back(Action::WakeAt(Timestamp::from_micros(deadline)));
        }
        if req.purpose.is_media()
            && let Some(at) = self.inactivity.arm(conn, now)
        {
            actions.push_back(Action::WakeAt(Timestamp::from_micros(at)));
        }
        self.reqs.insert(conn, req);
        if let Some(deadline) = deadline {
            self.deadlines.arm(conn, deadline);
        }
        let timeouts = &self.timeouts;
        self.deadlines.compact(self.reqs.len(), || {
            self.reqs
                .iter()
                .filter_map(|(conn, r)| r.deadline(timeouts).map(|d| (d, *conn)))
        });
        let reqs = &self.reqs;
        self.inactivity.compact(|| {
            reqs.iter()
                .filter(|(_, r)| r.purpose.is_media())
                .map(|(conn, r)| (*conn, r.last_activity_us))
                .collect()
        });
    }

    pub(crate) fn overdue(&mut self, now: u64) -> Vec<ConnId> {
        let reqs = &self.reqs;
        let timeouts = &self.timeouts;
        self.deadlines.expire(now, |conn, deadline| {
            reqs.get(conn)
                .is_some_and(|r| r.deadline(timeouts) == Some(deadline))
        })
    }

    pub(crate) fn inactive(&mut self, now: u64) -> Expired {
        let reqs = &self.reqs;
        self.inactivity
            .expire(now, |conn| reqs.get(&conn).map(|r| r.last_activity_us))
    }

    pub(crate) fn clear(&mut self) {
        self.reqs.clear();
        self.deadlines.clear();
        self.inactivity.clear();
    }

    pub(crate) fn on_data(
        &mut self,
        conn: ConnId,
        bytes: &[u8],
        now: u64,
        refused_captures: &mut BTreeMap<ConnId, RefusedCapture>,
        oversize_reported: &mut BTreeSet<&'static str>,
        out: Outbox<'_>,
    ) -> Option<Req<P, X>> {
        let Outbox { actions, events } = out;
        if let Some(cap) = refused_captures.get_mut(&conn) {
            if cap.push(bytes)
                && let Some(cap) = refused_captures.remove(&conn)
            {
                events.push_back(cap.into_event());
                actions.push_back(Action::Close { conn });
            }
            return None;
        }
        let req = self.reqs.get_mut(&conn)?;
        if !req.responded_ok || bytes.is_empty() {
            return None;
        }
        req.last_activity_us = now;
        let field = req.purpose.oversize_field();
        if req.body.push(bytes).is_ok() {
            return None;
        }
        if let Some(field) = field
            && oversize_reported.insert(field)
        {
            events.push_back(Event::Error {
                kind: ErrorKind::BodyTooLarge { field },
                fatal: false,
            });
        }
        let req = self.reqs.remove(&conn)?;
        actions.push_back(Action::Close { conn });
        Some(req)
    }

    pub(crate) fn stalled_conns(
        &mut self,
        now: u64,
        actions: &mut VecDeque<Action>,
    ) -> Vec<ConnId> {
        let expired = self.inactive(now);
        for at in expired.wakes {
            actions.push_back(Action::WakeAt(Timestamp::from_micros(at)));
        }
        expired.stalled
    }

    pub(crate) fn stall(
        &mut self,
        conn: ConnId,
        stash: &mut Stash<P>,
        out: Outbox<'_>,
    ) -> Option<Req<P, X>> {
        let Outbox { actions, events } = out;
        let mut req = self.reqs.remove(&conn)?;
        let received = u64::try_from(req.body.len()).unwrap_or(u64::MAX);
        let kept = (req.responded_ok || req.resume_from.is_some())
            .then(|| req.basis.take())
            .flatten()
            .filter(|basis| basis.request_headers(received).is_some());
        let resume = if kept.is_some() {
            ResumeMode::Range
        } else {
            ResumeMode::Full
        };
        if let Some(basis) = kept {
            stash.put(
                req.purpose,
                Parked {
                    url: req.original_url.clone(),
                    byte_range: req.byte_range,
                    body: core::mem::replace(&mut req.body, BodyAcc::new(0)),
                    basis,
                },
            );
        }
        events.push_back(Event::Status(Status::RequestStalled { resume }));
        actions.push_back(Action::Close { conn });
        Some(req)
    }
}

pub(crate) const REDIRECT_STATUSES: [StatusCode; 5] = [
    StatusCode::MOVED_PERMANENTLY,
    StatusCode::FOUND,
    StatusCode::SEE_OTHER,
    StatusCode::TEMPORARY_REDIRECT,
    StatusCode::PERMANENT_REDIRECT,
];
pub(crate) const STATUS_UNAUTHORIZED: StatusCode = StatusCode::UNAUTHORIZED;
pub(crate) const STATUS_PROXY_AUTH_REQUIRED: StatusCode = StatusCode::PROXY_AUTHENTICATION_REQUIRED;

pub(crate) enum HeadRoute<P, X> {
    Send(Req<P, X>),
    RedirectsExhausted(Req<P, X>),
    Parked,
    Other(Req<P, X>),
}

pub(crate) struct Outbox<'a> {
    pub(crate) actions: &'a mut VecDeque<Action>,
    pub(crate) events: &'a mut VecDeque<Event>,
}

#[derive(Clone, Copy)]
pub(crate) struct ResponseHead<'a> {
    pub(crate) conn: ConnId,
    pub(crate) status: StatusCode,
    pub(crate) headers: &'a HeaderMap,
}

pub(crate) fn route_head<P: PurposeKind, X>(
    mut req: Req<P, X>,
    head: ResponseHead<'_>,
    max_redirects: u32,
    auth: &mut AuthState,
    out: Outbox<'_>,
    parked: &mut Vec<Req<P, X>>,
) -> HeadRoute<P, X> {
    let ResponseHead {
        conn,
        status,
        headers,
    } = head;
    let Outbox { actions, events } = out;
    if REDIRECT_STATUSES.contains(&status) {
        actions.push_back(Action::Close { conn });
        return match redirect::resolve(&req.url, headers, req.redirects, max_redirects) {
            Some(target) => {
                req.redirects = req.redirects.saturating_add(1);
                req.url = target;
                HeadRoute::Send(req)
            }
            None => HeadRoute::RedirectsExhausted(req),
        };
    }
    if (status == STATUS_UNAUTHORIZED || status == STATUS_PROXY_AUTH_REQUIRED)
        && req.url.origin() == auth.origin().origin()
    {
        actions.push_back(Action::Close { conn });
        return match auth.handle_unauthorized(status, headers, req.authed, req.stale_replayed) {
            AuthDecision::Replay => HeadRoute::Send(req),
            AuthDecision::ReplayStale => {
                req.stale_replayed = true;
                HeadRoute::Send(req)
            }
            AuthDecision::Wait(wait) => {
                events.extend(wait.into_iter().map(Event::from));
                parked.push(req);
                HeadRoute::Parked
            }
            _ => HeadRoute::Other(req),
        };
    }
    HeadRoute::Other(req)
}

pub(crate) fn record_refusal(
    head: ResponseHead<'_>,
    retry_after_us: u64,
    refused_captures: &mut BTreeMap<ConnId, RefusedCapture>,
    out: Outbox<'_>,
) {
    let ResponseHead {
        conn,
        status,
        headers,
    } = head;
    let Outbox { actions, events } = out;
    let refused =
        status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::SERVICE_UNAVAILABLE;
    if refused {
        let cap = RefusedCapture::new(status, retry_after_us, headers);
        if cap.wants_body() {
            refused_captures.insert(conn, cap);
            return;
        }
        events.push_back(cap.into_event());
    }
    actions.push_back(Action::Close { conn });
}
