//! TPDU transport layer — a sans-IO state machine for the single transport
//! connection per CI slot (ETSI EN 50221 §A.4).
//!
//! Connection lifecycle (Figures 6/7): `Idle → Creating → Active`. The host
//! sends `Create_T_C`; the module answers `C_T_C_Reply` (→ `Active`) or the
//! host times out back to `Idle`. In `Active` the host **polls regularly** —
//! per §A.4, a poll is a `T_Data_Last` with an empty data field — and, whenever
//! a Status Byte reports Data-Available (DA), sends `T_RCV` to receive the
//! queued message. Chained module messages arrive as `T_Data_More*` then a
//! final `T_Data_Last` and are reassembled into one SPDU payload.
//!
//! Timing: EN 50221 mandates regular polling and a reply-timeout arc but does
//! not fix the interval, so [`DEFAULT_POLL_INTERVAL`] / [`DEFAULT_REPLY_TIMEOUT`]
//! are implementation-chosen defaults. All timing is expressed via the sans-IO
//! [`Tick`](crate::event::Event::Tick)/timer model so it is deterministic and
//! testable without a clock.

use std::collections::VecDeque;
use std::time::Duration;

use broadcast_common::{Parse, Serialize};
use dvb_ci::tpdu::{CommandTpdu, DataBlock, ResponseTpdu, SbValue, TcObject, create_t_c, tags};

/// Length of a standalone/appended `T_SB` object: `tag · 0x02 · t_c_id · SB`.
const SB_OBJECT_LEN: usize = 4;

/// Parse a `T_SB` object (`0x80 0x02 t_c_id sb_value`) at the start of `bytes`.
fn parse_sb(bytes: &[u8]) -> Option<(u8, SbValue)> {
    if bytes.len() >= SB_OBJECT_LEN && bytes[0] == tags::SB && bytes[1] == 0x02 {
        Some((bytes[2], SbValue(bytes[3])))
    } else {
        None
    }
}

/// Conventional host poll interval (implementation-chosen; §A.4 mandates only
/// "poll regularly").
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(100);
/// Conventional reply timeout for an expected `R_TPDU` (the §A.4 Figure 6/7
/// "Timeout" arc; value implementation-chosen).
pub const DEFAULT_REPLY_TIMEOUT: Duration = Duration::from_millis(1000);

/// Transport connection state (EN 50221 §A.4, Figures 6/7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TcState {
    /// No connection; nothing sent.
    Idle,
    /// `Create_T_C` sent, awaiting `C_T_C_Reply`.
    Creating,
    /// Connection up; polling/exchanging data.
    Active,
}

/// What the transport layer wants done after handling an input.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Out {
    /// Link-layer TPDU frames to write to the device, in order.
    pub writes: Vec<Vec<u8>>,
    /// Fully-reassembled SPDU payloads to pass up to the session layer.
    pub spdus: Vec<Vec<u8>>,
    /// Requested delay until the next [`Tick`](crate::event::Event::Tick).
    pub timer: Option<Duration>,
    /// A transport error (e.g. reply timeout, unexpected tag).
    pub error: Option<TransportError>,
}

