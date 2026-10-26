//! The `UTCTiming` sampler: walks the MPD's alternatives in order, issues the
//! HTTP requests, and turns the first usable answer into a clock sample
//! (ISO/IEC 23009-1 5.8.5.7 Table 35; DASH-IF IOP v4.3 4.7; DESIGN 3.3).
//!
//! Pure bookkeeping. The engine owns the clock and the ids; this type only
//! decides which candidate is next and what an answer is worth.

use super::action::{Action, UtcId, UtcMethod};
use super::config::DashConfig;
use super::error::Failure;
use super::meta::UtcResponse;
use crate::mpd::model::UtcTiming;
use crate::timing::utc_timing::{
    Exchange, UtcSample, UtcScheme, sample_from_body, sample_from_date_header, sample_from_direct,
};

const STATUS_METHOD_NOT_ALLOWED: u16 = 405;
const STATUS_NOT_IMPLEMENTED: u16 = 501;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Cand {
    Direct(String),
    Http { scheme: UtcScheme, url: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Inflight {
    id: UtcId,
    sent_us: u64,
    deadline_us: u64,
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
    pub(crate) wait_deadline_us: Option<u64>,
    finished: bool,
}

/// The candidates an MPD offers, in the order they are tried: `direct` and
/// the HTTP schemes in document order, `http-head` last (IOP 4.7.2: a cache
/// with an unsynchronised clock makes `Date` the weakest source).
fn ordered(elements: &[UtcTiming], accept: &mut dyn FnMut(&str) -> Option<String>) -> Vec<Cand> {
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
            UtcScheme::Ntp | UtcScheme::Sntp | UtcScheme::Unknown => {}
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
        accept: &mut dyn FnMut(&str) -> Option<String>,
        mpd_exchange: Exchange,
    ) -> Self {
        Self {
            cands: ordered(elements, accept),
            idx: 0,
            inflight: None,
            reissue: None,
            mpd_exchange,
            gating: false,
            wait_deadline_us: None,
            finished: false,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.cands.is_empty()
    }

    /// The instant the in-flight request times out.
    pub(crate) fn deadline_us(&self) -> Option<u64> {
        self.inflight.map(|i| i.deadline_us)
    }

    fn advance(&mut self) {
        self.inflight = None;
        self.reissue = None;
        self.idx += 1;
    }

    pub(crate) fn poll(&mut self, now_us: u64, next_id: &mut u32, cfg: &DashConfig) -> Step {
        if self.finished {
            return Step::Exhausted;
        }
        if let Some(i) = self.inflight {
            if now_us < i.deadline_us {
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
                    let r = sample_from_direct(value, self.mpd_exchange, cfg.utc_max_rtt_us);
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
                        sent_us: now_us,
                        deadline_us: now_us.saturating_add(cfg.utc_timeout_us),
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
        let exchange = Exchange::new(resp.sent_us, resp.received_us);
        let result = match (scheme, inflight.method) {
            (UtcScheme::HttpHead, _) => resp.date.map_or_else(
                || Err(crate::timing::TimingError::ExchangeReversed),
                |d| sample_from_date_header(d, exchange, cfg.utc_max_rtt_us),
            ),
            _ => sample_from_body(scheme, resp.body, exchange, cfg.utc_max_rtt_us),
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
