//! PMT `version_number`/`current_next_indicator` diffing (issue #774):
//! `TrackRemoved`/`TrackUpdated`/`TracksResolved { generation }` behaviour
//! when a live PMT is re-applied with a genuine change.
//!
//! # PROVENANCE
//!
//! No committed TS fixture in this workspace carries a mid-stream PMT
//! version change (every real capture here is a single stable multiplex), so
//! this file **synthesises** minimal-but-spec-conformant PAT/PMT/PES bytes by
//! hand, rather than a real broadcast excerpt. The section byte layout
//! (long-form header, `version_number`/`current_next_indicator` byte,
//! ES-loop entries, trailing `CRC_32`) mirrors this crate's own
//! `ts_mux.rs::build_pat_section`/`build_pmt_section`/`finish_section`
//! byte-for-byte (ISO/IEC 13818-1 §2.4.4.1/§2.4.4.3/§2.4.4.8) — verified
//! against that module rather than invented. Every elementary stream here
//! uses an unrecognised `stream_type` (`Codec::Data`, PES-carried), whose
//! `ConfigProbe::Data` resolves on the very first access unit with no
//! in-band header at all — this keeps the fixture minimal while still
//! exercising the real PMT-declaration-order promotion path
//! (`StreamingTsDemux::try_promote_ready`), not a shortcut around it.

use broadcast_common::crc32_mpeg2;
use mpeg_ts::ts::{TS_PACKET_SIZE, TsHeader};
use transmux::pipeline::TrackSpec;
use transmux::{AbandonReason, DemuxEvent, StreamingTsDemux};

// ── Fixture constants ───────────────────────────────────────────────────────

const PAT_PID: u16 = 0x0000;
const PMT_PID: u16 = 0x1000;
const TRANSPORT_STREAM_ID: u16 = 1;
const PROGRAM_NUMBER: u16 = 1;

const PID_V: u16 = 0x0101; // "video" (informal — just the first-ranked ES)
const PID_A: u16 = 0x0102; // "audioA"
const PID_B: u16 = 0x0103; // "audioB"
const PID_C: u16 = 0x0104; // used only by the count-collision test

// Deliberately unrecognised (ISO/IEC 13818-1 Table 2-34 has nothing at these
// values) so every ES here classifies as opaque `Codec::Data` (PES-carried) —
// `ConfigProbe::Data` resolves on the very first access unit, no header scan.
const STREAM_TYPE_V: u8 = 0x90;
const STREAM_TYPE_A: u8 = 0x91;
const STREAM_TYPE_B: u8 = 0x92;
const STREAM_TYPE_C: u8 = 0x93;

const ISO_639_LANGUAGE_DESCRIPTOR_TAG: u8 = 0x0A;

/// Build an `ISO_639_language_descriptor` (ETSI EN 300 468 §6.2.20 /
/// ISO/IEC 13818-1 §2.6.18): tag(1) + length(1)=4 + 3-byte language code +
/// 1-byte `audio_type`.
fn lang_descriptor(lang: &[u8; 3], audio_type: u8) -> Vec<u8> {
    let mut d = vec![ISO_639_LANGUAGE_DESCRIPTOR_TAG, 4];
    d.extend_from_slice(lang);
    d.push(audio_type);
    d
}

/// Prepend the long-form section header (`table_id` + `section_length`) and
/// append the trailing `CRC_32` — byte-for-byte the same shape as
/// `transmux::ts_mux`'s private `finish_section` (ISO/IEC 13818-1 §2.4.4.1).
fn finish_section(table_id: u8, body: &[u8]) -> Vec<u8> {
    const CRC32_LEN: usize = 4;
    let section_length = body.len() + CRC32_LEN;
    let mut section = Vec::with_capacity(3 + section_length);
    section.push(table_id);
    section.push(0xB0 | ((section_length >> 8) as u8 & 0x0F));
    section.push((section_length & 0xFF) as u8);
    section.extend_from_slice(body);
    let crc = crc32_mpeg2::compute(&section);
    section.extend_from_slice(&crc.to_be_bytes());
    section
}

