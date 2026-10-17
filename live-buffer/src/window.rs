//! Sliding minimum over a trailing time window (monotonic deque).

use crate::error::Error;
use alloc::collections::VecDeque;

/// Monotonic-deque sliding minimum of `(time, value)` samples.
///
/// Amortised O(1) per push. The deque is pre-sized to `max_samples + 1` so no
/// allocation happens after construction; over the cap the oldest entry is
/// dropped (the window shrinks) and counted.
#[derive(Clone, Debug)]
pub struct MinWindow {
    dq: VecDeque<(u64, i64)>,
    window_us: u64,
    cap: usize,
    overflows: u32,
}

impl MinWindow {
    /// New window of `window_us` holding at most `max_samples` entries.
    ///
    /// # Errors
    /// [`Error::ZeroLengthWindow`] when `window_us == 0`.
    pub fn new(window_us: u64, max_samples: u16) -> Result<Self, Error> {
        if window_us == 0 {
            return Err(Error::ZeroLengthWindow);
        }
        let cap = usize::from(max_samples).max(1);
        Ok(Self {
            dq: VecDeque::with_capacity(cap + 1),
            window_us,
            cap,
            overflows: 0,
        })
    }

    /// Add a sample at time `t` (non-decreasing) and return the minimum of
    /// the samples within the trailing window (inclusive of this one).
    pub fn push(&mut self, t: u64, v: i64) -> i64 {
        while self.dq.back().is_some_and(|b| b.1 >= v) {
            self.dq.pop_back();
        }
        self.dq.push_back((t, v));
        while self.dq.len() > 1
            && self
                .dq
                .front()
                .is_some_and(|f| f.0.saturating_add(self.window_us) <= t)
        {
            self.dq.pop_front();
        }
        while self.dq.len() > self.cap {
            self.dq.pop_front();
            self.overflows = self.overflows.saturating_add(1);
        }
        self.dq.front().map_or(v, |f| f.1)
    }

    /// Current minimum, `None` when empty.
    #[must_use]
    pub fn min(&self) -> Option<i64> {
        self.dq.front().map(|f| f.1)
    }

    /// Entries currently held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.dq.len()
    }

    /// True when no sample is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.dq.is_empty()
    }

    /// Entries dropped because the cap was hit.
    #[must_use]
    pub fn overflows(&self) -> u32 {
        self.overflows
    }

    /// Drop every sample (the overflow counter is kept).
    pub fn clear(&mut self) {
        self.dq.clear();
    }
}
