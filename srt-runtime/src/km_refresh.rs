//! SEK-rotation ("KM Refresh") driver — `draft-sharabayko-srt-01` §6.1.6 (KM
//! Refresh), curated at `specs/rules/srt-crypto.md` ("KM Refresh — §6.1.6").
//!
//! Sans-IO: [`KmRefreshDriver::on_packet_sent`] / [`KmRefreshDriver::tick`]
//! take a caller-supplied packet count — this crate never reads a wall clock
//! or a socket (the same contract as [`crate::handshake_sm`] / [`crate::arq`]
//! / [`crate::tsbpd`]). The driver only tracks *when* to rotate and *which*
//! parity is active; it does not generate, wrap, or send key material
//! itself — the actual SEK PRNG/wrap/Key-Material-message send is the
//! caller's job (mirroring [`crate::handshake_sm::CryptoConfig`]'s design:
//! this crate's sans-IO core never owns a CSPRNG), triggered by
//! [`KmRefreshEvent::PreAnnounce`].
//!
//! # Thresholds (§6.1.6, "Recommended values")
//!
//! - **KM Refresh Period = `2^25` packets**: how long a key stays active
//!   before switchover.
//! - **KM Pre-Announcement Period = `4000` packets**: how long *before*
//!   switchover the new key is announced, and — symmetrically — how long
//!   *after* switchover the old key stays valid before decommission. "Both
//!   keys are valid in parallel for `2 * Pre-Announcement Period`", to
//!   tolerate late/retransmitted packets that were encrypted under the old
//!   key.
//!
//! All three thresholds are measured from the start of the *current* active
//! key's epoch (packet 0 of that key):
//!
//! ```text
//! 0 ── refresh - pre_announce ── refresh ── refresh + pre_announce
//!      PreAnnounce fires         Switchover  Decommission fires
//!      (generate + wrap +       (old key     (old key dropped)
//!       ready next key)          still valid)
//! ```
//!
//! [`KmRefreshThresholds::RECOMMENDED`] is the spec's `2^25`/`4000` pair;
//! `tests/km_refresh.rs` drives the same state machine with a scaled-down
//! threshold pair so the test suite does not need `2^25` real iterations —
//! the state machine logic is identical, only the threshold constants
//! differ.

use alloc::vec::Vec;

/// Which of the two alternating SEKs (`draft-sharabayko-srt-01` §3.1's data
/// packet `KK` field / §6.1.6's odd/even alternation) is meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum KeyParity {
    /// The even-numbered SEK.
    Even,
    /// The odd-numbered SEK.
    Odd,
}

impl KeyParity {
    /// The other parity — every rotation alternates (§6.1.6).
    pub fn other(self) -> Self {
        match self {
            KeyParity::Even => KeyParity::Odd,
            KeyParity::Odd => KeyParity::Even,
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            KeyParity::Even => "even",
            KeyParity::Odd => "odd",
        }
    }
}

broadcast_common::impl_spec_display!(KeyParity);

/// KM Refresh packet-count thresholds (`draft-sharabayko-srt-01` §6.1.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KmRefreshThresholds {
    /// Packets an active key is used for before switchover (spec-recommended
    /// `2^25`).
    pub refresh_period: u64,
    /// Packets before switchover the next key is announced, and after
    /// switchover the old key stays valid (spec-recommended `4000`).
    pub pre_announcement_period: u64,
}

impl KmRefreshThresholds {
    /// The draft's own recommended values (§6.1.6): `2^25` packets refresh
    /// period, `4000` packets pre-announcement period.
    pub const RECOMMENDED: KmRefreshThresholds = KmRefreshThresholds {
        refresh_period: 1 << 25,
        pre_announcement_period: 4000,
    };
}

/// One state transition [`KmRefreshDriver::on_packet_sent`] /
/// [`KmRefreshDriver::tick`] can fire (`draft-sharabayko-srt-01` §6.1.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum KmRefreshEvent {
    /// `refresh_period - pre_announcement_period` packets sent under the
    /// active key: generate, wrap, and send a fresh SEK for `next_parity`
    /// now (§6.1.5's Key Material exchange, driven out-of-band of this
    /// driver — see the module doc).
    PreAnnounce {
        /// The parity the newly-generated key will use.
        next_parity: KeyParity,
    },
    /// `refresh_period` packets sent under the active key: start encrypting
    /// *new* outgoing packets with `new_active` from now on. The previous
    /// key remains valid for decrypting late/retransmitted packets until
    /// [`KmRefreshEvent::Decommission`] fires for it.
    Switchover {
        /// The parity now active for new packets.
        new_active: KeyParity,
    },
    /// `pre_announcement_period` packets after switchover: the previous key
    /// may be dropped — no more retransmits will need it (§6.1.6, "both keys
    /// valid in parallel for `2 * Pre-Announcement Period`").
    Decommission {
        /// The parity being retired.
        retired: KeyParity,
    },
}

