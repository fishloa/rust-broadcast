//! Per-session ad-break state.
//!
//! Issue #929's design decision: the per-session record is session id ->
//! ad-break decision -> timeline offsets, and it is deliberately **not** a
//! media cursor. Every viewer in a break watches one of a small set of ad
//! assets, and outside a break every viewer sees byte-identical primary
//! content; what differs per session is which URIs appear in that session's
//! rendered playlist. `media-plane`'s "writer cost is O(N) in cursor count"
//! rule is about the shared media rings — a `SessionStore` holds no cursor
//! into any of them, just this one small [`BreakState`] record per session.

use crate::decision::AdBreakDecision;
use crate::error::{Error, Result};
use crate::splice::ConditionedSplicePoint;
use alloc::collections::BTreeMap;
use alloc::string::String;
use broadcast_common::stage::Timestamp;
use core::time::Duration;

/// One session's ad break: the decision being rendered, plus the conditioned
/// splice points it entered (and, once known, resumed) at.
// `AdBreakDecision` is `PartialEq`-only (carries `f64` fields), so this is too.
#[derive(Debug, Clone, PartialEq)]
pub struct BreakState {
    /// The decision being rendered for this break.
    pub decision: AdBreakDecision,
    /// The conditioned splice point the break was entered at.
    pub entered_at: ConditionedSplicePoint,
    /// The conditioned splice point playback resumed at, once known.
    pub resumed_at: Option<ConditionedSplicePoint>,
}

impl BreakState {
    /// Start a new break with no resumption point yet known.
    pub fn new(decision: AdBreakDecision, entered_at: ConditionedSplicePoint) -> Self {
        BreakState {
            decision,
            entered_at,
            resumed_at: None,
        }
    }

    /// Whether [`Self::resumed_at`] has been recorded (the break has ended).
    pub fn is_resumed(&self) -> bool {
        self.resumed_at.is_some()
    }
}

/// The default bound on tracked sessions (see [`SessionStore::with_max_sessions`]).
pub const DEFAULT_MAX_SESSIONS: usize = 65_536;

/// Per-session ad-break records.
///
/// A `BTreeMap` keyed by session id: the records here are small (one
/// [`BreakState`] per session, per the module's design decision), so there
/// is no per-peer *media* state to bound with a cursor count the way
/// `media-plane`'s `Trunk` must. The store is nonetheless bounded: it holds at
/// most [`DEFAULT_MAX_SESSIONS`] sessions (or the bound set by
/// [`Self::with_max_sessions`]), and [`Self::begin_break`] for a new session id
/// past that is [`Error::SessionStoreFull`], so a flood of distinct ids cannot
/// grow it forever. This crate has no clock of its own: the caller feeds the
/// time in with [`Self::advance`] and calls [`Self::expire_idle`] to drop
/// sessions nobody has touched for a while; [`Self::end`] evicts one directly.
#[derive(Debug, Clone)]
pub struct SessionStore {
    sessions: BTreeMap<String, Tracked>,
    max_sessions: usize,
    now_us: u64,
}

#[derive(Debug, Clone)]
struct Tracked {
    state: BreakState,
    touched_us: u64,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionStore {
    /// An empty store bounded at [`DEFAULT_MAX_SESSIONS`].
    pub fn new() -> Self {
        SessionStore {
            sessions: BTreeMap::new(),
            max_sessions: DEFAULT_MAX_SESSIONS,
            now_us: 0,
        }
    }

    /// Bounds the store at `max_sessions` tracked sessions (at least 1).
    pub fn with_max_sessions(mut self, max_sessions: usize) -> Self {
        self.max_sessions = max_sessions.max(1);
        self
    }

    /// Moves the store's clock to `now` (the caller's own monotonic clock, the
    /// `broadcast_common::stage::Timestamp` every sans-IO engine takes; a value
    /// earlier than the current one is ignored). Records touched from now on
    /// carry this time.
    pub fn advance(&mut self, now: Timestamp) {
        self.now_us = self.now_us.max(now.as_micros());
    }

    /// Evicts every session last touched (by [`Self::begin_break`] or
    /// [`Self::resume`]) more than `ttl` before the store's clock; returns
    /// how many were evicted.
    pub fn expire_idle(&mut self, ttl: Duration) -> usize {
        let ttl_us = u64::try_from(ttl.as_micros()).unwrap_or(u64::MAX);
        let before = self.sessions.len();
        let now = self.now_us;
        self.sessions
            .retain(|_, t| now.saturating_sub(t.touched_us) <= ttl_us);
        before - self.sessions.len()
    }

    /// Start a break for `session_id`. Errors with
    /// [`Error::BreakAlreadyActive`] if the session already has an
    /// unresumed break — call [`Self::resume`] or [`Self::end`] first — and
    /// with [`Error::SessionStoreFull`] if `session_id` is new and the store
    /// is at its bound.
    pub fn begin_break(&mut self, session_id: impl Into<String>, state: BreakState) -> Result<()> {
        let session_id = session_id.into();
        match self.sessions.get(&session_id) {
            Some(existing) if !existing.state.is_resumed() => {
                return Err(Error::BreakAlreadyActive(session_id));
            }
            None if self.sessions.len() >= self.max_sessions => {
                return Err(Error::SessionStoreFull {
                    max: self.max_sessions,
                });
            }
            _ => {}
        }
        self.sessions.insert(
            session_id,
            Tracked {
                state,
                touched_us: self.now_us,
            },
        );
        Ok(())
    }

    /// Record the resumption point for `session_id`'s active break. Errors
    /// with [`Error::NoActiveBreak`] if there is no record for the session and
    /// with [`Error::BreakAlreadyResumed`] if its break was already resumed
    /// (the first resumption point is kept).
    pub fn resume(&mut self, session_id: &str, resumed_at: ConditionedSplicePoint) -> Result<()> {
        let tracked = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| Error::NoActiveBreak(session_id.into()))?;
        if tracked.state.is_resumed() {
            return Err(Error::BreakAlreadyResumed(session_id.into()));
        }
        tracked.state.resumed_at = Some(resumed_at);
        tracked.touched_us = self.now_us;
        Ok(())
    }