/// Transport-layer errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TransportError {
    /// No `C_T_C_Reply` within the reply timeout (§A.4 timeout arc).
    #[error("transport connection setup timed out")]
    SetupTimeout,
    /// A C_TPDU (poll or data block) sent while `Active` went unanswered
    /// past the reply timeout (§A.4 timeout arc, applied to the `Active`
    /// state rather than only connection setup).
    #[error("no reply to an in-flight C_TPDU within the reply timeout")]
    ReplyTimeout,
    /// A reply arrived for a different `t_c_id` than ours.
    #[error("unexpected t_c_id {got} (expected {expected})")]
    WrongTcId {
        /// The `t_c_id` received.
        got: u8,
        /// Our connection's `t_c_id`.
        expected: u8,
    },
    /// The module reported a `T_C_Error`.
    #[error("module reported T_C_Error")]
    ModuleError,
    /// A frame could not be parsed as an `R_TPDU`.
    #[error("malformed R_TPDU")]
    Malformed,
    /// A chained `T_Data_More*` reassembly exceeded [`MAX_REASSEMBLY_LEN`]
    /// without a terminating `T_Data_Last` — the partial reassembly is
    /// dropped rather than retained (a well-behaved module chains a bounded
    /// number of fragments; this guards a stuck or hostile one from growing
    /// host memory without limit).
    #[error("chained TPDU reassembly exceeded {MAX_REASSEMBLY_LEN} bytes without a T_Data_Last")]
    ReassemblyOverflow,
    /// [`Transport::send_spdu`] was called with [`MAX_OUTBOUND_QUEUE`] SPDUs
    /// already queued — the caller is producing faster than the half-duplex
    /// link can drain (one block per module turn); queueing further would
    /// grow unboundedly. The new SPDU is dropped; the caller should retry
    /// once earlier sends have drained.
    #[error("outbound queue is full ({MAX_OUTBOUND_QUEUE} SPDUs already queued)")]
    OutboundQueueFull,
    /// [`Transport::send_spdu`] was called with an SPDU longer than
    /// [`MAX_SPDU_LEN`] — it cannot be wrapped in a single `T_Data_Last`
    /// `CommandTpdu` (`length_field`'s own max encodable value). Previously
    /// this was only discovered when the resulting `CommandTpdu` failed to
    /// serialize, which `Transport::cmd` reported by panicking.
    #[error(
        "SPDU of {len} bytes exceeds the {MAX_SPDU_LEN}-byte maximum a single T_Data_Last can carry"
    )]
    SpduTooLarge {
        /// The SPDU's actual length.
        len: usize,
    },
}

/// Defensive cap on a single chained-fragment reassembly (`Transport::on_data`).
/// Not a spec limit — EN 50221 places no bound on how many `T_Data_More`
/// fragments may precede a `T_Data_Last` — but an unbounded accumulator lets
/// a stuck or hostile module grow host memory without limit; 2 MiB is far
/// beyond any legitimate CI application object this transport carries
/// (`dvb-ci`'s own largest length-prefixed objects top out at 65535 bytes).
pub const MAX_REASSEMBLY_LEN: usize = 2 * 1024 * 1024;

/// Defensive cap on [`Transport::send_spdu`]'s queue depth. The link sends
/// one queued block per module turn (§A.4 half-duplex, issue #337), so a
/// caller enqueueing faster than that drains would otherwise grow the queue
/// without limit.
pub const MAX_OUTBOUND_QUEUE: usize = 64;

/// Largest SPDU [`Transport::send_spdu`] can wrap in one `T_Data_Last`
/// `CommandTpdu`: `length_field`'s own max encodable value (65535, EN 50221
/// Table 1) minus the 1-byte `t_c_id` that shares the length-prefixed span.
pub const MAX_SPDU_LEN: usize = 65_535 - 1;

/// The single transport connection for a slot.
#[derive(Debug)]
pub struct Transport {
    tcid: u8,
    state: TcState,
    reassembly: Vec<u8>,
    poll_interval: Duration,
    reply_timeout: Duration,
    /// Time accumulated since the last poll (drives the poll cadence).
    since_poll: Duration,
    /// Time accumulated since a command that expects a reply (drives the
    /// reply timeout); `None` when not awaiting a reply. While `Some`, a host
    /// C_TPDU is *in flight* (sent, module not yet answered) — the link is
    /// half-duplex, so no further data block may be sent until it clears.
    awaiting: Option<Duration>,
    /// SPDUs queued to send, one `T_Data_Last` per module turn. EN 50221's link
    /// is polled half-duplex: the host sends a single data block, then must wait
    /// for the module's `T_SB` before sending the next. Sending two back-to-back
    /// makes a real CAM drop the second (issue #337).
    outbound: VecDeque<Vec<u8>>,
}

impl Default for Transport {
    fn default() -> Self {
        Self::new(1)
    }
}