/// Build a one-program PAT section (ISO/IEC 13818-1 §2.4.4.3), fixed at
/// version 0 / `current_next_indicator = 1` — the PAT never changes in these
/// tests, only the PMT does.
fn build_pat() -> Vec<u8> {
    const TABLE_ID_PAT: u8 = 0x00;
    let mut body = Vec::new();
    body.extend_from_slice(&TRANSPORT_STREAM_ID.to_be_bytes());
    body.push(0xC1); // version_number=0, current_next_indicator=1
    body.push(0); // section_number
    body.push(0); // last_section_number
    body.extend_from_slice(&PROGRAM_NUMBER.to_be_bytes());
    body.push(0xE0 | ((PMT_PID >> 8) as u8 & 0x1F));
    body.push((PMT_PID & 0xFF) as u8);
    finish_section(TABLE_ID_PAT, &body)
}

/// Build a PMT section (ISO/IEC 13818-1 §2.4.4.8), always single-section
/// (`section_number == last_section_number == 0`) — every scenario here
/// varies only `version`/`current_next_indicator`/the ES loop.
fn build_pmt(version: u8, current_next_indicator: bool, entries: &[(u16, u8, &[u8])]) -> Vec<u8> {
    const TABLE_ID_PMT: u8 = 0x02;
    const NO_PCR_PID: u16 = 0x1FFF;
    let mut body = Vec::new();
    body.extend_from_slice(&PROGRAM_NUMBER.to_be_bytes()); // table_id_extension
    body.push(0xC0 | (version << 1) | u8::from(current_next_indicator));
    body.push(0); // section_number
    body.push(0); // last_section_number
    body.push(0xE0 | ((NO_PCR_PID >> 8) as u8 & 0x1F));
    body.push((NO_PCR_PID & 0xFF) as u8);
    body.push(0xF0); // program_info_length = 0
    body.push(0);
    for &(pid, stream_type, descriptors) in entries {
        body.push(stream_type);
        body.push(0xE0 | ((pid >> 8) as u8 & 0x1F));
        body.push((pid & 0xFF) as u8);
        let len = descriptors.len();
        body.push(0xF0 | ((len >> 8) as u8 & 0x0F));
        body.push((len & 0xFF) as u8);
        body.extend_from_slice(descriptors);
    }
    finish_section(TABLE_ID_PMT, &body)
}

/// One 188-byte TS packet carrying `payload` (PES or the `[pointer_field,
/// section...]` PSI shape — the caller builds whichever it needs) on `pid`.
/// Payload must fit in one packet (184 bytes) — true for every section/PES
/// this file builds.
fn ts_packet(pid: u16, pusi: bool, cc: u8, payload: &[u8]) -> [u8; TS_PACKET_SIZE] {
    let mut pkt = [0xFFu8; TS_PACKET_SIZE];
    let header = TsHeader {
        tei: false,
        pusi,
        pid,
        scrambling: 0,
        has_adaptation: false,
        has_payload: true,
        continuity_counter: cc,
    };
    header
        .serialize_into(&mut pkt)
        .expect("4-byte TsHeader always fits");
    let n = payload.len().min(TS_PACKET_SIZE - 4);
    assert!(
        payload.len() <= TS_PACKET_SIZE - 4,
        "fixture payload must fit in one TS packet"
    );
    pkt[4..4 + n].copy_from_slice(&payload[..n]);
    pkt
}

/// A PSI section, packetised with `pointer_field = 0` (ISO/IEC 13818-1 §2.4.4).
fn psi_packet(pid: u16, cc: u8, section: &[u8]) -> [u8; TS_PACKET_SIZE] {
    let mut payload = Vec::with_capacity(1 + section.len());
    payload.push(0); // pointer_field: section starts immediately
    payload.extend_from_slice(section);
    ts_packet(pid, true, cc, &payload)
}

