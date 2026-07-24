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
//! A CI slot descrambles the single TS routed to it, and only needs three PID
//! classes out of that TS: the target services' **ES PIDs**
//! ([`descramble_pids`](crate::driver::Driver::descramble_pids)), their
//! **ECM PIDs** ([`ca_pids`](crate::driver::Driver::ca_pids) — ISO/IEC
//! 13818-1 §2.6.16 `CA_descriptor` `CA_PID`, carrying the control words
//! without which the module has ES to descramble but no key to do it with),
//! and the **EMM PIDs** ([`emm_pids`](crate::driver::Driver::emm_pids) —
//! entitlements). [`CaDescrambler::feed_ts`] filters the input TS to
//! [`required_pids`](CaDescrambler::required_pids) = `descramble_pids ∪
//! ca_pids ∪ emm_pids` and writes only those packets to `ci0` — a 30–50
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

use std::collections::BTreeSet;
use std::io;
use std::time::Duration;

use dvb_si::tables::cat::CatSection;
use dvb_si::tables::pmt::PmtSection;

use crate::dataplane::{CiDataDevice, TS_PACKET_LEN};
use crate::device::CaDevice;
use crate::driver::Driver;
use crate::event::Notification;
use crate::managed::CaError;

/// MPEG-2 TS sync byte (ISO/IEC 13818-1 §2.4.3.2).
const TS_SYNC_BYTE: u8 = 0x47;
/// Mask for the PID's upper byte within a TS packet header (byte 1): the top
/// 3 bits are `transport_error_indicator`/`payload_unit_start_indicator`/
/// `transport_priority`, the low 5 are `PID[12:8]`.
const TS_PID_HIGH_MASK: u8 = 0x1F;

/// The 13-bit PID carried by one 188-byte TS packet's header (bytes 1–2),
/// masking off the non-PID flag bits in byte 1.
fn packet_pid(packet: &[u8]) -> u16 {
    (u16::from(packet[1] & TS_PID_HIGH_MASK) << 8) | u16::from(packet[2])
}

/// Keep only the packets in `scrambled` whose PID is in `allow`, concatenated
/// in order.
///
/// # Errors
/// [`io::ErrorKind::InvalidInput`] if `scrambled` is not a whole number of
/// [`TS_PACKET_LEN`]-byte packets, or if any packet's sync byte isn't `0x47`
/// (misaligned input — filtering garbage would silently corrupt the PID
/// read).
fn filter_ts(scrambled: &[u8], allow: &BTreeSet<u16>) -> io::Result<Vec<u8>> {
    if scrambled.len() % TS_PACKET_LEN != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "scrambled TS is not a whole number of 188-byte packets",
        ));
    }
    let mut out = Vec::new();
    for packet in scrambled.chunks_exact(TS_PACKET_LEN) {
        if packet[0] != TS_SYNC_BYTE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "TS packet sync byte != 0x47 (misaligned input)",
            ));
        }
        if allow.contains(&packet_pid(packet)) {
            out.extend_from_slice(packet);
        }
    }
    Ok(out)
}

/// Turnkey CAS descrambler (#763 Layer 2): a [`Driver`] (control plane, `caM`)
/// paired with a [`CiDataDevice`] (data plane, `ciM`) for one CI slot. See the
/// module docs for the feed-filter policy.
pub struct CaDescrambler<D: CaDevice, C: CiDataDevice> {
    driver: Driver<D>,
    ci: C,
}