impl Transport {
    /// New transport for `tcid` with default timings.
    #[must_use]
    pub fn new(tcid: u8) -> Self {
        Self {
            tcid,
            state: TcState::Idle,
            reassembly: Vec::new(),
            poll_interval: DEFAULT_POLL_INTERVAL,
            reply_timeout: DEFAULT_REPLY_TIMEOUT,
            since_poll: Duration::ZERO,
            awaiting: None,
            outbound: VecDeque::new(),
        }
    }

    /// Override the poll interval / reply timeout.
    #[must_use]
    pub fn with_timing(mut self, poll: Duration, reply: Duration) -> Self {
        self.poll_interval = poll;
        self.reply_timeout = reply;
        self
    }

    /// Current connection state.
    #[must_use]
    pub fn state(&self) -> TcState {
        self.state
    }

    /// `data` must be at most [`MAX_SPDU_LEN`] bytes — `send_spdu` enforces
    /// this before ever queueing, and every other caller passes empty data,
    /// so `serialize_into` failing here would mean that invariant broke,
    /// not a normal runtime condition (r10-W-20: this used to be able to
    /// panic on caller-controlled data that reached here unchecked).
    fn cmd(&self, tag: u8, data: &[u8]) -> Vec<u8> {
        let c = CommandTpdu {
            tag,
            t_c_id: self.tcid,
            data,
        };
        let mut buf = vec![0u8; c.serialized_len()];
        let n = c
            .serialize_into(&mut buf)
            .expect("data.len() <= MAX_SPDU_LEN, enforced by every caller");
        buf.truncate(n);
        buf
    }

    fn poll_frame(&self) -> Vec<u8> {
        // §A.4: poll == T_Data_Last with empty data.
        self.cmd(tags::DATA_LAST, &[])
    }

    /// Open the connection: emit `Create_T_C` and arm the reply timeout.
    pub fn init(&mut self) -> Out {
        self.state = TcState::Creating;
        self.awaiting = Some(Duration::ZERO);
        let obj: TcObject = create_t_c(self.tcid);
        Out {
            writes: vec![obj.to_bytes()],
            timer: Some(self.reply_timeout),
            ..Out::default()
        }
    }

    /// Queue an upper-layer SPDU to send (wrapped in a `T_Data_Last`). The block
    /// is transmitted now if the link is free, else held until the in-flight
    /// C_TPDU is answered — one data block per module turn (§A.4 half-duplex).
    pub fn send_spdu(&mut self, spdu: &[u8]) -> Out {
        if self.state != TcState::Active {
            return Out::default();
        }
        // r10-W-20: reject up front rather than queueing an SPDU that would
        // later fail to serialize (previously discovered by `cmd`
        // panicking deep inside `flush`/`tick`).
        if spdu.len() > MAX_SPDU_LEN {
            return Out {
                error: Some(TransportError::SpduTooLarge { len: spdu.len() }),
                ..Out::default()
            };
        }
        // r10-W-17: cap queue depth so a caller producing faster than the
        // half-duplex link drains cannot grow it without limit.
        if self.outbound.len() >= MAX_OUTBOUND_QUEUE {
            return Out {
                error: Some(TransportError::OutboundQueueFull),
                ..Out::default()
            };
        }
        self.outbound.push_back(spdu.to_vec());
        self.flush()
    }

    /// Emit the next queued data block if the link is free (Active and no
    /// C_TPDU in flight); otherwise nothing (it waits for the module's `T_SB`).
    fn flush(&mut self) -> Out {
        if self.state != TcState::Active || self.awaiting.is_some() {
            return Out::default();
        }
        match self.outbound.pop_front() {
            Some(spdu) => {
                self.awaiting = Some(Duration::ZERO);
                self.since_poll = Duration::ZERO;
                Out {
                    writes: vec![self.cmd(tags::DATA_LAST, &spdu)],
                    timer: Some(self.poll_interval),
                    ..Out::default()
                }
            }
            None => Out::default(),
        }
    }