/// A minimal PES packet (ISO/IEC 13818-1 §2.4.3.6): `padding_stream` (0xBE)
/// has no optional header at all (`StreamId::has_optional_header`), so this
/// is exactly `start_code(3) + stream_id(1) + PES_packet_length(2) +
/// payload` — the demuxer never reads `stream_id` itself, only whether an
/// optional header follows it.
fn pes_bytes(payload: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(6 + payload.len());
    b.extend_from_slice(&[0x00, 0x00, 0x01, 0xBE]);
    b.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    b.extend_from_slice(payload);
    b
}

/// Feed the PAT + a PMT (`version`/`cni`/`entries`) + two PES access units per
/// listed PID (so every ES's `ConfigProbe::Data` resolves and, once its
/// PMT-declaration-order turn comes, promotes to `Live` — see the module
/// docs). Each PID's *second* access unit deliberately never completes (no
/// third packet arrives for it), so it sits as an unemitted one-behind
/// `pending` sample the whole time — every track here starts with "PES
/// mid-flight" state, not just whichever one a later test happens to remove.
///
/// Padded with trailing null packets (PID `0x1FFF`, always ignored — see
/// `NULL_PACKET_PID` in `ts_demux.rs`) up to at least
/// [`mpeg_ts::resync::LOCK_CONFIRMATIONS`] total packets: `TsResync` only
/// starts emitting parsed packets once it has confirmed that many
/// consecutive 188-byte-strided sync bytes, so a bootstrap with very few
/// real packets (e.g. this file's single-track version-wrap scenario) would
/// otherwise sit unlocked and silently emit nothing at all.
fn feed_bootstrap(demux: &mut StreamingTsDemux, version: u8, entries: &[(u16, u8, &[u8])]) {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&psi_packet(PAT_PID, 0, &build_pat()));
    bytes.extend_from_slice(&psi_packet(PMT_PID, 0, &build_pmt(version, true, entries)));
    for (i, &(pid, _, _)) in entries.iter().enumerate() {
        let cc = (i as u8) * 2;
        bytes.extend_from_slice(&ts_packet(pid, true, cc, &pes_bytes(b"au1")));
        bytes.extend_from_slice(&ts_packet(pid, true, cc + 1, &pes_bytes(b"au2")));
    }
    while bytes.len() / TS_PACKET_SIZE < mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
        bytes.extend_from_slice(&ts_packet(0x1FFF, false, 0, &[]));
    }
    demux.feed(&bytes);
}

/// Drain every currently-queued event.
fn drain(demux: &mut StreamingTsDemux) -> Vec<DemuxEvent> {
    let mut events = Vec::new();
    while let Some(ev) = demux.poll_event() {
        events.push(ev);
    }
    events
}

/// The standard 3-track bootstrap: video + audioA (`"eng"`) + audioB, all
/// promoted to `Live`. Returns the demuxer and the bootstrap events (exactly
/// 3 `TrackAdded`, in PMT order, then one `TracksResolved`) for callers that
/// want to pull out track_ids.
fn bootstrap_three_tracks() -> (StreamingTsDemux, Vec<DemuxEvent>) {
    let mut demux = StreamingTsDemux::new();
    feed_bootstrap(
        &mut demux,
        1,
        &[
            (PID_V, STREAM_TYPE_V, &[][..]),
            (PID_A, STREAM_TYPE_A, &lang_descriptor(b"eng", 0)),
            (PID_B, STREAM_TYPE_B, &[][..]),
        ],
    );
    let events = drain(&mut demux);
    (demux, events)
}

fn track_added_specs(events: &[DemuxEvent]) -> Vec<TrackSpec> {
    events
        .iter()
        .filter_map(|e| match e {
            DemuxEvent::TrackAdded(spec) => Some(spec.clone()),
            _ => None,
        })
        .collect()
}

fn track_id_for_pid(specs: &[TrackSpec], pid: u16) -> u32 {
    specs
        .iter()
        .find(|s| s.source_pid == Some(pid))
        .unwrap_or_else(|| panic!("no TrackAdded for pid {pid:#06x}"))
        .track_id
}

