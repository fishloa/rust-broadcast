//! Integration test: the §6.1.6 KM Refresh (SEK-rotation) driver,
//! `srt_runtime::km_refresh` (issue #621).
//!
//! `draft-sharabayko-srt-01` §6.1.6 recommends a KM Refresh Period of `2^25`
//! packets and a KM Pre-Announcement Period of `4000` packets — driving a
//! real [`srt_runtime::km_refresh::KmRefreshDriver`] through `2^25` real
//! `on_packet_sent` calls one at a time would work (the driver is O(1) per
//! call) but is unnecessary CPU/test-time for what is purely a threshold
//! state machine; [`srt_runtime::km_refresh::KmRefreshDriver::on_packet_sent`]
//! also accepts a batched packet count, so this test instead **jumps
//! straight to just past each threshold** with the real, spec-recommended
//! [`srt_runtime::km_refresh::KmRefreshThresholds::RECOMMENDED`] constants
//! (`2^25` / `4000`) — no scaled-down threshold is needed at all, since the
//! state machine only does O(1) arithmetic regardless of how large a jump it
//! is asked to cross in one call. This exercises the *exact* production
//! thresholds, not a stand-in.

#![cfg(feature = "crypto")]

use srt_runtime::km_refresh::{KeyParity, KmRefreshDriver, KmRefreshEvent, KmRefreshThresholds};

const REFRESH_PERIOD: u64 = KmRefreshThresholds::RECOMMENDED.refresh_period; // 2^25
const PRE_ANNOUNCE: u64 = KmRefreshThresholds::RECOMMENDED.pre_announcement_period; // 4000

#[test]
fn recommended_thresholds_are_the_spec_values() {
    assert_eq!(REFRESH_PERIOD, 1u64 << 25);
    assert_eq!(PRE_ANNOUNCE, 4000);
}

#[test]
fn pre_announce_fires_at_refresh_minus_pre_announce_packets() {
    let mut driver = KmRefreshDriver::new(KmRefreshThresholds::RECOMMENDED, KeyParity::Even);
    assert_eq!(driver.active_parity(), KeyParity::Even);

    // One packet short of the threshold: nothing fires yet.
    let short_of_threshold = REFRESH_PERIOD - PRE_ANNOUNCE - 1;
    assert_eq!(driver.on_packet_sent(short_of_threshold), Vec::new());
    assert_eq!(driver.active_parity(), KeyParity::Even);

    // One more packet crosses `2^25 - 4000`: PreAnnounce for the *other*
    // (Odd) parity — the new key must be generated/wrapped/sent now, but the
    // active key for new traffic has not changed yet.
    let events = driver.on_packet_sent(1);
    assert_eq!(
        events,
        vec![KmRefreshEvent::PreAnnounce {
            next_parity: KeyParity::Odd
        }]
    );
    assert_eq!(driver.active_parity(), KeyParity::Even);
    assert!(driver.is_key_valid(KeyParity::Even));
    assert!(!driver.is_key_valid(KeyParity::Odd));
}

#[test]
fn switchover_fires_at_refresh_period_and_changes_the_active_key() {
    let mut driver = KmRefreshDriver::new(KmRefreshThresholds::RECOMMENDED, KeyParity::Even);

    // Jump straight past pre-announce to one packet short of switchover.
    let events = driver.on_packet_sent(REFRESH_PERIOD - 1);
    assert_eq!(
        events,
        vec![KmRefreshEvent::PreAnnounce {
            next_parity: KeyParity::Odd
        }],
        "pre-announce must already have fired on the way here"
    );
    assert_eq!(
        driver.active_parity(),
        KeyParity::Even,
        "not yet switched over"
    );

    // The `2^25`th packet: Switchover. New outgoing traffic now uses Odd;
    // Even (just retired) is still valid for late/retransmitted packets.
    let events = driver.on_packet_sent(1);
    assert_eq!(
        events,
        vec![KmRefreshEvent::Switchover {
            new_active: KeyParity::Odd
        }]
    );
    assert_eq!(driver.active_parity(), KeyParity::Odd);
    assert!(driver.is_key_valid(KeyParity::Odd), "new key is active");
    assert!(
        driver.is_key_valid(KeyParity::Even),
        "old key must still be valid immediately after switchover"
    );
}

#[test]
fn decommission_fires_at_refresh_plus_pre_announce_and_drops_the_old_key() {
    let mut driver = KmRefreshDriver::new(KmRefreshThresholds::RECOMMENDED, KeyParity::Even);

    // Drive to one packet short of decommission (`2^25 + 4000 - 1`).
    let events = driver.on_packet_sent(REFRESH_PERIOD + PRE_ANNOUNCE - 1);
    assert!(
        events.contains(&KmRefreshEvent::Switchover {
            new_active: KeyParity::Odd
        }),
        "switchover must already have fired: {events:?}"
    );
    assert!(
        driver.is_key_valid(KeyParity::Even),
        "old key still valid one packet before decommission"
    );

    // The last packet of the `2 * Pre-Announcement Period` transition
    // window: Decommission. The old (Even) key is dropped.
    let events = driver.on_packet_sent(1);
    assert_eq!(
        events,
        vec![KmRefreshEvent::Decommission {
            retired: KeyParity::Even
        }]
    );
    assert_eq!(driver.active_parity(), KeyParity::Odd);
    assert!(
        !driver.is_key_valid(KeyParity::Even),
        "old key must no longer be valid/held after decommission"
    );
    assert!(driver.is_key_valid(KeyParity::Odd));
}

#[test]
fn one_call_spanning_the_whole_recommended_window_fires_all_three_events_in_order() {
    let mut driver = KmRefreshDriver::new(KmRefreshThresholds::RECOMMENDED, KeyParity::Even);
    let events = driver.on_packet_sent(REFRESH_PERIOD + PRE_ANNOUNCE);
    assert_eq!(
        events,
        vec![
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
    assert_eq!(driver.active_parity(), KeyParity::Odd);
    assert!(!driver.is_key_valid(KeyParity::Even));
}

#[test]
fn rotation_continues_into_a_second_cycle_with_the_real_thresholds() {
    let mut driver = KmRefreshDriver::new(KmRefreshThresholds::RECOMMENDED, KeyParity::Even);

    // First full cycle: Even -> Odd.
    let events = driver.on_packet_sent(REFRESH_PERIOD + PRE_ANNOUNCE);
    assert!(events.contains(&KmRefreshEvent::Switchover {
        new_active: KeyParity::Odd
    }));
    assert_eq!(driver.active_parity(), KeyParity::Odd);

    // Second full cycle, measured from the new epoch: Odd -> Even again.
    let events = driver.on_packet_sent(REFRESH_PERIOD + PRE_ANNOUNCE);
    assert_eq!(
        events,
        vec![
            KmRefreshEvent::PreAnnounce {
                next_parity: KeyParity::Even
            },
            KmRefreshEvent::Switchover {
                new_active: KeyParity::Even
            },
            KmRefreshEvent::Decommission {
                retired: KeyParity::Odd
            },
        ]
    );
    assert_eq!(driver.active_parity(), KeyParity::Even);
    assert_eq!(driver.total_sent(), 2 * (REFRESH_PERIOD + PRE_ANNOUNCE));
}
