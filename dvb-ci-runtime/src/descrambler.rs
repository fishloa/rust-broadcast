//! Turnkey CAS descrambler — #763 Layer 2.
//!
//! Layer 1 ([`ManagedCa`](crate::managed::ManagedCa) on [`Driver`]) is the
//! sans-IO control plane over the CA device (`caM`): parsed PMT/CAT in,
//! `ca_pmt` APDUs out. This module adds the turnkey wrapper that *also* owns
//! the CI slot's TS **data plane** (`CiDataDevice`, `ciM`): the caller shovels
//! scrambled TS in and gets descrambled TS back out, with no PID math of its
//! own to do.
//!
//! # Feed policy — filter, don't shovel
//!
//! A CI slot descrambles the single TS routed to it, and only needs four PID
//! classes out of that TS: the target services' **ES PIDs**
//! ([`descramble_pids`](crate::driver::Driver::descramble_pids)), their
//! **ECM PIDs** ([`ca_pids`](crate::driver::Driver::ca_pids) — ISO/IEC
//! 13818-1 §2.6.16 `CA_descriptor` `CA_PID`, carrying the control words
//! without which the module has ES to descramble but no key to do it with),
//! the **EMM PIDs** ([`emm_pids`](crate::driver::Driver::emm_pids) —
//! entitlements), and each service's **PCR PID** (ISO/IEC 13818-1 §2.4.4.8 —
//! when it names a dedicated PID distinct from every ES/CA PID, a legitimate
//! DVB configuration, the descrambled TS still needs its clock reference).
//! [`CaDescrambler::feed_ts`] filters the input TS to
//! [`required_pids`](CaDescrambler::required_pids) = `descramble_pids ∪
//! ca_pids ∪ emm_pids ∪ PCR ∪ CAT` (the CAT PID is hardware-evidenced as
//! required, #1296) and writes only those packets to `ci0` — a 30–50
//! Mbit/s mux collapses to the handful of wanted services plus a low-rate
//! ECM/EMM trickle. PAT/PMT are **not** fed on `ci0`: the CAM receives the
//! PMT via the `ca_pmt` control-plane APDU (Layer 1). [`required_pids`](CaDescrambler::required_pids)
//! is also exposed directly so an efficient caller can pre-filter at the
//! tuner/HW PID filter and never hand `feed_ts` the full mux.
//!
//! **Multi-tuner is multi-slot.** One [`CaDescrambler`] = one CI slot = one
//! input TS path. Descrambling services spread across several tuners means
//! one `CaDescrambler` per slot, each fed its own tuner's filtered subset;
//! merging selected services from multiple muxes into a single slot needs a
//! remux + PID remap (PIDs collide across muxes) and is explicitly out of
//! scope here — that's a muxer's job, upstream.
//!
//! The 13-bit PID is read inline (named consts below) rather than pulling in
//! an `mpeg-ts` dependency for one field.

use mpeg_ts::pid::Pid;
use std::collections::BTreeSet;
use std::io;
use std::time::Duration;

use dvb_si::tables::cat::CatSection;
use dvb_si::tables::pmt::PmtSection;

use crate::dataplane::{CiDataDevice, TS_PACKET_LEN, partial_write_progress};
use crate::device::CaDevice;
use crate::driver::Driver;
use crate::event::Notification;
use crate::managed::CaError;

/// MPEG-2 TS sync byte (ISO/IEC 13818-1 §2.4.3.3).
const TS_SYNC_BYTE: u8 = 0x47;
/// Mask for the PID's upper byte within a TS packet header (byte 1): the top
/// 3 bits are `transport_error_indicator`/`payload_unit_start_indicator`/
/// `transport_priority`, the low 5 are `PID[12:8]`.
const TS_PID_HIGH_MASK: u8 = 0x1F;
/// Number of whole TS packets to read from `ci0` per [`CiDataDevice::read`]
/// call while draining descrambled output in `feed_ts` — a batch buffer size,
/// not a wire value.
const READ_BATCH_PACKETS: usize = 32;
/// How many consecutive `write` attempts that moved no packet at all
/// ([`PartialWrite`] with `written == 0`) `feed_ts` retries, draining the
/// device between attempts, before it returns the stall as an error.
const MAX_STALLED_WRITES: u32 = 20;