    /// The current break record for `session_id`, if any. A resumed record
    /// stays until [`Self::end`] evicts it, so a caller can still render the
    /// just-ended break's `EXT-X-DATERANGE` for one more playlist reload.
    pub fn get(&self, session_id: &str) -> Option<&BreakState> {
        self.sessions.get(session_id).map(|t| &t.state)
    }

    /// Evict `session_id`'s record entirely (session ended, or its break has
    /// aged out of the sliding window). Returns the evicted record, if any.
    pub fn end(&mut self, session_id: &str) -> Option<BreakState> {
        self.sessions.remove(session_id).map(|t| t.state)
    }

    /// Number of tracked sessions.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::splice::condition_splice_point;

    fn splice_point() -> ConditionedSplicePoint {
        condition_splice_point(1_000, &[1_000], 0).unwrap()
    }

    #[test]
    fn begin_get_resume_end_lifecycle() {
        let mut store = SessionStore::new();
        assert!(store.is_empty());

        let decision = AdBreakDecision::single_asset("b1", "https://x/a.m3u8");
        store
            .begin_break("s1", BreakState::new(decision.clone(), splice_point()))
            .unwrap();
        assert_eq!(store.len(), 1);
        assert!(!store.get("s1").unwrap().is_resumed());

        store.resume("s1", splice_point()).unwrap();
        assert!(store.get("s1").unwrap().is_resumed());

        let evicted = store.end("s1").unwrap();
        assert_eq!(evicted.decision, decision);
        assert!(store.is_empty());
    }

    #[test]
    fn cannot_begin_a_break_while_one_is_already_active() {
        let mut store = SessionStore::new();
        let decision = AdBreakDecision::single_asset("b1", "https://x/a.m3u8");
        store
            .begin_break("s1", BreakState::new(decision.clone(), splice_point()))
            .unwrap();

        let err = store
            .begin_break("s1", BreakState::new(decision, splice_point()))
            .unwrap_err();
        assert!(matches!(err, Error::BreakAlreadyActive(ref s) if s == "s1"));
    }

    #[test]
    fn begin_break_is_allowed_again_after_resume() {
        let mut store = SessionStore::new();
        let decision = AdBreakDecision::single_asset("b1", "https://x/a.m3u8");
        store
            .begin_break("s1", BreakState::new(decision.clone(), splice_point()))
            .unwrap();
        store.resume("s1", splice_point()).unwrap();

        // A second, later break for the same session is fine once resumed.
        store
            .begin_break("s1", BreakState::new(decision, splice_point()))
            .unwrap();
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn resume_without_an_active_break_errors() {
        let mut store = SessionStore::new();
        let err = store.resume("nope", splice_point()).unwrap_err();
        assert!(matches!(err, Error::NoActiveBreak(ref s) if s == "nope"));
    }

    #[test]
    fn resume_on_an_already_resumed_break_keeps_the_first_point() {
        let mut store = SessionStore::new();
        let decision = AdBreakDecision::single_asset("b1", "https://x/a.m3u8");
        store
            .begin_break("s1", BreakState::new(decision, splice_point()))
            .unwrap();
        let first = condition_splice_point(2_000, &[2_000], 0).unwrap();
        store.resume("s1", first).unwrap();
        let err = store.resume("s1", splice_point()).unwrap_err();
        assert!(matches!(err, Error::BreakAlreadyResumed(ref s) if s == "s1"));
        assert_eq!(store.get("s1").unwrap().resumed_at, Some(first));
    }

    #[test]
    fn the_store_is_bounded_and_existing_sessions_still_begin_breaks() {
        let mut store = SessionStore::new().with_max_sessions(2);
        let decision = AdBreakDecision::single_asset("b1", "https://x/a.m3u8");
        for id in ["a", "b"] {
            store
                .begin_break(id, BreakState::new(decision.clone(), splice_point()))
                .unwrap();
        }
        let err = store
            .begin_break("c", BreakState::new(decision.clone(), splice_point()))
            .unwrap_err();
        assert!(matches!(err, Error::SessionStoreFull { max: 2 }));
        store.resume("a", splice_point()).unwrap();
        store
            .begin_break("a", BreakState::new(decision, splice_point()))
            .unwrap();
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn expire_idle_drops_only_sessions_untouched_for_longer_than_the_ttl() {
        let mut store = SessionStore::new();
        let decision = AdBreakDecision::single_asset("b1", "https://x/a.m3u8");
        store.advance(Timestamp::from_micros(1_000));
        store
            .begin_break("old", BreakState::new(decision.clone(), splice_point()))
            .unwrap();
        store.advance(Timestamp::from_micros(5_000));
        store
            .begin_break("new", BreakState::new(decision, splice_point()))
            .unwrap();
        store.advance(Timestamp::from_micros(6_000));
        assert_eq!(store.expire_idle(Duration::from_micros(2_000)), 1);
        assert!(store.get("old").is_none());
        assert!(store.get("new").is_some());
        store.advance(Timestamp::from_micros(1));
        assert_eq!(
            store.expire_idle(Duration::from_micros(1_000)),
            0,
            "the clock never goes backwards"
        );
    }
}
