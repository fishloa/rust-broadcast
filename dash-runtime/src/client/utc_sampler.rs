//! The `UTCTiming` sampler: walks the MPD's alternatives in order, issues the
//! HTTP requests, and turns the first usable answer into a clock sample
//! (ISO/IEC 23009-1 5.8.5.7 Table 35; DASH-IF IOP v4.3 4.7; DESIGN 3.3).
//!
//! Pure bookkeeping. The engine owns the clock and the ids; this type only
//! decides which candidate is next and what an answer is worth.

use broadcast_common::stage::Timestamp;

use super::action::{Action, UtcId, UtcMethod};
use super::config::DashConfig;
use super::error::Failure;
use super::meta::UtcResponse;
use crate::timing::utc_timing::{
    Exchange, UtcSample, UtcScheme, sample_from_body, sample_from_date_header, sample_from_direct,
};
use broadcast_dash::UtcTiming;
use url::Url;

const STATUS_METHOD_NOT_ALLOWED: u16 = 405;
const STATUS_NOT_IMPLEMENTED: u16 = 501;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Cand {
    Direct(String),
    Http { scheme: UtcScheme, url: Url },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Inflight {
    id: UtcId,
    sent: Timestamp,
    deadline: Option<Timestamp>,
    method: UtcMethod,
    rtt_retried: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Reissue {
    method: UtcMethod,
    rtt_retried: bool,
}

/// What the sampler wants next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    Fetch(Action),
    Sample(UtcSample),
    Idle,
    Exhausted,
}

/// Walks the candidates of one sampling round.
#[derive(Debug, Clone)]
pub(crate) struct UtcSampler {
    cands: Vec<Cand>,
    idx: usize,
    inflight: Option<Inflight>,
    reissue: Option<Reissue>,
    mpd_exchange: Exchange,
    pub(crate) gating: bool,
    pub(crate) wait_deadline: Option<Timestamp>,
    finished: bool,
}

/// The candidates an MPD offers, in the order they are tried: `direct` and
/// the HTTP schemes in document order, `http-head` last (IOP 4.7.2: a cache
/// with an unsynchronised clock makes `Date` the weakest source).
fn ordered(elements: &[UtcTiming], accept: &mut dyn FnMut(&str) -> Option<Url>) -> Vec<Cand> {
    let mut first = Vec::new();
    let mut last = Vec::new();
    for e in elements {
        match e.scheme {
            UtcScheme::Direct => first.push(Cand::Direct(e.value.clone())),
            UtcScheme::HttpHead => {
                for v in e.values() {
                    if let Some(url) = accept(v) {
                        last.push(Cand::Http {
                            scheme: e.scheme,
                            url,
                        });
                    }
                }
            }
            UtcScheme::HttpXsdate | UtcScheme::HttpIso | UtcScheme::HttpNtp => {
                for v in e.values() {
                    if let Some(url) = accept(v) {
                        first.push(Cand::Http {
                            scheme: e.scheme,
                            url,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    first.extend(last);
    first
}

impl UtcSampler {
    /// A sampler over `elements`. `accept` maps an `@value` URL to the
    /// absolute URL to fetch, or `None` to skip it.
    pub(crate) fn new(
        elements: &[UtcTiming],
        accept: &mut dyn FnMut(&str) -> Option<Url>,
        mpd_exchange: Exchange,
    ) -> Self {
        Self {
            cands: ordered(elements, accept),
            idx: 0,
            inflight: None,
            reissue: None,
            mpd_exchange,
            gating: false,
            wait_deadline: None,
            finished: false,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.cands.is_empty()
    }

    /// The instant the in-flight request times out; `None` (a deadline past
    /// the end of the clock) never fires.
    pub(crate) fn deadline(&self) -> Option<Timestamp> {
        self.inflight.and_then(|i| i.deadline)
    }

    fn advance(&mut self) {
        self.inflight = None;
        self.reissue = None;
        self.idx += 1;
    }

    pub(crate) fn poll(&mut self, now: Timestamp, next_id: &mut u32, cfg: &DashConfig) -> Step {
        if self.finished {
            return Step::Exhausted;
        }
        if let Some(i) = self.inflight {
            if i.deadline.is_none_or(|d| now < d) {
                return Step::Idle;
            }
            self.advance();
        }
        loop {
            let Some(cand) = self.cands.get(self.idx) else {
                self.finished = true;
                return Step::Exhausted;
            };
            match cand {
                Cand::Direct(value) => {
                    let r = sample_from_direct(value, self.mpd_exchange, cfg.utc_max_rtt);
                    match r {
                        Ok(sample) => {
                            self.finished = true;
                            return Step::Sample(sample);
                        }
                        Err(_) => {
                            self.advance();
                        }
                    }
                }
                Cand::Http { scheme, url } => {
                    let re = self.reissue.take();
                    let method = re.map_or_else(
                        || {
                            if *scheme == UtcScheme::HttpHead {
                                UtcMethod::Head
                            } else {
                                UtcMethod::Get
                            }
                        },
                        |r| r.method,
                    );
                    let id = UtcId(*next_id);
                    *next_id = next_id.wrapping_add(1);
                    self.inflight = Some(Inflight {
                        id,
                        sent: now,
                        deadline: now.checked_add(cfg.utc_timeout),
                        method,
                        rtt_retried: re.is_some_and(|r| r.rtt_retried),
                    });
                    return Step::Fetch(Action::FetchUtcTiming {
                        id,
                        scheme: *scheme,
                        url: url.clone(),
                        method,
                    });
                }
            }
        }
    }

    /// Whether `id` is the request being waited for.
    pub(crate) fn awaits(&self, id: UtcId) -> bool {
        self.inflight.is_some_and(|i| i.id == id)
    }

    /// An answer. `Some` is a sample (the round is over); `None` means the
    /// answer was not usable and the next candidate (or a re-issue) is up.
    pub(crate) fn on_response(
        &mut self,
        id: UtcId,
        resp: &UtcResponse<'_>,
        cfg: &DashConfig,
    ) -> Option<UtcSample> {
        let inflight = self.inflight.filter(|i| i.id == id)?;
        let Some(Cand::Http { scheme, .. }) = self.cands.get(self.idx) else {
            return None;
        };
        let scheme = *scheme;
        let exchange = Exchange::new(resp.sent, resp.received);
        let result = match (scheme, inflight.method) {
            (UtcScheme::HttpHead, _) => resp.date.map_or_else(
                || Err(crate::timing::TimingError::ExchangeReversed),
                |d| sample_from_date_header(d, exchange, cfg.utc_max_rtt),
            ),
            _ => sample_from_body(scheme, resp.body, exchange, cfg.utc_max_rtt),
        };
        match result {
            Ok(sample) => {
                self.inflight = None;
                self.finished = true;
                Some(sample)
            }
            Err(crate::timing::TimingError::RttTooLarge { .. }) if !inflight.rtt_retried => {
                self.inflight = None;
                self.reissue = Some(Reissue {
                    method: inflight.method,
                    rtt_retried: true,
                });
                None
            }
            Err(_) => {
                self.advance();
                None
            }
        }
    }

    /// A failed request.
    pub(crate) fn on_failure(&mut self, id: UtcId, failure: Failure) {
        let Some(inflight) = self.inflight.filter(|i| i.id == id) else {
            return;
        };
        let head_refused = inflight.method == UtcMethod::Head
            && matches!(
                failure,
                Failure::Status(STATUS_METHOD_NOT_ALLOWED | STATUS_NOT_IMPLEMENTED)
            );
        if head_refused {
            self.inflight = None;
            self.reissue = Some(Reissue {
                method: UtcMethod::Get,
                rtt_retried: inflight.rtt_retried,
            });
        } else {
            self.advance();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Step, UtcSampler};
    use crate::client::action::{Action, UtcId, UtcMethod};
    use crate::client::config::DashConfig;
    use crate::client::error::Failure;
    use crate::timing::utc_timing::{Exchange, UtcScheme};
    use broadcast_common::stage::Timestamp;
    use broadcast_dash::UtcTiming;
    use url::Url;

    const XSDATE: &str = "urn:mpeg:dash:utc:http-xsdate:2014";
    const HTTP_HEAD: &str = "urn:mpeg:dash:utc:http-head:2014";

    fn now() -> Timestamp {
        Timestamp::from_micros(1_000_000)
    }

    fn timing(scheme_id_uri: &str, value: &str) -> UtcTiming {
        let mut t = UtcTiming::default();
        t.scheme = UtcScheme::from_scheme_id_uri(scheme_id_uri);
        t.scheme_id_uri = scheme_id_uri.to_owned();
        t.value = value.to_owned();
        t
    }

    fn accept(v: &str) -> Option<Url> {
        Url::parse(v).ok()
    }

    fn exchange() -> Exchange {
        Exchange::new(now(), now())
    }

    fn fetch_id(s: &mut UtcSampler, next_id: &mut u32) -> (UtcId, UtcMethod) {
        match s.poll(now(), next_id, &DashConfig::default()) {
            Step::Fetch(Action::FetchUtcTiming { id, method, .. }) => (id, method),
            other => panic!("no fetch: {other:?}"),
        }
    }

    #[test]
    fn a_sampler_with_no_candidate_is_empty_and_awaits_nothing() {
        let s = UtcSampler::new(&[], &mut accept, exchange());
        assert!(s.is_empty());
        assert!(!s.awaits(UtcId(0)));
    }

    #[test]
    fn only_the_in_flight_id_is_awaited() {
        let elems = [timing(XSDATE, "http://utc.test/x")];
        let mut s = UtcSampler::new(&elems, &mut accept, exchange());
        assert!(!s.is_empty());
        let mut next = 0u32;
        let (id, method) = fetch_id(&mut s, &mut next);
        assert_eq!(id.get(), 0);
        assert_eq!(method, UtcMethod::Get);
        assert!(s.awaits(UtcId(0)));
        assert!(!s.awaits(UtcId(1)));
    }

    #[test]
    fn a_refused_head_is_reissued_as_a_get() {
        let elems = [timing(HTTP_HEAD, "http://utc.test/head")];
        let mut s = UtcSampler::new(&elems, &mut accept, exchange());
        let mut next = 0u32;
        let (id, method) = fetch_id(&mut s, &mut next);
        assert_eq!(method, UtcMethod::Head);
        s.on_failure(id, Failure::Status(405));
        let (_, method) = fetch_id(&mut s, &mut next);
        assert_eq!(method, UtcMethod::Get);
    }

    #[test]
    fn a_head_that_fails_for_another_reason_moves_on() {
        let elems = [timing(HTTP_HEAD, "http://utc.test/head")];
        let mut s = UtcSampler::new(&elems, &mut accept, exchange());
        let mut next = 0u32;
        let (id, method) = fetch_id(&mut s, &mut next);
        assert_eq!(method, UtcMethod::Head);
        s.on_failure(id, Failure::Status(500));
        assert_eq!(
            s.poll(now(), &mut next, &DashConfig::default()),
            Step::Exhausted
        );
    }
}