/// The 13-bit PID carried by one 188-byte TS packet's header (bytes 1–2),
/// masking off the non-PID flag bits in byte 1.
fn packet_pid(packet: &[u8]) -> u16 {
    (u16::from(packet[1] & TS_PID_HIGH_MASK) << 8) | u16::from(packet[2])
}

/// Keep only the packets in `scrambled` whose PID is in `allow`, concatenated
/// in order. Returns `(kept, bad_sync_count)`.
///
/// r10-W-22: a packet whose sync byte isn't `0x47` is **skipped**, not
/// treated as reason to reject the whole batch — a live TS mux occasionally
/// carries a bit-error-corrupted packet (satellite/terrestrial noise, a
/// glitchy demux), and one bad packet among (typically) thousands is not
/// evidence the *batch* is misaligned; only the caller's own `chunks_exact`
/// slicing guarantees alignment here, per-packet corruption does not move
/// it. `bad_sync_count` gives the caller visibility into how many were
/// dropped, rather than silently discarding them with no signal at all.
///
/// # Errors
/// [`io::ErrorKind::InvalidInput`] if `scrambled` is not a whole number of
/// [`TS_PACKET_LEN`]-byte packets — that is a caller framing bug (the input
/// itself isn't packet-aligned), distinct from one packet's payload being
/// corrupted.
fn filter_ts(scrambled: &[u8], allow: &BTreeSet<Pid>) -> io::Result<(Vec<u8>, usize)> {
    if !scrambled.len().is_multiple_of(TS_PACKET_LEN) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "scrambled TS is not a whole number of 188-byte packets",
        ));
    }
    let mut out = Vec::new();
    let mut bad_sync_count = 0usize;
    for packet in scrambled.chunks_exact(TS_PACKET_LEN) {
        if packet[0] != TS_SYNC_BYTE {
            bad_sync_count += 1;
            continue;
        }
        if allow.contains(&Pid::new(packet_pid(packet))) {
            out.extend_from_slice(packet);
        }
    }
    Ok((out, bad_sync_count))
}

/// Turnkey CAS descrambler (#763 Layer 2): a [`Driver`] (control plane, `caM`)
/// paired with a [`CiDataDevice`] (data plane, `ciM`) for one CI slot. See the
/// module docs for the feed-filter policy.
pub struct CaDescrambler<D: CaDevice, C: CiDataDevice> {
    driver: Driver<D>,
    ci: C,
    /// r10-W-22: running count of packets [`Self::feed_ts`] has skipped for
    /// a bad sync byte (see [`Self::bad_sync_packets`]).
    bad_sync_packets: u64,
    /// Descrambled TS already drained from the device by a [`Self::feed_ts`]
    /// call that then failed. Kept (never dropped) and returned at the front
    /// of the next call's output, or via [`Self::take_undelivered`].
    undelivered: Vec<u8>,
}

impl<D: CaDevice, C: CiDataDevice> CaDescrambler<D, C> {
    /// New descrambler over an already-constructed control-plane `driver` and
    /// data-plane `ci` device (both left for the caller to `init()`/wire up
    /// as needed before use).
    #[must_use]
    pub fn new(driver: Driver<D>, ci: C) -> Self {
        Self {
            driver,
            ci,
            bad_sync_packets: 0,
            undelivered: Vec::new(),
        }
    }