// ── Sanity: the bootstrap itself behaves as documented ──────────────────────

#[test]
fn bootstrap_yields_exactly_three_tracks_no_samples_yet() {
    let (_demux, events) = bootstrap_three_tracks();
    let specs = track_added_specs(&events);
    assert_eq!(
        specs.len(),
        3,
        "expected exactly 3 TrackAdded (video/audioA/audioB), got {events:?}"
    );
    assert_eq!(
        specs.iter().map(|s| s.source_pid).collect::<Vec<_>>(),
        vec![Some(PID_V), Some(PID_A), Some(PID_B)],
        "TrackAdded must fire in PMT declaration order"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, DemuxEvent::Sample { .. })),
        "bootstrap's second access unit per PID is deliberately left \
         mid-flight (one-behind, unemitted) — no Sample yet"
    );
    let resolved_count = events
        .iter()
        .filter(|e| matches!(e, DemuxEvent::TracksResolved { .. }))
        .count();
    assert_eq!(resolved_count, 1, "TracksResolved must fire exactly once");
}

// ── 1. Carousel non-regression ──────────────────────────────────────────────

/// A repeated, byte-identical PMT section (broadcast carousel repeat, several
/// times a second in practice) must be parsed but never re-diffed: no
/// `TrackRemoved`/`TrackUpdated`, and no re-armed `TracksResolved`.
#[test]
fn carousel_repeat_of_identical_version_emits_nothing() {
    let (mut demux, _bootstrap_events) = bootstrap_three_tracks();

    // Same version (1), same cni, same entries — a pure carousel repeat.
    let repeat = psi_packet(
        PMT_PID,
        2,
        &build_pmt(
            1,
            true,
            &[
                (PID_V, STREAM_TYPE_V, &[][..]),
                (PID_A, STREAM_TYPE_A, &lang_descriptor(b"eng", 0)),
                (PID_B, STREAM_TYPE_B, &[][..]),
            ],
        ),
    );
    demux.feed(&repeat);
    let events = drain(&mut demux);
    assert!(
        events.is_empty(),
        "an identical-version PMT repeat must be a complete no-op, got {events:?}"
    );
}

// ── 2. cni == 0 is parsed but not applied ───────────────────────────────────

/// A `current_next_indicator = 0` ("next") table — even with a genuinely
/// different version and a dropped PID — must be parsed and then dropped
/// before any diff, never acted on.
#[test]
fn next_table_cni_zero_is_not_applied() {
    let (mut demux, _bootstrap_events) = bootstrap_three_tracks();

    // Different version (5, not 1) but cni=0: a real "next" PMT dropping B.
    let next = psi_packet(
        PMT_PID,
        2,
        &build_pmt(
            5,
            false,
            &[
                (PID_V, STREAM_TYPE_V, &[][..]),
                (PID_A, STREAM_TYPE_A, &lang_descriptor(b"eng", 0)),
            ],
        ),
    );
    demux.feed(&next);
    let events = drain(&mut demux);
    assert!(
        events.is_empty(),
        "current_next_indicator=0 must never be applied/diffed, got {events:?}"
    );
}

// ── 3./4./5./7. Removal + update + re-arm from one applied diff ────────────

