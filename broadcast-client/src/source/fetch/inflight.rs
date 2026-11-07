//! The in-flight request table shared by the HLS and DASH adapters: request records, per-class
//! timeouts, deadline and inactivity bookkeeping, and the connection-lease dance of sending one.

use alloc::collections::VecDeque;
use alloc::string::ToString;
use alloc::vec::Vec;
use std::collections::HashMap;
use url::Url;

use crate::events::Action;
use crate::source::fetch::body::BodyAcc;
use crate::source::fetch::conn::{ConnAlloc, ConnLease};
use crate::source::fetch::deadlines::Deadlines;
use crate::source::fetch::inactivity::{Expired, Inactivity};
use crate::source::fetch::resume::Basis;

#[derive(Debug, Clone, Copy)]
pub(crate) enum TimeoutClass {
    Manifest,
    Segment,
    Aux,
}

pub(crate) trait PurposeKind: Copy {
    fn is_media(self) -> bool;
    fn timeout_class(self) -> TimeoutClass;
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
    pub(crate) byte_range: Option<(u64, u64)>,
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
        byte_range: Option<(u64, u64)>,
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
    pub(crate) reqs: HashMap<u32, Req<P, X>>,
    pub(crate) deadlines: Deadlines<u32>,
    inactivity: Inactivity,
    conn_alloc: ConnAlloc,
    pub(crate) timeouts: Timeouts,
}

impl<P: PurposeKind, X> InFlight<P, X> {
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
            method: method.to_string(),
            url: req.url.to_string(),
            headers,
            byte_range: req.byte_range.filter(|_| resume.is_none()),
            identity: None,
        });
        actions.push_back(Action::WakeAt(now.saturating_add(timeout)));
        let deadline = now.checked_add(timeout);
        if req.purpose.is_media()
            && let Some(at) = self.inactivity.arm(conn, now)
        {
            actions.push_back(Action::WakeAt(at));
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

    pub(crate) fn overdue(&mut self, now: u64) -> Vec<u32> {
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
}