impl<D: CaDevice, C: CiDataDevice> CaDescrambler<D, C> {
    /// New descrambler over an already-constructed control-plane `driver` and
    /// data-plane `ci` device (both left for the caller to `init()`/wire up
    /// as needed before use).
    #[must_use]
    pub fn new(driver: Driver<D>, ci: C) -> Self {
        Self { driver, ci }
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
    /// # Errors
    /// [`io::ErrorKind::InvalidInput`] if `scrambled` is not a whole number
    /// of [`TS_PACKET_LEN`]-byte packets or carries a misaligned packet
    /// (sync byte != `0x47`); otherwise any I/O error from the underlying
    /// [`CiDataDevice`].
    pub fn feed_ts(&mut self, scrambled: &[u8]) -> io::Result<Vec<u8>> {
        let allow: BTreeSet<u16> = self.required_pids().into_iter().collect();
        let kept = filter_ts(scrambled, &allow)?;
        if !kept.is_empty() {
            self.ci.write(&kept)?;
        }

        let mut out = Vec::new();
        let mut buf = [0u8; 32 * TS_PACKET_LEN];
        loop {
            let n = self.ci.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        Ok(out)
    }

    /// `descramble_pids ∪ ca_pids ∪ emm_pids` — the PIDs the CAM needs on
    /// `ci0` (delegates to [`Driver::required_pids`]).
    #[must_use]
    pub fn required_pids(&self) -> Vec<u16> {
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
    use crate::dataplane::MockCiDataDevice;
    use crate::device::MockCaDevice;
    use crate::driver::tests::{
        CA_SESSION, build_ca_pmt_fixture, build_cat_fixture, build_clear_pmt_fixture,
        ca_descriptor, ca_pmt_reply_for, driver_with_sessions, feed, r_apdu, ser,
    };
    use crate::managed::CaError;
    use broadcast_common::Parse;

    fn packet(pid: u16, fill: u8) -> Vec<u8> {
        let mut p = vec![fill; TS_PACKET_LEN];
        p[0] = TS_SYNC_BYTE;
        // Set the reserved top bits (transport_error/pusi/priority) to
        // exercise the mask, per the brief's `hi = 0x40 | (pid>>8)`.
        p[1] = 0x40 | ((pid >> 8) as u8);
        p[2] = pid as u8;
        p
    }

    #[test]
    fn filter_ts_keeps_only_allowed_pids() {
        let p_100 = packet(0x100, 0xAA);
        let p_64 = packet(0x64, 0xBB);
        let p_200 = packet(0x200, 0xCC);

        let mut scrambled = Vec::new();
        scrambled.extend_from_slice(&p_100);
        scrambled.extend_from_slice(&p_64);
        scrambled.extend_from_slice(&p_200);

        let allow: BTreeSet<u16> = [0x100, 0x64].into_iter().collect();
        let kept = filter_ts(&scrambled, &allow).unwrap();

        let mut expected = Vec::new();
        expected.extend_from_slice(&p_100);
        expected.extend_from_slice(&p_64);
        assert_eq!(
            kept, expected,
            "0x200 must be dropped, the two allowed packets kept byte-exact and in order"
        );

        // Bite: an empty allow-set drops everything — a reintroduced
        // no-filter passthrough would keep 0x200 and fail this.
        let empty: BTreeSet<u16> = BTreeSet::new();
        assert!(filter_ts(&scrambled, &empty).unwrap().is_empty());

        // Unaligned input.
        assert_eq!(
            filter_ts(&scrambled[..scrambled.len() - 1], &allow)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );

        // Misaligned sync byte.
        let mut bad = p_100.clone();
        bad[0] = 0x00;
        assert_eq!(
            filter_ts(&bad, &allow).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
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
                    ca_system_ids: vec![0x0648],
                }),
            ),
        );
        d.take_notifications();
        let mut descriptors = Vec::new();
        descriptors.extend_from_slice(&ca_descriptor(0x0648, 0x1FF0));
        let cat_bytes = build_cat_fixture(&descriptors);
        let cat = CatSection::parse(&cat_bytes).unwrap();
        d.set_cat(&cat).unwrap();

        assert_eq!(
            d.required_pids(),
            vec![0x0064, 0x0065, 0x0100, 0x0101, 0x1FF0],
            "precondition: required_pids = descramble_pids ∪ ca_pids ∪ emm_pids"
        );

        let descrambled_script = packet(0x100, 0xEE);
        let mut descrambler = descrambler_with(d, [descrambled_script.clone()]);
        assert_eq!(
            descrambler.required_pids(),
            vec![0x0064, 0x0065, 0x0100, 0x0101, 0x1FF0],
            "required_pids must delegate through the wrapper"
        );

        // One required-PID packet (0x100, an ES PID) + one junk packet on a
        // PID NOT in required_pids.
        let required_pkt = packet(0x100, 0x11);
        let junk_pkt = packet(0x999, 0x22);
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