    /// Advance logical time by `elapsed`: poll if due, or time out a pending
    /// reply.
    pub fn tick(&mut self, elapsed: Duration) -> Out {
        match self.state {
            TcState::Idle => Out::default(),
            TcState::Creating => {
                if let Some(w) = self.awaiting.as_mut() {
                    *w += elapsed;
                    if *w >= self.reply_timeout {
                        self.state = TcState::Idle;
                        self.awaiting = None;
                        // r10-W-17: SPDUs queued against the failed
                        // connection attempt must not linger and get sent
                        // against a later, unrelated re-established
                        // connection.
                        self.outbound.clear();
                        return Out {
                            error: Some(TransportError::SetupTimeout),
                            ..Out::default()
                        };
                    }
                }
                Out {
                    timer: Some(self.reply_timeout),
                    ..Out::default()
                }
            }
            TcState::Active => {
                // r10-W-16: a C_TPDU (poll or data block) we already sent is
                // half-duplex — the link cannot carry another until the
                // module answers it. Track how long it's been outstanding
                // and time it out, exactly as `Creating` does for
                // `Create_T_C`; only once it clears (cleared by `on_frame`,
                // or here on timeout) does the poll cadence below apply.
                // Previously this branch never consulted `awaiting` at all
                // once `since_poll` crossed `poll_interval`, so it just kept
                // sending a fresh poll on top of a still-outstanding one.
                if let Some(w) = self.awaiting.as_mut() {
                    *w += elapsed;
                    if *w >= self.reply_timeout {
                        self.awaiting = None;
                        return Out {
                            error: Some(TransportError::ReplyTimeout),
                            timer: Some(self.poll_interval),
                            ..Out::default()
                        };
                    }
                    return Out {
                        timer: Some(self.reply_timeout - *w),
                        ..Out::default()
                    };
                }
                self.since_poll += elapsed;
                if self.since_poll >= self.poll_interval {
                    self.since_poll = Duration::ZERO;
                    // A queued data block goes out in preference to an empty poll.
                    if !self.outbound.is_empty() {
                        return self.flush();
                    }
                    self.awaiting = Some(Duration::ZERO);
                    Out {
                        writes: vec![self.poll_frame()],
                        timer: Some(self.poll_interval),
                        ..Out::default()
                    }
                } else {
                    Out {
                        timer: Some(self.poll_interval - self.since_poll),
                        ..Out::default()
                    }
                }
            }
        }
    }

    /// Handle one link-layer frame read from the device.
    ///
    /// A module frame is a leading object (`C_T_C_Reply` / `T_Data_*` / …)
    /// followed by an appended `T_SB`, or a standalone `T_SB` (the reply to a
    /// poll with nothing queued). The `T_SB`'s DA bit drives whether the host
    /// must `T_RCV` next.
    pub fn on_frame(&mut self, frame: &[u8]) -> Out {
        self.awaiting = None;
        match frame.first().copied() {
            // C_T_C_Reply (+ appended T_SB): connection becomes Active.
            Some(tags::C_T_C_REPLY) => match TcObject::parse(frame) {
                Ok(o) if o.t_c_id == self.tcid => {
                    self.state = TcState::Active;
                    self.since_poll = Duration::ZERO;
                    let da = parse_sb(&frame[3..]).is_some_and(|(_, sb)| sb.data_available());
                    self.after_status(da)
                }
                Ok(o) => self.wrong_tcid(o.t_c_id),
                Err(_) => self.malformed(),
            },
            // Standalone T_SB — the reply to a poll.
            Some(tags::SB) => match parse_sb(frame) {
                Some((tcid, _)) if tcid != self.tcid => self.wrong_tcid(tcid),
                Some((_, sb)) => self.after_status(sb.data_available()),
                None => self.malformed(),
            },
            Some(tags::T_C_ERROR) => Out {
                error: Some(TransportError::ModuleError),
                ..Out::default()
            },
            Some(tags::DATA_LAST | tags::DATA_MORE) => self.on_data(frame),
            _ => self.malformed(),
        }
    }

