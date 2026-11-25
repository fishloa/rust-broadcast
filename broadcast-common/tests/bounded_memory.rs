#![cfg(feature = "std")]

//! An over-limit bounded read allocates in proportion to the limit, never to
//! the input: a hostile stream that never ends must not grow the buffer.

use std::io::Read;

use broadcast_common::bounded::{BoundedReadError, Limit, read_bounded};

/// The cap under test, in bytes.
const LIMIT_BYTES: u64 = 64 * 1024;
/// The cap under test.
const LIMIT: Limit = Limit::new("endless", LIMIT_BYTES);
/// `read_to_end` may grow a `Vec` by doubling, so one step past the cap is at
/// most twice it; the rest covers the test harness's own allocations on this
/// thread.
const VEC_GROWTH: usize = 2 * LIMIT_BYTES as usize;
/// Named slack for the harness and the error value.
const SLACK: usize = 64 * 1024;
/// Per-thread live-byte ceiling.
const BUDGET: usize = VEC_GROWTH + SLACK;

#[global_allocator]
static ALLOC: test_alloc::ThreadCapped<BUDGET> = test_alloc::ThreadCapped::new();

/// A stream of zeros with a declared length that is never allocated.
struct Endless {
    remaining: u64,
}

impl Read for Endless {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let want = u64::try_from(buf.len()).unwrap_or(u64::MAX);
        let count = usize::try_from(self.remaining.min(want)).unwrap_or(usize::MAX);
        buf[..count].fill(0);
        self.remaining = self.remaining.saturating_sub(count as u64);
        Ok(count)
    }
}

#[test]
fn an_over_limit_read_allocates_no_more_than_the_cap() {
    // Four gibibytes claimed; reading them all would abort every thread.
    let mut input = Endless {
        remaining: 4 * 1024 * 1024 * 1024,
    };
    let error = read_bounded(&mut input, LIMIT).expect_err("over the cap");
    assert!(matches!(
        error,
        BoundedReadError::TooLarge {
            limit_name: "endless",
            limit_bytes: LIMIT_BYTES,
        }
    ));
}

#[test]
fn an_at_or_under_limit_read_is_returned() {
    let mut input = Endless {
        remaining: LIMIT_BYTES,
    };
    let bytes = read_bounded(&mut input, LIMIT).expect("at the cap");
    assert_eq!(u64::try_from(bytes.len()).unwrap_or(u64::MAX), LIMIT_BYTES);
}
