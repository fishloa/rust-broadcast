//! The four bounds every awaited network I/O of a protocol adapter carries:
//! connect, handshake, idle read and write.
//!
//! One type for the RTSP, RTMP and SRT adapters (and any later one) so a host
//! maps its own configuration onto timeouts once, and no adapter owns a
//! private copy of the same four fields.

use core::time::Duration;

/// Bounds on every awaited I/O of a protocol adapter: no wait is unbounded.
///
/// [`Default`] is the bound a TCP signalling protocol wants (10 s connect and
/// handshake, 30 s idle read, 10 s write); a protocol with other norms starts
/// from [`Timeouts::new`] and its own values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Timeouts {
    /// Resolving and connecting (or binding) the transport.
    pub connect: Duration,
    /// Completing the protocol handshake (TLS, a handshake exchange, or the
    /// first complete request on a server connection).
    pub handshake: Duration,
    /// Longest wait for the next complete frame from a connected peer. One
    /// deadline covers the whole frame, so a peer dripping bytes still times
    /// out.
    pub read_idle: Duration,
    /// Longest wait for the transport to accept a write.
    pub write: Duration,
}

impl Timeouts {
    /// Timeouts with every bound given.
    #[must_use]
    pub const fn new(
        connect: Duration,
        handshake: Duration,
        read_idle: Duration,
        write: Duration,
    ) -> Self {
        Timeouts {
            connect,
            handshake,
            read_idle,
            write,
        }
    }

    /// Sets [`connect`](Self::connect).
    #[must_use]
    pub const fn with_connect(mut self, connect: Duration) -> Self {
        self.connect = connect;
        self
    }

    /// Sets [`handshake`](Self::handshake).
    #[must_use]
    pub const fn with_handshake(mut self, handshake: Duration) -> Self {
        self.handshake = handshake;
        self
    }

    /// Sets [`read_idle`](Self::read_idle).
    #[must_use]
    pub const fn with_read_idle(mut self, read_idle: Duration) -> Self {
        self.read_idle = read_idle;
        self
    }

    /// Sets [`write`](Self::write).
    #[must_use]
    pub const fn with_write(mut self, write: Duration) -> Self {
        self.write = write;
        self
    }
}

impl Default for Timeouts {
    fn default() -> Self {
        Timeouts::new(
            Duration::from_secs(10),
            Duration::from_secs(10),
            Duration::from_secs(30),
            Duration::from_secs(10),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_with_sets_only_its_own_field() {
        let base = Timeouts::default();
        let one = Duration::from_secs(1);
        assert_eq!(
            base.with_connect(one),
            Timeouts {
                connect: one,
                ..base
            }
        );
        assert_eq!(
            base.with_handshake(one),
            Timeouts {
                handshake: one,
                ..base
            }
        );
        assert_eq!(
            base.with_read_idle(one),
            Timeouts {
                read_idle: one,
                ..base
            }
        );
        assert_eq!(base.with_write(one), Timeouts { write: one, ..base });
    }
}