/// Sans-IO SEK-rotation state machine (`draft-sharabayko-srt-01` §6.1.6).
///
/// Tracks which [`KeyParity`] is currently active and fires
/// [`KmRefreshEvent`]s as the packet count crosses the configured
/// [`KmRefreshThresholds`]. Does not hold key material itself — see the
/// module doc. Rotates indefinitely: once a cycle's [`KmRefreshEvent::Decommission`]
/// fires, the next cycle's thresholds are armed again against the new active
/// key's epoch.
#[derive(Debug, Clone, PartialEq)]
pub struct KmRefreshDriver {
    thresholds: KmRefreshThresholds,
    active: KeyParity,
    /// Packet count at which `active` most recently became active (`0` for
    /// the initial key negotiated at handshake time).
    epoch_start: u64,
    /// Total packets sent so far (monotonic).
    total_sent: u64,
    pre_announced: bool,
    switched_over: bool,
    decommissioned: bool,
}

impl KmRefreshDriver {
    /// A fresh driver. `initial_parity` is the SEK negotiated at handshake
    /// time (this crate's handshake convention is [`KeyParity::Even`] — see
    /// `crate::handshake_sm::build_key_material_extension`, `crypto` feature
    /// only).
    pub fn new(thresholds: KmRefreshThresholds, initial_parity: KeyParity) -> Self {
        KmRefreshDriver {
            thresholds,
            active: initial_parity,
            epoch_start: 0,
            total_sent: 0,
            pre_announced: false,
            switched_over: false,
            decommissioned: false,
        }
    }

    /// The currently-active parity for *new* outgoing packets.
    pub fn active_parity(&self) -> KeyParity {
        self.active
    }

    /// Total packets recorded via [`Self::on_packet_sent`]/[`Self::tick`].
    pub fn total_sent(&self) -> u64 {
        self.total_sent
    }

    /// Whether `parity`'s key should still be considered held/valid. The
    /// active key always is; the just-retired key is too, from switchover
    /// until decommission (§6.1.6's "both keys valid in parallel" transition
    /// window) — a data packet under either may legitimately arrive during
    /// that window (in-flight or retransmitted).
    pub fn is_key_valid(&self, parity: KeyParity) -> bool {
        if parity == self.active {
            return true;
        }
        self.switched_over && !self.decommissioned
    }

    /// Record `n` more packets sent under the active key and return any
    /// [`KmRefreshEvent`]s newly crossed, in spec order (PreAnnounce,
    /// Switchover, Decommission). A single call can fire more than one event
    /// if `n` is large enough to cross multiple thresholds at once.
    pub fn on_packet_sent(&mut self, n: u64) -> Vec<KmRefreshEvent> {
        self.total_sent = self.total_sent.saturating_add(n);
        let mut events = Vec::new();
        let since_epoch = self.total_sent.saturating_sub(self.epoch_start);

        let pre_announce_at = self
            .thresholds
            .refresh_period
            .saturating_sub(self.thresholds.pre_announcement_period);
        if !self.pre_announced && since_epoch >= pre_announce_at {
            self.pre_announced = true;
            events.push(KmRefreshEvent::PreAnnounce {
                next_parity: self.active.other(),
            });
        }

        if !self.switched_over && since_epoch >= self.thresholds.refresh_period {
            self.switched_over = true;
            let new_active = self.active.other();
            self.active = new_active;
            // The new epoch starts exactly at the switchover point, so the
            // decommission threshold below (measured from `epoch_start`) is
            // `pre_announcement_period` packets *after* switchover — §6.1.6's
            // `refresh_period + pre_announcement_period`, not
            // `2 * refresh_period + pre_announcement_period`.
            self.epoch_start += self.thresholds.refresh_period;
            events.push(KmRefreshEvent::Switchover { new_active });
        }

        if self.switched_over && !self.decommissioned {
            let since_switchover = self.total_sent.saturating_sub(self.epoch_start);
            if since_switchover >= self.thresholds.pre_announcement_period {
                self.decommissioned = true;
                events.push(KmRefreshEvent::Decommission {
                    retired: self.active.other(),
                });
                // Arm the next cycle: `epoch_start` already marks the
                // current key's start, so the next PreAnnounce/Switchover
                // are correctly measured from here.
                self.pre_announced = false;
                self.switched_over = false;
                self.decommissioned = false;
            }
        }

        events
    }

