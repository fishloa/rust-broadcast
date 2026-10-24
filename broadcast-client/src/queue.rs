//! Bounded event queue: when full, the oldest queued samples are dropped and counted; every
//! other event is kept.

use alloc::collections::VecDeque;

use crate::events::Event;

pub(crate) struct EventQueue {
    inner: VecDeque<Event>,
    samples: usize,
    cap: usize,
    dropped: u64,
}

impl EventQueue {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            inner: VecDeque::new(),
            samples: 0,
            cap,
            dropped: 0,
        }
    }

    pub(crate) fn push_back(&mut self, e: Event) {
        if matches!(e, Event::Sample { .. }) {
            self.samples = self.samples.saturating_add(1);
        }
        self.inner.push_back(e);
        while self.inner.len() > self.cap && self.samples > 0 {
            self.drop_oldest_sample();
        }
    }

    pub(crate) fn extend(&mut self, events: impl IntoIterator<Item = Event>) {
        for e in events {
            self.push_back(e);
        }
    }

    pub(crate) fn pop_front(&mut self) -> Option<Event> {
        let e = self.inner.pop_front()?;
        if matches!(e, Event::Sample { .. }) {
            self.samples = self.samples.saturating_sub(1);
        }
        Some(e)
    }

    pub(crate) fn clear(&mut self) {
        self.inner.clear();
        self.samples = 0;
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub(crate) fn dropped(&self) -> u64 {
        self.dropped
    }

    fn drop_oldest_sample(&mut self) {
        if let Some(at) = self
            .inner
            .iter()
            .position(|e| matches!(e, Event::Sample { .. }))
        {
            self.inner.remove(at);
            self.samples = self.samples.saturating_sub(1);
            self.dropped = self.dropped.saturating_add(1);
        } else {
            self.samples = 0;
        }
    }
}