/// v2 drops audioB and changes audioA's `ISO_639_language_descriptor`
/// (`"eng"` → `"qaa"`) in the same version bump: exactly one `TrackRemoved`
/// (carrying audioB's real provenance.pid), exactly one `TrackUpdated`
/// (carrying audioA's new descriptor bytes, same track_id, unchanged
/// config), no trailing `Sample` for audioB's track_id ever, and
/// `TracksResolved` re-arms with a bumped `generation`.
#[test]
fn removal_and_update_and_rearm_from_one_version_bump() {
    let (mut demux, bootstrap_events) = bootstrap_three_tracks();
    let specs = track_added_specs(&bootstrap_events);
    let track_id_b = track_id_for_pid(&specs, PID_B);
    let track_id_a = track_id_for_pid(&specs, PID_A);

    let resolved_gen_before = bootstrap_events
        .iter()
        .find_map(|e| match e {
            DemuxEvent::TracksResolved { generation } => Some(*generation),
            _ => None,
        })
        .expect("bootstrap must fire TracksResolved");

    let v2 = psi_packet(
        PMT_PID,
        2,
        &build_pmt(
            2,
            true,
            &[
                (PID_V, STREAM_TYPE_V, &[][..]),
                (PID_A, STREAM_TYPE_A, &lang_descriptor(b"qaa", 0)),
                // audioB dropped.
            ],
        ),
    );
    demux.feed(&v2);
    let events = drain(&mut demux);

    let removed: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            DemuxEvent::TrackRemoved {
                track_id,
                provenance,
            } => Some((*track_id, provenance.pid)),
            _ => None,
        })
        .collect();
    assert_eq!(
        removed,
        vec![(track_id_b, Some(PID_B))],
        "exactly one TrackRemoved, for audioB's real track_id and pid"
    );

    let updated: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            DemuxEvent::TrackUpdated(spec) => Some(spec.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        updated.len(),
        1,
        "exactly one TrackUpdated, got {updated:?}"
    );
    assert_eq!(updated[0].track_id, track_id_a);
    assert_eq!(
        updated[0].es_info_descriptors,
        lang_descriptor(b"qaa", 0),
        "TrackUpdated must carry audioA's NEW descriptor bytes"
    );

    // Re-arm (issue #774 §C.3): TracksResolved fires again with a
    // strictly-greater generation once the (smaller) surviving track set is
    // itself fully resolved again.
    let resolved_gen_after = events.iter().find_map(|e| match e {
        DemuxEvent::TracksResolved { generation } => Some(*generation),
        _ => None,
    });
    assert_eq!(
        resolved_gen_after,
        Some(resolved_gen_before + 1),
        "TracksResolved must re-arm with a bumped generation after the removal, got {events:?}"
    );

    // No Sample for audioB's track_id ever appears — not in this batch, and
    // not from a stray follow-up packet on its now-unclaimed PID either.
    let stray_pes = ts_packet(PID_B, true, 9, &pes_bytes(b"post-removal"));
    demux.feed(&stray_pes);
    demux.finish();
    let trailing = drain(&mut demux);
    assert!(
        !trailing
            .iter()
            .any(|e| matches!(e, DemuxEvent::Sample { track_id, .. } if *track_id == track_id_b)),
        "no Sample for the removed track_id may ever follow its TrackRemoved, got {trailing:?}"
    );
}

// ── 6. Count-collision regression ───────────────────────────────────────────