    /// Take the descrambled TS that an earlier failed [`Self::feed_ts`] had
    /// already drained from the device before its error. Normally the next
    /// `feed_ts` returns these bytes first (in order); call this to recover
    /// them without feeding more input, e.g. before giving up on the device.
    pub fn take_undelivered(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.undelivered)
    }

    /// Total TS packets [`Self::feed_ts`] has skipped across its lifetime
    /// for carrying a sync byte other than `0x47` (r10-W-22) — a live mux
    /// occasionally has one, and `feed_ts` drops just that packet rather
    /// than rejecting the whole batch; this counter is the caller's only
    /// visibility into that happening, so a persistently climbing count is
    /// worth surfacing (a flaky tuner/demux, not necessarily this crate).
    #[must_use]
    pub fn bad_sync_packets(&self) -> u64 {
        self.bad_sync_packets
    }

    /// Add a service to the descrambled set (delegates to
    /// [`Driver::add_service`]).
    ///
    /// # Errors
    /// See [`Driver::add_service`].
    pub fn add_service(&mut self, pmt: &PmtSection<'_>) -> Result<(), CaError> {
        self.driver.add_service(pmt)
    }

    /// Feed a freshly-parsed CAT to the managed CAS-layer state (delegates to
    /// [`Driver::set_cat`]).
    ///
    /// # Errors
    /// See [`Driver::set_cat`].
    pub fn set_cat(&mut self, cat: &CatSection<'_>) -> Result<(), CaError> {
        self.driver.set_cat(cat)
    }

    /// Filter `scrambled` to [`required_pids`](Self::required_pids) and write
    /// only those packets to `ci0`, then drain and return all
    /// currently-available descrambled TS.
    ///
    /// A packet with a bad sync byte (`!= 0x47`) is skipped, not treated as
    /// grounds to reject the whole batch (r10-W-22) — see
    /// [`Self::bad_sync_packets`] for the running count of how many.
    ///
    /// Descrambled output is never silently lost: if a later write or drain
    /// fails after some output was already drained, those bytes are retained
    /// and returned first by the next `feed_ts` call (or recovered with
    /// [`Self::take_undelivered`]). Input packets not yet accepted by the
    /// device when the error occurs are not retained; the caller re-feeds
    /// them.
    ///
    /// # Errors
    /// [`io::ErrorKind::InvalidInput`] if `scrambled` is not a whole number
    /// of [`TS_PACKET_LEN`]-byte packets; otherwise any I/O error from the
    /// underlying [`CiDataDevice`].
    pub fn feed_ts(&mut self, scrambled: &[u8]) -> io::Result<Vec<u8>> {
        let allow: BTreeSet<Pid> = self.required_pids().into_iter().collect();
        let (kept, bad_sync_count) = filter_ts(scrambled, &allow)?;
        self.bad_sync_packets += bad_sync_count as u64;
        let mut out = std::mem::take(&mut self.undelivered);
        match self.write_and_drain(&kept, &mut out) {
            Ok(()) => Ok(out),
            Err(e) => {
                self.undelivered = out;
                Err(e)
            }
        }
    }

    /// Write `kept` to the device, draining descrambled output into `out`.
    fn write_and_drain(&mut self, kept: &[u8], out: &mut Vec<u8>) -> io::Result<()> {
        // A full device buffer is not fatal: the device reports the whole
        // packets it took ([`PartialWrite`]), and draining the descrambled
        // output is what makes room, so drain and retry the unsent tail. Give
        // up only after a bounded number of rounds that moved nothing.
        let mut pending: &[u8] = kept;
        let mut stalled = 0;
        while !pending.is_empty() {
            match self.ci.write(pending) {
                Ok(()) => break,
                Err(e) => {
                    let Some(written) = partial_write_progress(&e) else {
                        return Err(e);
                    };
                    let written = written.min(pending.len()) / TS_PACKET_LEN * TS_PACKET_LEN;
                    pending = &pending[written..];
                    stalled = if written == 0 { stalled + 1 } else { 0 };
                    if stalled > MAX_STALLED_WRITES {
                        return Err(e);
                    }
                    self.drain_into(out)?;
                }
            }
        }
        self.drain_into(out)
    }

    /// Read everything the module has descrambled so far into `out`.
    fn drain_into(&mut self, out: &mut Vec<u8>) -> io::Result<()> {
        let mut buf = [0u8; READ_BATCH_PACKETS * TS_PACKET_LEN];
        loop {
            let n = self.ci.read(&mut buf)?;
            if n == 0 {
                return Ok(());
            }
            out.extend_from_slice(&buf[..n]);
        }
    }

    /// `descramble_pids ∪ ca_pids ∪ emm_pids ∪ PCR ∪ CAT` — the PIDs the CAM
    /// needs on `ci0` (the CAT PID is always included; delegates to [`Driver::required_pids`]).
    #[must_use]
    pub fn required_pids(&self) -> Vec<Pid> {
        self.driver.required_pids()
    }

    /// Drain the notifications collected so far (delegates to
    /// [`Driver::take_notifications`]).
    pub fn take_notifications(&mut self) -> Vec<Notification> {
        self.driver.take_notifications()
    }

    /// Pump the control-plane device (delegates to [`Driver::pump`]).
    ///
    /// # Errors
    /// See [`Driver::pump`].
    pub fn pump(&mut self, timeout: Duration) -> io::Result<bool> {
        self.driver.pump(timeout)
    }

    /// Borrow the control-plane [`Driver`].
    #[must_use]
    pub fn driver(&self) -> &Driver<D> {
        &self.driver
    }

    /// Mutably borrow the control-plane [`Driver`] (e.g. to drive it
    /// directly for control-plane-only operations this wrapper doesn't
    /// re-expose).
    pub fn driver_mut(&mut self) -> &mut Driver<D> {
        &mut self.driver
    }

    /// Borrow the data-plane [`CiDataDevice`].
    #[must_use]
    pub fn ci(&self) -> &C {
        &self.ci
    }

    /// Mutably borrow the data-plane [`CiDataDevice`].
    pub fn ci_mut(&mut self) -> &mut C {
        &mut self.ci
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataplane::{MockCiDataDevice, PartialWrite};
    use crate::device::MockCaDevice;
    use crate::driver::tests::{
        CA_SESSION, build_ca_pmt_fixture, build_ca_pmt_fixture_dedicated_pcr, build_cat_fixture,
        build_clear_pmt_fixture, ca_descriptor, ca_pmt_reply_for, driver_with_sessions, feed,
        r_apdu, ser,
    };
    use crate::managed::CaError;
    use broadcast_common::Parse;
    use mpeg_ts::pid::well_known::CAT as CAT_PID;

    fn packet(pid: Pid, fill: u8) -> Vec<u8> {
        let mut p = vec![fill; TS_PACKET_LEN];
        p[0] = TS_SYNC_BYTE;
        // Set the reserved top bits (transport_error/pusi/priority) to
        // exercise the mask, per the brief's `hi = 0x40 | (pid>>8)`.
        p[1] = 0x40 | ((pid.value() >> 8) as u8);
        p[2] = pid.value() as u8;
        p
    }

    #[test]
    fn filter_ts_keeps_only_allowed_pids() {
        let p_100 = packet(Pid::new(0x100), 0xAA);
        let p_64 = packet(Pid::new(0x64), 0xBB);
        let p_200 = packet(Pid::new(0x200), 0xCC);

        let mut scrambled = Vec::new();
        scrambled.extend_from_slice(&p_100);
        scrambled.extend_from_slice(&p_64);
        scrambled.extend_from_slice(&p_200);

        let allow: BTreeSet<Pid> = [0x100, 0x64].into_iter().map(Pid::new).collect();
        let (kept, bad_sync) = filter_ts(&scrambled, &allow).unwrap();

        let mut expected = Vec::new();
        expected.extend_from_slice(&p_100);
        expected.extend_from_slice(&p_64);
        assert_eq!(
            kept, expected,
            "0x200 must be dropped, the two allowed packets kept byte-exact and in order"
        );
        assert_eq!(bad_sync, 0);

        // Bite: an empty allow-set drops everything — a reintroduced
        // no-filter passthrough would keep 0x200 and fail this.
        let empty: BTreeSet<Pid> = BTreeSet::new();
        assert!(filter_ts(&scrambled, &empty).unwrap().0.is_empty());

        // Unaligned input is still a hard error (a caller framing bug, not
        // a single corrupted packet).
        assert_eq!(
            filter_ts(&scrambled[..scrambled.len() - 1], &allow)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );

        // r10-W-22: a bad sync byte on ONE packet must not reject the whole
        // batch — it's skipped (counted) and the OTHER, well-formed packets
        // in the same batch are still kept.
        let mut bad = p_100.clone();
        bad[0] = 0x00;
        let mut mixed = Vec::new();
        mixed.extend_from_slice(&bad);
        mixed.extend_from_slice(&p_64);
        let (kept, bad_sync) = filter_ts(&mixed, &allow).unwrap();
        assert_eq!(bad_sync, 1);
        assert_eq!(
            kept, p_64,
            "the well-formed packet after the bad one must still be kept"
        );
    }

    /// Wrap a `Driver<MockCaDevice>` already carrying CAS-layer state into a
    /// `CaDescrambler` over a scripted `MockCiDataDevice`.
    fn descrambler_with(
        driver: Driver<MockCaDevice>,
        descrambled: impl IntoIterator<Item = Vec<u8>>,
    ) -> CaDescrambler<MockCaDevice, MockCiDataDevice> {
        CaDescrambler::new(driver, MockCiDataDevice::new(descrambled))
    }

    #[test]
    fn feed_ts_filters_to_required_pids_and_returns_descrambled() {
        use dvb_ci::objects::ca_info::CaInfo;

        let mut d = driver_with_sessions();
        d.take_notifications();

        // add_service: descramble_pids = [0x100, 0x101], ca_pids = [0x64, 0x65].
        let pmt_bytes = build_ca_pmt_fixture(1546);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();
        d.add_service(&pmt).unwrap();

        // ca_info + set_cat: emm_pids = [0x1FF0] (CAT ∩ ca_info CAIDs, mirrors
        // the Task 4 pattern).
        feed(
            &mut d,
            r_apdu(
                CA_SESSION,
                &ser(&CaInfo {
                    length_form: dvb_ci::length::LengthForm::Minimal,
                    ca_system_ids: vec![0x0648],
                }),
            ),
        );
        d.take_notifications();
        let mut descriptors = Vec::new();
        descriptors.extend_from_slice(&ca_descriptor(0x0648, Pid::new(0x1FF0)));
        let cat_bytes = build_cat_fixture(&descriptors);
        let cat = CatSection::parse(&cat_bytes).unwrap();
        d.set_cat(&cat).unwrap();

        assert_eq!(
            d.required_pids(),
            vec![
                CAT_PID,
                Pid::new(0x0064),
                Pid::new(0x0065),
                Pid::new(0x0100),
                Pid::new(0x0101),
                Pid::new(0x1FF0)
            ],
            "precondition: required_pids = CAT ∪ descramble_pids ∪ ca_pids ∪ emm_pids"
        );

        let descrambled_script = packet(Pid::new(0x100), 0xEE);
        let mut descrambler = descrambler_with(d, [descrambled_script.clone()]);
        assert_eq!(
            descrambler.required_pids(),
            vec![
                CAT_PID,
                Pid::new(0x0064),
                Pid::new(0x0065),
                Pid::new(0x0100),
                Pid::new(0x0101),
                Pid::new(0x1FF0)
            ],
            "required_pids must delegate through the wrapper"
        );

        // One required-PID packet (0x100, an ES PID) + one junk packet on a
        // PID NOT in required_pids.
        let required_pkt = packet(Pid::new(0x100), 0x11);
        let junk_pkt = packet(Pid::new(0x999), 0x22);
        let mut scrambled = Vec::new();
        scrambled.extend_from_slice(&required_pkt);
        scrambled.extend_from_slice(&junk_pkt);

        let out = descrambler.feed_ts(&scrambled).unwrap();

        assert_eq!(
            descrambler.ci().written_ts(),
            required_pkt,
            "ci0 must receive ONLY the required-PID packet; the junk packet on 0x999 must be dropped"
        );
        assert_eq!(
            out, descrambled_script,
            "feed_ts must return the scripted descrambled TS read back from ci0"
        );
    }

    /// r10-W-22: a batch containing one bad-sync packet among otherwise
    /// well-formed ones must still forward the good packets — pre-fix,
    /// `feed_ts` returned `Err` for the whole batch and `ci0` never saw any
    /// of it.
    #[test]
    fn feed_ts_skips_a_bad_sync_packet_and_keeps_the_rest_of_the_batch() {
        let mut d = driver_with_sessions();
        d.take_notifications();
        let pmt_bytes = build_ca_pmt_fixture(1546);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();
        d.add_service(&pmt).unwrap();

        let mut descrambler = descrambler_with(d, [Vec::new()]);
        assert_eq!(descrambler.bad_sync_packets(), 0);

        let mut bad = packet(Pid::new(0x100), 0x11);
        bad[0] = 0x00; // corrupt sync byte
        let good = packet(Pid::new(0x100), 0x33);
        let mut scrambled = Vec::new();
        scrambled.extend_from_slice(&bad);
        scrambled.extend_from_slice(&good);

        let out = descrambler.feed_ts(&scrambled);
        assert!(out.is_ok(), "one bad packet must not fail the whole batch");
        assert_eq!(
            descrambler.ci().written_ts(),
            good,
            "the well-formed packet must still reach ci0"
        );
        assert_eq!(descrambler.bad_sync_packets(), 1);

        // Feeding another bad packet accumulates the count.
        descrambler.feed_ts(&bad).unwrap();
        assert_eq!(descrambler.bad_sync_packets(), 2);
    }

    /// A data device whose input buffer holds `room` packets until it is read:
    /// a `write` bigger than the room takes what fits and reports it as a
    /// [`PartialWrite`] (the contract of [`CiDataDevice::write`]); `read`
    /// drains the buffer (frees the room) and returns the packets it held.
    struct SmallBufferDevice {
        room: usize,
        buffered: Vec<u8>,
        accepted: Vec<u8>,
        write_calls: usize,
    }
    impl CiDataDevice for SmallBufferDevice {
        fn write(&mut self, ts: &[u8]) -> io::Result<()> {
            self.write_calls += 1;
            let free = self.room * TS_PACKET_LEN - self.buffered.len();
            let take = ts.len().min(free);
            self.buffered.extend_from_slice(&ts[..take]);
            self.accepted.extend_from_slice(&ts[..take]);
            if take == ts.len() {
                return Ok(());
            }
            Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                PartialWrite {
                    written: take,
                    requested: ts.len(),
                    device: "test".to_owned(),
                },
            ))
        }
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.buffered.len().min(buf.len());
            buf[..n].copy_from_slice(&self.buffered[..n]);
            self.buffered.drain(..n);
            Ok(n)
        }
        fn poll(&mut self, _timeout: Duration) -> io::Result<bool> {
            Ok(!self.buffered.is_empty())
        }
    }

    /// Scripted device: `write` is either accepted or always a zero-progress
    /// `PartialWrite`; `read` replays `reads` in order (`Err` entries fail).
    struct ScriptedDevice {
        reject_writes: bool,
        reads: std::collections::VecDeque<io::Result<Vec<u8>>>,
    }
    impl CiDataDevice for ScriptedDevice {
        fn write(&mut self, ts: &[u8]) -> io::Result<()> {
            if !self.reject_writes {
                return Ok(());
            }
            Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                PartialWrite {
                    written: 0,
                    requested: ts.len(),
                    device: "test".to_owned(),
                },
            ))
        }
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.reads.pop_front() {
                None => Ok(0),
                Some(Err(e)) => Err(e),
                Some(Ok(ts)) => {
                    buf[..ts.len()].copy_from_slice(&ts);
                    Ok(ts.len())
                }
            }
        }
        fn poll(&mut self, _timeout: Duration) -> io::Result<bool> {
            Ok(!self.reads.is_empty())
        }
    }

    fn scripted(
        reject_writes: bool,
        reads: Vec<io::Result<Vec<u8>>>,
    ) -> CaDescrambler<MockCaDevice, ScriptedDevice> {
        let mut d = driver_with_sessions();
        d.take_notifications();
        CaDescrambler::new(
            d,
            ScriptedDevice {
                reject_writes,
                reads: reads.into(),
            },
        )
    }

    /// Output already drained when a later read fails must not be dropped: it
    /// is delivered, in order, by the next `feed_ts`.
    #[test]
    fn feed_ts_keeps_drained_output_when_a_later_read_fails() {
        let a = packet(Pid::new(0x100), 0xA1);
        let b = packet(Pid::new(0x100), 0xB2);
        let mut descrambler = scripted(
            false,
            vec![
                Ok(a.clone()),
                Err(io::Error::other("boom")),
                Ok(b.clone()),
            ],
        );
        let err = descrambler.feed_ts(&[]).unwrap_err();
        assert_eq!(err.to_string(), "boom");
        let got = descrambler.feed_ts(&[]).unwrap();
        assert_eq!(got, [a, b].concat(), "A drained before the error must survive");
        assert!(descrambler.take_undelivered().is_empty());
    }

    /// Same guarantee on the stalled-write give-up path, recovered via
    /// `take_undelivered`.
    #[test]
    fn feed_ts_keeps_drained_output_when_the_device_stalls() {
        let a = packet(Pid::new(0x100), 0xA1);
        let mut descrambler = scripted(true, vec![Ok(a.clone())]);
        let input = packet(CAT_PID, 0x11);
        let err = descrambler.feed_ts(&input).unwrap_err();
        assert!(partial_write_progress(&err).is_some());
        assert_eq!(descrambler.take_undelivered(), a);
        assert!(descrambler.take_undelivered().is_empty());
    }

    /// Review ci-cam #7: a device buffer smaller than the burst must not lose
    /// or reorder packets — `feed_ts` resumes after the whole packets the
    /// device reported taking, draining between attempts.
    #[test]
    fn feed_ts_resumes_after_a_partial_write_without_losing_packets() {
        let mut d = driver_with_sessions();
        d.take_notifications();
        let pmt_bytes = build_ca_pmt_fixture(1546);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();
        d.add_service(&pmt).unwrap();
        let dev = SmallBufferDevice {
            room: 3,
            buffered: Vec::new(),
            accepted: Vec::new(),
            write_calls: 0,
        };
        let mut descrambler = CaDescrambler::new(d, dev);
        let burst: Vec<u8> = (0..10u8).flat_map(|i| packet(Pid::new(0x100), i)).collect();
        let out = descrambler.feed_ts(&burst).unwrap();
        assert_eq!(
            descrambler.ci().accepted,
            burst,
            "every packet, once, in order"
        );
        assert_eq!(out, burst, "the drain between attempts returns them all");
        assert!(descrambler.ci().write_calls > 1, "the burst needed retries");
    }

    /// A device that never makes room is reported, not spun on forever.
    #[test]
    fn feed_ts_gives_up_on_a_device_that_never_drains() {
        let mut d = driver_with_sessions();
        d.take_notifications();
        let pmt_bytes = build_ca_pmt_fixture(1546);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();
        d.add_service(&pmt).unwrap();
        let dev = SmallBufferDevice {
            room: 0,
            buffered: Vec::new(),
            accepted: Vec::new(),
            write_calls: 0,
        };
        let mut descrambler = CaDescrambler::new(d, dev);
        let err = descrambler
            .feed_ts(&packet(Pid::new(0x100), 1))
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(
            descrambler.ci().write_calls,
            MAX_STALLED_WRITES as usize + 1
        );
    }

    #[test]
    fn feed_ts_keeps_a_dedicated_pcr_pid_packet() {
        // #763 final-review Fix 1: a service whose PCR is carried on a
        // dedicated PID (distinct from every ES/CA PID) must still have that
        // PID routed to ci0 — otherwise the descrambled TS read back has no
        // clock reference.
        let mut d = driver_with_sessions();
        d.take_notifications();

        let pmt_bytes = build_ca_pmt_fixture_dedicated_pcr(1550);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();
        d.add_service(&pmt).unwrap();

        assert!(
            d.required_pids().contains(&Pid::new(0x00FF)),
            "precondition: required_pids must include the dedicated PCR PID, got {:?}",
            d.required_pids()
        );

        let mut descrambler = descrambler_with(d, []);

        // One packet on the dedicated PCR PID + one junk packet on a PID not
        // in required_pids.
        let pcr_pkt = packet(Pid::new(0x00FF), 0x33);
        let junk_pkt = packet(Pid::new(0x0AAA), 0x44);
        let mut scrambled = Vec::new();
        scrambled.extend_from_slice(&pcr_pkt);
        scrambled.extend_from_slice(&junk_pkt);

        descrambler.feed_ts(&scrambled).unwrap();

        assert_eq!(
            descrambler.ci().written_ts(),
            pcr_pkt,
            "ci0 must receive the dedicated PCR PID packet (clock reference); \
             the junk packet must be dropped"
        );
    }

    #[test]
    fn take_notifications_delegates_entitlement() {
        use dvb_ci::objects::ca_pmt_reply::CaEnable;

        let mut d = driver_with_sessions();
        d.take_notifications();

        let pmt_bytes = build_ca_pmt_fixture(1546);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();
        d.add_service(&pmt).unwrap();
        d.take_notifications();

        let mut descrambler = descrambler_with(d, []);

        // First-ever ca_pmt_reply for a tracked program: establishes the
        // baseline AND reports it (ManagedCa::record_reply), so it must
        // surface as Notification::Entitlement through the wrapper's
        // take_notifications() — not via feed_ts.
        feed(
            descrambler.driver_mut(),
            r_apdu(
                CA_SESSION,
                &ser(&ca_pmt_reply_for(1546, Some(CaEnable::Possible))),
            ),
        );

        let notes = descrambler.take_notifications();
        let hits = notes
            .iter()
            .filter(|n| {
                matches!(
                    n,
                    Notification::Entitlement {
                        program_number: 1546,
                        ca_enable: CaEnable::Possible,
                        descrambling_ok: true,
                    }
                )
            })
            .count();
        assert_eq!(
            hits, 1,
            "expected exactly one Entitlement notification to surface via CaDescrambler::take_notifications(), got {notes:?}"
        );
    }

    #[test]
    fn add_service_delegates() {
        let d = Driver::new(MockCaDevice::new([]));
        let mut descrambler = descrambler_with(d, []);

        let pmt_bytes = build_clear_pmt_fixture(999);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();

        let err = descrambler.add_service(&pmt).unwrap_err();
        assert!(
            matches!(
                err,
                CaError::NoCaDescriptor {
                    program_number: 999
                }
            ),
            "expected CaError::NoCaDescriptor via delegation, got {err:?}"
        );
    }
}
