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

/// Per-session ad-break records.
///
/// A `BTreeMap` keyed by session id: the records here are small (one
/// [`BreakState`] per session, per the module's design decision), so there
/// is no per-peer *media* state to bound with a cursor count the way
/// `media-plane`'s `Trunk` must. Eviction policy (TTL, LRU, session-end) is
/// the caller's concern — this store only exposes [`Self::end`] as the
/// mechanism; deciding *when* to call it is out of scope for a sans-IO core
/// with no clock of its own.
#[derive(Debug, Clone, Default)]
pub struct SessionStore {
    sessions: BTreeMap<String, BreakState>,
}

impl SessionStore {
    /// An empty store.
    pub fn new() -> Self {
        SessionStore {
            sessions: BTreeMap::new(),
        }
    }

    /// Start a break for `session_id`. Errors with
    /// [`Error::BreakAlreadyActive`] if the session already has an
    /// unresumed break — call [`Self::resume`] or [`Self::end`] first.
    pub fn begin_break(&mut self, session_id: impl Into<String>, state: BreakState) -> Result<()> {
        let session_id = session_id.into();
        if let Some(existing) = self.sessions.get(&session_id)
            && !existing.is_resumed()
        {
            return Err(Error::BreakAlreadyActive(session_id));
        }
        self.sessions.insert(session_id, state);
        Ok(())
    }

    /// Record the resumption point for `session_id`'s active break. Errors
    /// with [`Error::NoActiveBreak`] if there is no record for the session.
    pub fn resume(&mut self, session_id: &str, resumed_at: ConditionedSplicePoint) -> Result<()> {
        let state = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| Error::NoActiveBreak(session_id.into()))?;
        state.resumed_at = Some(resumed_at);
        Ok(())
    }

    /// The current break record for `session_id`, if any. A resumed record
    /// stays until [`Self::end`] evicts it, so a caller can still render the
    /// just-ended break's `EXT-X-DATERANGE` for one more playlist reload.
    pub fn get(&self, session_id: &str) -> Option<&BreakState> {
        self.sessions.get(session_id)
    }

    /// Evict `session_id`'s record entirely (session ended, or its break has
    /// aged out of the sliding window). Returns the evicted record, if any.
    pub fn end(&mut self, session_id: &str) -> Option<BreakState> {
        self.sessions.remove(session_id)
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
}