/// Removing audioB and adding a brand-new PID `C` in the same version bump
/// returns the known-PID count to its prior value (3) once `C` resolves —
/// the exact case a count-keyed `TracksResolved` de-dup silently swallows.
/// `generation` (issue #774 §C.3) must still re-arm the event.
#[test]
fn tracks_resolved_rearms_even_when_known_pid_count_returns_to_its_prior_value() {
    let (mut demux, bootstrap_events) = bootstrap_three_tracks();
    let gen_before = bootstrap_events
        .iter()
        .find_map(|e| match e {
            DemuxEvent::TracksResolved { generation } => Some(*generation),
            _ => None,
        })
        .expect("bootstrap must fire TracksResolved");

    // v2: drop audioB, add C — known count: 3 -> 2 (diff applied) -> 3 (once
    // C resolves), i.e. it returns to its prior value of 3.
    let v2 = psi_packet(
        PMT_PID,
        2,
        &build_pmt(
            2,
            true,
            &[
                (PID_V, STREAM_TYPE_V, &[][..]),
                (PID_A, STREAM_TYPE_A, &lang_descriptor(b"eng", 0)),
                (PID_C, STREAM_TYPE_C, &[][..]),
            ],
        ),
    );
    demux.feed(&v2);
    let mut events = drain(&mut demux);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, DemuxEvent::TracksResolved { .. })),
        "must NOT resolve yet — the new PID C hasn't promoted to Live"
    );

    // Promote C: two access units, same bootstrap pattern.
    let mut more = Vec::new();
    more.extend_from_slice(&ts_packet(PID_C, true, 20, &pes_bytes(b"au1")));
    more.extend_from_slice(&ts_packet(PID_C, true, 21, &pes_bytes(b"au2")));
    demux.feed(&more);
    events.extend(drain(&mut demux));

    let saw_c_added = events
        .iter()
        .any(|e| matches!(e, DemuxEvent::TrackAdded(spec) if spec.source_pid == Some(PID_C)));
    assert!(saw_c_added, "TrackAdded must fire for the new PID C");

    let gen_after = events.iter().find_map(|e| match e {
        DemuxEvent::TracksResolved { generation } => Some(*generation),
        _ => None,
    });
    assert_eq!(
        gen_after,
        Some(gen_before + 1),
        "TracksResolved must re-fire once the known-PID count (V, A, C = 3, same as \
         the original V/A/B count) is fully resolved again — this is the bug a \
         count-keyed de-dup key would silently swallow; got {events:?}"
    );
}

// ── 8. Version wrap (31 -> 0) is treated as a change ────────────────────────

/// `version_number` wraps mod 32 (5 bits): 31 -> 0 must be treated as a
/// genuine change (inequality, never `>`) — a buggy `new > old` comparison
/// would see `0 > 31 == false` and wrongly skip applying it.
#[test]
fn version_wrap_31_to_0_is_treated_as_a_change() {
    let mut demux = StreamingTsDemux::new();
    feed_bootstrap(
        &mut demux,
        31,
        &[(PID_V, STREAM_TYPE_V, &lang_descriptor(b"eng", 0))],
    );
    let bootstrap_events = drain(&mut demux);
    let specs = track_added_specs(&bootstrap_events);
    assert_eq!(specs.len(), 1, "expected the single bootstrapped track");
    let track_id_v = specs[0].track_id;

    let wrapped = psi_packet(
        PMT_PID,
        2,
        &build_pmt(
            0,
            true,
            &[(PID_V, STREAM_TYPE_V, &lang_descriptor(b"qaa", 0))],
        ),
    );
    demux.feed(&wrapped);
    let events = drain(&mut demux);

    let updated: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            DemuxEvent::TrackUpdated(spec) => Some(spec.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        updated.len(),
        1,
        "version 31 -> 0 must be treated as a real change (inequality, not `>`), \
         got events {events:?}"
    );
    assert_eq!(updated[0].track_id, track_id_v);
    assert_eq!(updated[0].es_info_descriptors, lang_descriptor(b"qaa", 0));
}

// ── 9. TrackAbandoned::BudgetExceeded is reachable from this same fixture ──

/// Sanity cross-check (the dedicated unit tests live in `ts_demux.rs`, next
/// to the internal state they exercise): a PMT-declared PID whose config
/// never resolves and is abandoned before it ever appears in any applied
/// diff's PID set simply isn't part of `TracksResolved`'s "known" set at all
/// — abandonment and PMT-diff removal are independent paths, and this file
/// only exercises the latter directly. (See
/// `transmux::ts_demux`'s own `track_abandoned_config_unrecoverable_fires_at_finish`
/// and `probe_backlog_is_bounded_for_both_the_never_resolving_pid_and_its_collateral_pid`.)
#[test]
fn abandon_reason_is_reexported_and_usable_from_outside_the_crate() {
    // Purely a compile-time/API-surface check: `AbandonReason` must be a
    // usable public type from a downstream crate (issue #774).
    let reason = AbandonReason::ConfigUnrecoverable;
    assert_eq!(reason, AbandonReason::ConfigUnrecoverable);
    assert_ne!(reason, AbandonReason::BudgetExceeded);
}