    /// Record exactly one packet sent — convenience wrapper over
    /// [`Self::on_packet_sent`].
    pub fn tick(&mut self) -> Vec<KmRefreshEvent> {
        self.on_packet_sent(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCALED: KmRefreshThresholds = KmRefreshThresholds {
        refresh_period: 100,
        pre_announcement_period: 10,
    };

    #[test]
    fn recommended_thresholds_match_spec_values() {
        assert_eq!(KmRefreshThresholds::RECOMMENDED.refresh_period, 1 << 25);
        assert_eq!(
            KmRefreshThresholds::RECOMMENDED.pre_announcement_period,
            4000
        );
    }

    #[test]
    fn key_parity_alternates_and_labels() {
        assert_eq!(KeyParity::Even.other(), KeyParity::Odd);
        assert_eq!(KeyParity::Odd.other(), KeyParity::Even);
        assert_eq!(KeyParity::Even.to_string(), "even");
        assert_eq!(KeyParity::Odd.to_string(), "odd");
    }

    #[test]
    fn fires_pre_announce_switchover_decommission_in_order() {
        let mut d = KmRefreshDriver::new(SCALED, KeyParity::Even);
        assert_eq!(d.active_parity(), KeyParity::Even);
        assert!(d.is_key_valid(KeyParity::Even));
        assert!(!d.is_key_valid(KeyParity::Odd));

        // Just before pre-announce: nothing fires.
        assert_eq!(d.on_packet_sent(89), Vec::new());
        assert_eq!(d.active_parity(), KeyParity::Even);

        // Crosses 90 (100 - 10): PreAnnounce for the *other* (Odd) parity.
        assert_eq!(
            d.on_packet_sent(1),
            alloc::vec![KmRefreshEvent::PreAnnounce {
                next_parity: KeyParity::Odd
            }]
        );
        // Active key hasn't changed yet.
        assert_eq!(d.active_parity(), KeyParity::Even);
        assert!(d.is_key_valid(KeyParity::Even));
        assert!(!d.is_key_valid(KeyParity::Odd));

        // Re-crossing the same threshold does not re-fire. (total: 91)
        assert_eq!(d.on_packet_sent(1), Vec::new());

        // Crosses 100: Switchover to Odd.
        assert_eq!(d.on_packet_sent(8), Vec::new()); // total 99, not yet
        assert_eq!(
            d.on_packet_sent(1), // total 100
            alloc::vec![KmRefreshEvent::Switchover {
                new_active: KeyParity::Odd
            }]
        );
        assert_eq!(d.active_parity(), KeyParity::Odd);
        // Both keys valid during the transition window.
        assert!(d.is_key_valid(KeyParity::Odd));
        assert!(d.is_key_valid(KeyParity::Even));

        // Crosses 110 (100 + 10): Decommission the old (Even) key.
        assert_eq!(d.on_packet_sent(9), Vec::new()); // total 109
        assert_eq!(
            d.on_packet_sent(1), // total 110
            alloc::vec![KmRefreshEvent::Decommission {
                retired: KeyParity::Even
            }]
        );
        assert_eq!(d.active_parity(), KeyParity::Odd);
        assert!(d.is_key_valid(KeyParity::Odd));
        assert!(!d.is_key_valid(KeyParity::Even), "old key must be dropped");
    }

    #[test]
    fn large_jump_fires_all_three_events_in_one_call() {
        let mut d = KmRefreshDriver::new(SCALED, KeyParity::Even);
        let events = d.on_packet_sent(115);
        assert_eq!(
            events,
            alloc::vec![
                KmRefreshEvent::PreAnnounce {
                    next_parity: KeyParity::Odd
                },
                KmRefreshEvent::Switchover {
                    new_active: KeyParity::Odd
                },
                KmRefreshEvent::Decommission {
                    retired: KeyParity::Even
                },
            ]
        );
        assert_eq!(d.active_parity(), KeyParity::Odd);
        assert!(!d.is_key_valid(KeyParity::Even));
    }

    #[test]
    fn tick_is_a_single_packet_and_rotation_repeats_forever() {
        let mut d = KmRefreshDriver::new(SCALED, KeyParity::Even);
        let mut all = Vec::new();
        for _ in 0..250 {
            all.extend(d.tick());
        }
        assert_eq!(d.total_sent(), 250);
        // Two full cycles of 100 packets each fit in 250 ticks: expect two
        // full PreAnnounce/Switchover/Decommission triples, alternating
        // parity, plus one more PreAnnounce (at 90 packets into the third
        // 100-packet cycle, i.e. total 290 — not reached at 250) which does
        // NOT fire yet.
        let switchovers: Vec<_> = all
            .iter()
            .filter(|e| matches!(e, KmRefreshEvent::Switchover { .. }))
            .collect();
        assert_eq!(switchovers.len(), 2);
        assert_eq!(
            switchovers[0],
            &KmRefreshEvent::Switchover {
                new_active: KeyParity::Odd
            }
        );
        assert_eq!(
            switchovers[1],
            &KmRefreshEvent::Switchover {
                new_active: KeyParity::Even
            }
        );
        assert_eq!(d.active_parity(), KeyParity::Even);
    }
}