    /// r10-W-17: also clears any in-progress chained-fragment reassembly —
    /// a malformed frame invalidates it; leaving it in place would silently
    /// prepend stale bytes to the NEXT chain's data.
    fn malformed(&mut self) -> Out {
        self.reassembly.clear();
        Out {
            error: Some(TransportError::Malformed),
            ..Out::default()
        }
    }

    /// r10-W-17: also clears reassembly, for the same reason as [`Self::malformed`].
    fn wrong_tcid(&mut self, got: u8) -> Out {
        self.reassembly.clear();
        Out {
            error: Some(TransportError::WrongTcId {
                got,
                expected: self.tcid,
            }),
            ..Out::default()
        }
    }

    /// React to a Status Byte: if DA, solicit the queued message with `T_RCV`;
    /// otherwise resume the idle poll cadence.
    fn after_status(&mut self, data_available: bool) -> Out {
        if data_available {
            self.awaiting = Some(Duration::ZERO);
            Out {
                writes: vec![self.cmd(tags::RCV, &[])],
                ..Out::default()
            }
        } else {
            // Module idle: its `T_SB` freed the link, so send the next queued
            // data block if any (the #337 fix); otherwise resume polling.
            if !self.outbound.is_empty() {
                return self.flush();
            }
            self.since_poll = Duration::ZERO;
            Out {
                timer: Some(self.poll_interval),
                ..Out::default()
            }
        }
    }

