//! Per-request inactivity deadlines: a request is stalled when `limit_us` passed since its last
//! activity (the response head or a body byte). The heap holds the deadline computed when a
//! request was armed or last re-armed; an entry whose request has been active since is re-armed
//! at its true deadline instead of expiring it.

use alloc::vec::Vec;

use super::deadlines::Deadlines;

#[derive(Debug)]
pub(crate) struct Inactivity {
    limit_us: u64,
    deadlines: Deadlines<u32>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Expired {
    pub(crate) stalled: Vec<u32>,
    pub(crate) wakes: Vec<u64>,
}

impl Inactivity {
    pub(crate) fn new(limit_us: u64) -> Self {
        Self {
            limit_us,
            deadlines: Deadlines::new(),
        }
    }

    /// Starts watching `conn`, last active at `now`; the instant the host must poll at by.
    pub(crate) fn arm(&mut self, conn: u32, now: u64) -> Option<u64> {
        let deadline = now.checked_add(self.limit_us)?;
        self.deadlines.arm(conn, deadline);
        Some(deadline)
    }

    /// The connections stalled at `now`, and the new instants to poll at for the ones still
    /// active. `last_activity` is `None` for a connection that is gone.
    pub(crate) fn expire(
        &mut self,
        now: u64,
        last_activity: impl Fn(u32) -> Option<u64>,
    ) -> Expired {
        let candidates = self
            .deadlines
            .expire(now, |conn, _| last_activity(*conn).is_some());
        let mut out = Expired::default();
        for conn in candidates {
            let Some(deadline) = last_activity(conn).and_then(|a| a.checked_add(self.limit_us))
            else {
                continue;
            };
            if deadline <= now {
                if !out.stalled.contains(&conn) {
                    out.stalled.push(conn);
                }
            } else {
                self.deadlines.arm(conn, deadline);
                out.wakes.push(deadline);
            }
        }
        out.wakes.sort_unstable();
        out.wakes.dedup();
        out
    }

    pub(crate) fn compact(&mut self, live: impl Fn() -> Vec<(u32, u64)>) {
        let limit = self.limit_us;
        let live = live();
        self.deadlines.compact(live.len(), || {
            live.iter()
                .filter_map(|(conn, activity)| activity.checked_add(limit).map(|d| (d, *conn)))
        });
    }

    pub(crate) fn clear(&mut self) {
        self.deadlines.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: u64 = 100;

    #[test]
    fn a_silent_connection_stalls_at_exactly_the_limit() {
        let mut watch = Inactivity::new(LIMIT);
        watch.arm(1, 10);
        assert_eq!(watch.expire(109, |_| Some(10)), Expired::default());
        assert_eq!(watch.expire(110, |_| Some(10)).stalled, vec![1]);
    }

    #[test]
    fn activity_moves_the_deadline_and_asks_for_a_new_wake() {
        let mut watch = Inactivity::new(LIMIT);
        watch.arm(1, 0);
        let early = watch.expire(100, |_| Some(60));
        assert_eq!(early.stalled, Vec::<u32>::new());
        assert_eq!(early.wakes, vec![160]);
        assert_eq!(watch.expire(159, |_| Some(60)), Expired::default());
        assert_eq!(watch.expire(160, |_| Some(60)).stalled, vec![1]);
    }

    #[test]
    fn a_gone_connection_never_stalls() {
        let mut watch = Inactivity::new(LIMIT);
        watch.arm(1, 0);
        assert_eq!(watch.expire(1_000, |_| None), Expired::default());
    }

    #[test]
    fn connections_are_independent() {
        let mut watch = Inactivity::new(LIMIT);
        watch.arm(1, 0);
        watch.arm(2, 0);
        let out = watch.expire(100, |conn| Some(if conn == 1 { 0 } else { 90 }));
        assert_eq!(out.stalled, vec![1]);
        assert_eq!(out.wakes, vec![190]);
    }

    #[test]
    fn an_overflowing_deadline_is_never_armed() {
        let mut watch = Inactivity::new(LIMIT);
        assert_eq!(watch.arm(1, u64::MAX), None);
    }
}