    fn on_data(&mut self, frame: &[u8]) -> Out {
        let r = match ResponseTpdu::parse(frame) {
            Ok(r) => r,
            Err(_) => return self.malformed(),
        };
        if r.t_c_id != self.tcid {
            return self.wrong_tcid(r.t_c_id);
        }
        self.reassembly.extend_from_slice(r.data);
        if self.reassembly.len() > MAX_REASSEMBLY_LEN {
            // r10-W-17: EN 50221 places no bound on the number of chained
            // `T_Data_More` fragments — without this cap a stuck or hostile
            // module could grow host memory without limit.
            self.reassembly.clear();
            return Out {
                error: Some(TransportError::ReassemblyOverflow),
                ..Out::default()
            };
        }
        match r.block {
            // More chained fragments: each waits for another T_RCV (§A.4 item 10).
            Some(DataBlock::More) => {
                self.awaiting = Some(Duration::ZERO);
                Out {
                    writes: vec![self.cmd(tags::RCV, &[])],
                    ..Out::default()
                }
            }
            // Last (or only) fragment: emit the reassembled SPDU, then let the
            // appended Status Byte decide whether to receive another message.
            _ => {
                let mut out = self.after_status(r.sb_value.data_available());
                if !self.reassembly.is_empty() {
                    out.spdus.push(core::mem::take(&mut self.reassembly));
                }
                out
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvb_ci::tpdu::SbValue;

    /// Build an R_TPDU frame (module→host) for tests.
    fn r_tpdu(tag: u8, tcid: u8, data: &[u8], da: bool) -> Vec<u8> {
        // tag, length_field(=1+data), tcid, data..., SB(0x80), len=2, tcid, sb_value
        let mut v = vec![tag];
        v.push((1 + data.len()) as u8);
        v.push(tcid);
        v.extend_from_slice(data);
        v.extend_from_slice(&[tags::SB, 0x02, tcid, SbValue::new(da).0]);
        v
    }

    #[test]
    fn init_sends_create_tc_and_arms_timeout() {
        let mut t = Transport::new(1);
        let out = t.init();
        assert_eq!(out.writes, vec![vec![tags::CREATE_T_C, 0x01, 0x01]]);
        assert_eq!(t.state(), TcState::Creating);
        assert_eq!(out.timer, Some(DEFAULT_REPLY_TIMEOUT));
    }

    #[test]
    fn setup_times_out_to_idle() {
        let mut t = Transport::new(1);
        t.init();
        let out = t.tick(DEFAULT_REPLY_TIMEOUT);
        assert_eq!(out.error, Some(TransportError::SetupTimeout));
        assert_eq!(t.state(), TcState::Idle);
    }

    #[test]
    fn reply_activates_then_polls_on_interval() {
        let mut t = Transport::new(1);
        t.init();
        let out = t.on_frame(&[tags::C_T_C_REPLY, 0x01, 0x01]);
        assert_eq!(t.state(), TcState::Active);
        assert!(out.error.is_none());
        // Before the interval: no poll.
        let early = t.tick(DEFAULT_POLL_INTERVAL / 2);
        assert!(early.writes.is_empty());
        // Crossing the interval: an empty T_Data_Last poll.
        let due = t.tick(DEFAULT_POLL_INTERVAL);
        assert_eq!(due.writes, vec![vec![tags::DATA_LAST, 0x01, 0x01]]);
    }

    #[test]
    fn reassembles_more_then_last_into_one_spdu() {
        let mut t = Transport::new(1);
        t.init();
        t.on_frame(&[tags::C_T_C_REPLY, 0x01, 0x01]);
        // MORE: partial data, solicits RCV
        let o1 = t.on_frame(&r_tpdu(tags::DATA_MORE, 1, &[0xAA, 0xBB], false));
        assert!(o1.spdus.is_empty());
        assert_eq!(o1.writes, vec![vec![tags::RCV, 0x01, 0x01]]);
        // LAST: completes the SPDU
        let o2 = t.on_frame(&r_tpdu(tags::DATA_LAST, 1, &[0xCC], false));
        assert_eq!(o2.spdus, vec![vec![0xAA, 0xBB, 0xCC]]);
    }

    #[test]
    fn data_available_triggers_rcv() {
        let mut t = Transport::new(1);
        t.init();
        t.on_frame(&[tags::C_T_C_REPLY, 0x01, 0x01]);
        // LAST with DA set → host must RCV the next queued message.
        let o = t.on_frame(&r_tpdu(tags::DATA_LAST, 1, &[0x01], true));
        assert_eq!(o.spdus, vec![vec![0x01]]);
        assert_eq!(o.writes, vec![vec![tags::RCV, 0x01, 0x01]]);
    }

    #[test]
    fn two_sends_serialize_one_block_per_module_turn() {
        // #337: a real CAM drops a second T_Data_Last sent before it answers the
        // first. Two send_spdu in one turn must emit only ONE write; the second
        // goes out after the module's T_SB.
        let mut t = Transport::new(1);
        t.init();
        t.on_frame(&[tags::C_T_C_REPLY, 0x01, 0x01]);

        let first = t.send_spdu(&[0x92, 0x07]); // e.g. open_session_response
        assert_eq!(first.writes.len(), 1);
        assert_eq!(first.writes[0][0], tags::DATA_LAST);

        // Queued while the first is in flight → no write yet.
        let second = t.send_spdu(&[0x9F, 0x80, 0x10, 0x00]); // profile_enq
        assert!(
            second.writes.is_empty(),
            "second block must wait for the SB"
        );

        // Module acknowledges with a standalone T_SB (data_available = 0).
        let after_sb = t.on_frame(&[tags::SB, 0x02, 0x01, SbValue::new(false).0]);
        assert_eq!(
            after_sb.writes.len(),
            1,
            "second block flushes after the SB"
        );
        assert_eq!(after_sb.writes[0][0], tags::DATA_LAST);
        // It carries the profile_enq payload.
        assert!(
            after_sb.writes[0]
                .windows(4)
                .any(|w| w == [0x9F, 0x80, 0x10, 0x00])
        );
    }

    /// r10-W-16: while a C_TPDU sent in `Active` is still awaiting its
    /// reply, `tick` must not send another one (the link is half-duplex) —
    /// pre-fix, once `since_poll` crossed `poll_interval` again, `tick` sent
    /// a fresh poll regardless of `awaiting`.
    #[test]
    fn active_tick_does_not_poll_while_a_c_tpdu_is_in_flight() {
        let mut t = Transport::new(1);
        t.init();
        t.on_frame(&[tags::C_T_C_REPLY, 0x01, 0x01]);
        // Send a data block: awaiting becomes Some, no reply yet.
        let sent = t.send_spdu(&[0xAA]);
        assert_eq!(sent.writes.len(), 1, "the data block itself goes out");

        // Advance well past one poll_interval, twice over, with no reply.
        let tick1 = t.tick(DEFAULT_POLL_INTERVAL);
        assert!(
            tick1.writes.is_empty(),
            "must not poll while a C_TPDU is in flight, got {:?}",
            tick1.writes
        );
        let tick2 = t.tick(DEFAULT_POLL_INTERVAL);
        assert!(
            tick2.writes.is_empty(),
            "must not poll while a C_TPDU is in flight, got {:?}",
            tick2.writes
        );
    }

    /// r10-W-16: an in-flight C_TPDU that the module never answers must
    /// eventually time out (there was previously no timeout at all for the
    /// `Active` state's `awaiting` — only `Creating`'s `Create_T_C` had
    /// one).
    #[test]
    fn active_reply_timeout_surfaces_error_and_clears_awaiting() {
        let mut t = Transport::new(1);
        t.init();
        t.on_frame(&[tags::C_T_C_REPLY, 0x01, 0x01]);
        t.send_spdu(&[0xAA]);

        let out = t.tick(DEFAULT_REPLY_TIMEOUT);
        assert_eq!(out.error, Some(TransportError::ReplyTimeout));
        assert_eq!(t.state(), TcState::Active, "stays Active, just resets");

        // Awaiting cleared: a later poll can now go out again.
        let due = t.tick(DEFAULT_POLL_INTERVAL);
        assert_eq!(due.writes, vec![vec![tags::DATA_LAST, 0x01, 0x01]]);
    }

    #[test]
    fn wrong_tcid_is_flagged() {
        let mut t = Transport::new(1);
        t.init();
        let o = t.on_frame(&[tags::C_T_C_REPLY, 0x01, 0x09]);
        assert_eq!(
            o.error,
            Some(TransportError::WrongTcId {
                got: 9,
                expected: 1
            })
        );
    }

    /// r10-W-20: an over-long SPDU must be rejected with `SpduTooLarge`
    /// instead of `send_spdu` propagating it through `flush` into `cmd`,
    /// where it used to reach a `.expect()` and panic.
    #[test]
    fn send_spdu_rejects_an_oversized_spdu_instead_of_panicking() {
        let mut t = Transport::new(1);
        t.init();
        t.on_frame(&[tags::C_T_C_REPLY, 0x01, 0x01]);

        let oversized = vec![0u8; MAX_SPDU_LEN + 1];
        let out = t.send_spdu(&oversized);
        assert_eq!(
            out.error,
            Some(TransportError::SpduTooLarge {
                len: oversized.len()
            })
        );
        assert!(out.writes.is_empty());
        assert!(
            t.outbound.is_empty(),
            "the oversized SPDU must not be queued either"
        );
    }

    /// r10-W-17: a malformed frame arriving mid-chain must clear whatever
    /// partial reassembly was in progress, so it does not silently
    /// contaminate the NEXT chain's data with stale bytes.
    #[test]
    fn reassembly_is_cleared_on_a_malformed_frame_mid_chain() {
        let mut t = Transport::new(1);
        t.init();
        t.on_frame(&[tags::C_T_C_REPLY, 0x01, 0x01]);

        // Start a chain: one MORE fragment with stale bytes.
        let o1 = t.on_frame(&r_tpdu(tags::DATA_MORE, 1, &[0xDE, 0xAD], false));
        assert_eq!(o1.writes, vec![vec![tags::RCV, 0x01, 0x01]]);
        assert_eq!(t.reassembly, vec![0xDE, 0xAD]);

        // A malformed frame interrupts the chain.
        let o2 = t.on_frame(&[0xFF]);
        assert_eq!(o2.error, Some(TransportError::Malformed));
        assert!(
            t.reassembly.is_empty(),
            "stale bytes must not survive a malformed frame"
        );

        // A fresh, unrelated chain must reassemble cleanly, with no leftover
        // 0xDE 0xAD prefix from before.
        let o3 = t.on_frame(&r_tpdu(tags::DATA_LAST, 1, &[0x01, 0x02], false));
        assert_eq!(o3.spdus, vec![vec![0x01, 0x02]]);
    }

    /// r10-W-17: chained `T_Data_More` reassembly must not grow without
    /// bound — EN 50221 places no cap on fragment count, so a stuck or
    /// hostile module could otherwise exhaust host memory.
    #[test]
    fn reassembly_overflow_is_rejected_and_cleared() {
        let mut t = Transport::new(1);
        t.init();
        t.on_frame(&[tags::C_T_C_REPLY, 0x01, 0x01]);

        // Feed MORE fragments large enough to cross MAX_REASSEMBLY_LEN.
        // Each R_TPDU's `data` field is capped at 65534 bytes (length_field
        // max 65535 minus the 1-byte t_c_id), so a handful of maximal
        // fragments comfortably crosses the 2 MiB cap. Built via
        // `ResponseTpdu`'s own `Serialize` (not the small-payload `r_tpdu`
        // helper, whose length byte can't represent more than 127 bytes).
        let chunk = vec![0xAAu8; 65_534];
        let frame = ResponseTpdu {
            tag: tags::DATA_MORE,
            t_c_id: 1,
            data: &chunk,
            sb_value: SbValue::new(false),
            block: Some(DataBlock::More),
        }
        .to_bytes();
        let mut overflowed = false;
        for _ in 0..40 {
            let out = t.on_frame(&frame);
            if let Some(err) = out.error {
                assert_eq!(err, TransportError::ReassemblyOverflow);
                overflowed = true;
                break;
            }
        }
        assert!(overflowed, "40 * 65534 bytes must cross MAX_REASSEMBLY_LEN");
        assert!(
            t.reassembly.is_empty(),
            "overflowed reassembly must be cleared, not retained"
        );
    }

    /// r10-W-17: `send_spdu` must not grow the outbound queue without
    /// bound when the caller produces faster than the half-duplex link
    /// drains.
    #[test]
    fn send_spdu_rejects_once_the_outbound_queue_is_full() {
        let mut t = Transport::new(1);
        t.init();
        t.on_frame(&[tags::C_T_C_REPLY, 0x01, 0x01]);

        // The first send goes out immediately and leaves `awaiting` set, so
        // every subsequent send just queues (nothing drains it here).
        t.send_spdu(&[0x00]);
        for _ in 0..MAX_OUTBOUND_QUEUE {
            let out = t.send_spdu(&[0x00]);
            assert!(out.error.is_none());
        }
        assert_eq!(t.outbound.len(), MAX_OUTBOUND_QUEUE);

        let full = t.send_spdu(&[0x00]);
        assert_eq!(full.error, Some(TransportError::OutboundQueueFull));
        assert_eq!(
            t.outbound.len(),
            MAX_OUTBOUND_QUEUE,
            "the rejected SPDU must not be queued"
        );
    }

    /// r10-W-17: SPDUs queued before a connection attempt fails (setup
    /// timeout) must not linger and get sent against a later, unrelated
    /// re-established connection.
    #[test]
    fn outbound_queue_is_cleared_on_setup_timeout() {
        let mut t = Transport::new(1);
        t.init();
        t.on_frame(&[tags::C_T_C_REPLY, 0x01, 0x01]);
        t.send_spdu(&[0xAA]); // in flight
        t.send_spdu(&[0xBB]); // queued behind it
        assert_eq!(t.outbound.len(), 1);

        // Simulate an external re-init (e.g. the session layer resetting
        // the connection) while the old queue is still populated, then let
        // the new attempt time out.
        t.init();
        assert_eq!(t.state(), TcState::Creating);
        let out = t.tick(DEFAULT_REPLY_TIMEOUT);
        assert_eq!(out.error, Some(TransportError::SetupTimeout));
        assert!(
            t.outbound.is_empty(),
            "stale queued SPDUs must not survive into the next connection"
        );
    }
}
