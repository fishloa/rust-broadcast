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

use broadcast_common::{Unpackage, crc32_mpeg2};
use mpeg_ts::ts::{TS_PACKET_SIZE, TsHeader};
use transmux::pipeline::TrackSpec;
use transmux::{AbandonReason, CodecConfig, DemuxEvent, StreamingTsDemux, TsDemux};

// ── Fixture constants ───────────────────────────────────────────────────────

const PAT_PID: u16 = 0x0000;
const PMT_PID: u16 = 0x1000;
/// Second program's PMT PID — used only by the two-program refcount test.
const PMT_PID_2: u16 = 0x1001;
const TRANSPORT_STREAM_ID: u16 = 1;
const PROGRAM_NUMBER: u16 = 1;
/// Second program — used by the PAT-remap and two-program refcount tests.
const PROGRAM_NUMBER_2: u16 = 2;

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
/// version 0 / `current_next_indicator = 1` — the default PAT for scenarios
/// that only vary the PMT.
fn build_pat() -> Vec<u8> {
    build_pat_full(0, true, &[(PROGRAM_NUMBER, PMT_PID)])
}

/// Build a PAT section (§2.4.4.3) with an explicit `version_number`,
/// `current_next_indicator`, and `(program_number, program_map_PID)` list —
/// the general form the PAT-remap / "next" PAT / corrupt-PAT scenarios need.
fn build_pat_full(version: u8, current_next_indicator: bool, programs: &[(u16, u16)]) -> Vec<u8> {
    const TABLE_ID_PAT: u8 = 0x00;
    let mut body = Vec::new();
    body.extend_from_slice(&TRANSPORT_STREAM_ID.to_be_bytes());
    body.push(0xC0 | (version << 1) | u8::from(current_next_indicator));
    body.push(0); // section_number
    body.push(0); // last_section_number
    for &(program_number, pmt_pid) in programs {
        body.extend_from_slice(&program_number.to_be_bytes());
        body.push(0xE0 | ((pmt_pid >> 8) as u8 & 0x1F));
        body.push((pmt_pid & 0xFF) as u8);
    }
    finish_section(TABLE_ID_PAT, &body)
}

/// Build a PMT section (ISO/IEC 13818-1 §2.4.4.8), always single-section
/// (`section_number == last_section_number == 0`) — every scenario here
/// varies only `version`/`current_next_indicator`/the ES loop.
fn build_pmt(version: u8, current_next_indicator: bool, entries: &[(u16, u8, &[u8])]) -> Vec<u8> {
    build_pmt_for(PROGRAM_NUMBER, version, current_next_indicator, entries)
}

/// [`build_pmt`] with an explicit `program_number` (`table_id_extension`) —
/// needed once more than one program is in play.
fn build_pmt_for(
    program_number: u16,
    version: u8,
    current_next_indicator: bool,
    entries: &[(u16, u8, &[u8])],
) -> Vec<u8> {
    const TABLE_ID_PMT: u8 = 0x02;
    const NO_PCR_PID: u16 = 0x1FFF;
    let mut body = Vec::new();
    body.extend_from_slice(&program_number.to_be_bytes()); // table_id_extension
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

/// A PES packet carrying a 33-bit PTS (ISO/IEC 13818-1 §2.4.3.6 / §2.4.3.7).
///
/// `private_stream_1` (`0xBD`) *does* carry the optional PES header (unlike
/// the `padding_stream` [`pes_bytes`] uses), so the layout is
/// `start_code(3) + stream_id(1) + PES_packet_length(2)` then `'10' flags(1) +
/// PTS_DTS_flags='10'(1) + PES_header_data_length=5(1)` and the 5-byte
/// `'0010' PTS[32:30] marker PTS[29:15] marker PTS[14:0] marker` field.
fn pes_bytes_with_pts(payload: &[u8], pts: u64) -> Vec<u8> {
    /// `'10' flags(1) + PTS_DTS_flags(1) + PES_header_data_length(1)` + the
    /// 5-byte PTS field.
    const OPTIONAL_HEADER_LEN: usize = 3 + 5;
    let mut b = Vec::with_capacity(6 + OPTIONAL_HEADER_LEN + payload.len());
    b.extend_from_slice(&[0x00, 0x00, 0x01, 0xBD]);
    b.extend_from_slice(&((OPTIONAL_HEADER_LEN + payload.len()) as u16).to_be_bytes());
    b.push(0x80); // '10' marker, no scrambling/priority/copyright
    b.push(0x80); // PTS_DTS_flags = '10' (PTS only)
    b.push(5); // PES_header_data_length
    b.push(0x21 | ((((pts >> 30) & 0x07) as u8) << 1));
    b.push(((pts >> 22) & 0xFF) as u8);
    b.push(((((pts >> 15) & 0x7F) as u8) << 1) | 0x01);
    b.push(((pts >> 7) & 0xFF) as u8);
    b.push((((pts & 0x7F) as u8) << 1) | 0x01);
    b.extend_from_slice(payload);
    b
}

/// Packetise one PES across as many 188-byte TS packets as it needs (ISO/IEC
/// 13818-1 §2.4.3.6): `payload_unit_start_indicator` on the first packet only,
/// with an incrementing 4-bit continuity counter. Trailing bytes of the last
/// packet are `0xFF` fill, bounded by the `PES_packet_length` every builder
/// here sets.
fn pes_ts_packets(pid: u16, cc_start: u8, pes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut off = 0usize;
    let mut cc = cc_start;
    while off < pes.len() {
        let n = (pes.len() - off).min(TS_PACKET_SIZE - 4);
        out.extend_from_slice(&ts_packet(pid, off == 0, cc & 0x0F, &pes[off..off + n]));
        off += n;
        cc = cc.wrapping_add(1);
    }
    out
}

/// Flip one bit in a section's trailing `CRC_32` (§2.4.4.1). The section stays
/// structurally valid in every other respect — only the checksum is wrong,
/// which is exactly the failure mode a single-bit transmission error produces
/// and the one a demuxer must never act on.
fn corrupt_crc(mut section: Vec<u8>) -> Vec<u8> {
    let last = section.len() - 1;
    section[last] ^= 0x01;
    section
}

/// Feed `bytes` padded with trailing null packets so `TsResync` reaches lock
/// (see [`feed_bootstrap`]'s note on `LOCK_CONFIRMATIONS`).
fn feed_locked(demux: &mut StreamingTsDemux, bytes: &[u8]) {
    let mut buf = bytes.to_vec();
    while buf.len() / TS_PACKET_SIZE < mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
        buf.extend_from_slice(&ts_packet(0x1FFF, false, 0, &[]));
    }
    demux.feed(&buf);
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
        1,
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
        1,
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
            DemuxEvent::TracksResolved { generation, .. } => Some(*generation),
            _ => None,
        })
        .expect("bootstrap must fire TracksResolved");

    let v2 = psi_packet(
        PMT_PID,
        1,
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
                ..
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
        DemuxEvent::TracksResolved { generation, .. } => Some(*generation),
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
            DemuxEvent::TracksResolved { generation, .. } => Some(*generation),
            _ => None,
        })
        .expect("bootstrap must fire TracksResolved");

    // v2: drop audioB, add C — known count: 3 -> 2 (diff applied) -> 3 (once
    // C resolves), i.e. it returns to its prior value of 3.
    let v2 = psi_packet(
        PMT_PID,
        1,
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
        DemuxEvent::TracksResolved { generation, .. } => Some(*generation),
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
        1,
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
/// `TrackAbandoned` must actually be **emitted**, with the right reason and
/// the right PID — a PMT-declared H.264 PID whose SPS/PPS never arrive stays
/// `Probing` until `finish()` concludes end of input.
///
/// (This replaces an `assert_eq!(reason, AbandonReason::ConfigUnrecoverable)`
/// against a value the test had just constructed itself — an assertion that
/// could not fail, and left `TrackAbandoned`'s emission unasserted from
/// outside the crate entirely.)
#[test]
fn track_abandoned_config_unrecoverable_is_actually_emitted() {
    const STREAM_TYPE_AVC: u8 = 0x1B;

    let mut demux = StreamingTsDemux::new();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&psi_packet(PAT_PID, 0, &build_pat()));
    bytes.extend_from_slice(&psi_packet(
        PMT_PID,
        0,
        &build_pmt(1, true, &[(PID_V, STREAM_TYPE_AVC, &[][..])]),
    ));
    // Annex B access units carrying only non-IDR slice NALs — never an
    // SPS/PPS, so `ConfigProbe::H264` can never resolve.
    for (i, cc) in (0u8..3).enumerate() {
        let au = [0x00, 0x00, 0x00, 0x01, 0x41, 0xAA, 0xBB, i as u8];
        bytes.extend_from_slice(&pes_ts_packets(PID_V, cc, &pes_bytes(&au)));
    }
    feed_locked(&mut demux, &bytes);
    let before = drain(&mut demux);
    assert!(
        !before
            .iter()
            .any(|e| matches!(e, DemuxEvent::TrackAbandoned { .. })),
        "abandonment is an end-of-input conclusion here, not a mid-stream one: {before:?}"
    );

    demux.finish();
    let events = drain(&mut demux);
    let abandoned: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            DemuxEvent::TrackAbandoned {
                track_id,
                reason,
                provenance,
                ..
            } => Some((*track_id, *reason, provenance.pid)),
            _ => None,
        })
        .collect();
    assert_eq!(
        abandoned,
        vec![(None, AbandonReason::ConfigUnrecoverable, Some(PID_V))],
        "exactly one TrackAbandoned, with no track_id (TrackAdded never fired), \
         the ConfigUnrecoverable reason, and the real PID; got {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, DemuxEvent::TrackAdded(_))),
        "a track whose config never resolved must never have been added"
    );
}

// ── C1. Codec reclassification rebuilds every piece of derived state ────────

/// `AC-3_descriptor` (ETSI EN 300 468 Annex D, tag `0x6A`) in its minimal
/// legal form: tag + length(1) + an all-zero flags byte (every optional field
/// absent) — the same shape `fixtures/ts/dolby/eac3_dvb_0x06.ts` uses for the
/// `0x7A` enhanced variant (see its PROVENANCE.md).
const AC3_DESCRIPTOR: [u8; 3] = [0x6A, 0x01, 0x00];
/// PES private data (ISO/IEC 13818-1 Table 2-34) — DVB's descriptor-
/// disambiguated Dolby carriage.
const STREAM_TYPE_PES_PRIVATE: u8 = 0x06;
/// ANSI/SCTE-scoped `stream_type`: **section**-carried (§2.4.4).
const STREAM_TYPE_SCTE35: u8 = 0x86;
/// AVC (H.264) — **PES**-carried.
const STREAM_TYPE_AVC: u8 = 0x1B;

/// Real captured AC-3 syncframes, pulled straight out of the committed
/// `fixtures/ts/dolby/ac3.ts` (the same real ffmpeg-encoded AC-3 capture
/// `tests/dolby.rs`'s ffmpeg `dac3` oracle test bites against). Used verbatim,
/// so the reclassified track's config is recovered from genuine BSI bits, not
/// from bytes invented to satisfy the parser.
fn real_ac3_syncframes() -> Vec<Vec<u8>> {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/dolby/ac3.ts");
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));
    let media = TsDemux::new()
        .unpackage(&bytes)
        .expect("demux fixtures/ts/dolby/ac3.ts");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Ac3 { .. }))
        .expect("ac3.ts carries an AC-3 track");
    track.samples.iter().map(|s| s.data.to_vec()).collect()
}

/// **C1 (CRITICAL, remote-input panic).** A PMT version change that
/// reclassifies a still-`Probing` PID — `stream_type` `0x06` gaining an
/// `AC-3_descriptor`, so `Codec::Data(0x06)` becomes `Codec::Ac3` (issue
/// #641) — used to write `stream.codec` in place and leave the
/// `ConfigProbe::Data` built for the old codec behind. The next access unit
/// then reached `finalize_probe`'s `let Codec::Data(_) = codec else {
/// unreachable!() }` and **panicked on broadcast input**.
///
/// The PID must survive the reclassification and end up correctly typed as
/// AC-3, its config recovered from the real syncframes fed after the change.
#[test]
fn codec_reclassification_rebuilds_the_probe_instead_of_panicking() {
    let mut demux = StreamingTsDemux::new();

    // v1: bare `stream_type` 0x06 -> `Codec::Data(0x06)`, `ConfigProbe::Data`.
    // Deliberately no PES yet, so the PID is still *Probing* when v2 lands.
    let mut boot = Vec::new();
    boot.extend_from_slice(&psi_packet(PAT_PID, 0, &build_pat()));
    boot.extend_from_slice(&psi_packet(
        PMT_PID,
        0,
        &build_pmt(1, true, &[(PID_V, STREAM_TYPE_PES_PRIVATE, &[][..])]),
    ));
    feed_locked(&mut demux, &boot);
    let boot_events = drain(&mut demux);
    assert!(
        !boot_events
            .iter()
            .any(|e| matches!(e, DemuxEvent::TrackAdded(_))),
        "no access unit yet, so the PID must still be probing: {boot_events:?}"
    );

    // v2: same PID, same stream_type, now with an AC-3 descriptor.
    demux.feed(&psi_packet(
        PMT_PID,
        1,
        &build_pmt(
            2,
            true,
            &[(PID_V, STREAM_TYPE_PES_PRIVATE, &AC3_DESCRIPTOR[..])],
        ),
    ));
    drain(&mut demux);

    // Real AC-3 syncframes, one PES each. Before the fix this panicked on the
    // very first one.
    let frames = real_ac3_syncframes();
    assert!(
        frames.len() >= 3,
        "fixture must carry several real syncframes, got {}",
        frames.len()
    );
    for (i, frame) in frames.iter().take(3).enumerate() {
        demux.feed(&pes_ts_packets(PID_V, (i as u8) * 8, &pes_bytes(frame)));
    }
    demux.finish();
    let events = drain(&mut demux);

    let added: Vec<TrackSpec> = track_added_specs(&events);
    assert_eq!(
        added.len(),
        1,
        "the reclassified PID must resolve to exactly one track, got {events:?}"
    );
    assert_eq!(added[0].source_pid, Some(PID_V));
    assert!(
        matches!(added[0].config, CodecConfig::Ac3 { .. }),
        "the reclassified track must be typed AC-3 (config recovered from the \
         real syncframes), got {:?}",
        added[0].config
    );
    assert_eq!(
        added[0].es_info_descriptors,
        AC3_DESCRIPTOR.to_vec(),
        "the new track must carry the v2 ES_info descriptor loop"
    );
}

/// **Carrier reset.** ISO/IEC 13818-1 Table 2-34 splits `stream_type` into
/// section-carried and PES-carried families, reassembled by completely
/// different engines. A reclassification across that boundary
/// (`0x86` SCTE-35, sections → `0x1B` H.264, PES) used to keep the old
/// `Carrier`, so H.264 PES bytes were fed to a `SectionReassembler` and the
/// track produced silence while still claiming to exist.
#[test]
fn carrier_is_rebuilt_when_a_section_carried_pid_becomes_pes_carried() {
    let mut demux = StreamingTsDemux::new();
    let mut boot = Vec::new();
    boot.extend_from_slice(&psi_packet(PAT_PID, 0, &build_pat()));
    boot.extend_from_slice(&psi_packet(
        PMT_PID,
        0,
        &build_pmt(1, true, &[(PID_V, STREAM_TYPE_SCTE35, &[][..])]),
    ));
    feed_locked(&mut demux, &boot);
    drain(&mut demux);

    // v2 reclassifies the same PID as H.264.
    demux.feed(&psi_packet(
        PMT_PID,
        1,
        &build_pmt(2, true, &[(PID_V, STREAM_TYPE_AVC, &[][..])]),
    ));
    drain(&mut demux);

    // A minimal but real Annex B access unit: SPS + PPS + IDR slice. The
    // SPS/PPS pair is the Baseline one `avc_config`'s own round-trip test
    // uses; `ConfigProbe::H264` needs both to resolve.
    let mut au = Vec::new();
    for nal in [
        &[0x67u8, 0x42, 0x00, 0x1E, 0xAB, 0x40][..],
        &[0x68, 0xCE, 0x3C, 0x80][..],
        &[0x65, 0x88, 0x84, 0x00, 0x11, 0x22][..],
    ] {
        au.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        au.extend_from_slice(nal);
    }
    for cc in 0u8..3 {
        demux.feed(&pes_ts_packets(PID_V, cc * 4, &pes_bytes(&au)));
    }
    demux.finish();
    let events = drain(&mut demux);

    let added = track_added_specs(&events);
    assert_eq!(added.len(), 1, "expected one H.264 track, got {events:?}");
    assert!(
        matches!(added[0].config, CodecConfig::Avc { .. }),
        "the reclassified track must be typed AVC, got {:?}",
        added[0].config
    );
    let samples: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, DemuxEvent::Sample { .. }))
        .collect();
    assert!(
        !samples.is_empty(),
        "a PES-carried H.264 track must produce samples — a stale \
         SectionReassembler yields exactly zero, silently; got {events:?}"
    );
}

// ── C2. PSI CRC_32 validation before destructive application ────────────────

/// **C2 (CRITICAL).** A PMT whose trailing `CRC_32` is wrong must be dropped
/// silently and disturb **nothing**: no `TrackRemoved`, no `track_id`
/// reassignment, and — crucially — no `last_applied_version` bump, which
/// would otherwise suppress the genuine section that follows.
///
/// PMT application became destructive with issue #774's version diffing, so
/// one flipped bit in a version byte or an ES loop used to destroy a live
/// track and renumber the survivors.
#[test]
fn corrupt_crc_pmt_is_dropped_and_disturbs_nothing() {
    let (mut demux, bootstrap_events) = bootstrap_three_tracks();
    let specs = track_added_specs(&bootstrap_events);
    let track_id_b = track_id_for_pid(&specs, PID_B);

    // A v2 that drops audioB — but with a corrupted CRC_32.
    let bad = build_pmt(
        2,
        true,
        &[
            (PID_V, STREAM_TYPE_V, &[][..]),
            (PID_A, STREAM_TYPE_A, &lang_descriptor(b"eng", 0)),
            // audioB dropped.
        ],
    );
    demux.feed(&psi_packet(PMT_PID, 1, &corrupt_crc(bad)));
    let events = drain(&mut demux);
    assert!(
        events.is_empty(),
        "a CRC-failing PMT must be dropped silently, changing nothing: {events:?}"
    );

    // Regression guard: a *valid* section still applies — and specifically,
    // the same version 2 the corrupt section claimed must not have been
    // swallowed by a `last_applied_version` bump.
    let good = build_pmt(
        2,
        true,
        &[
            (PID_V, STREAM_TYPE_V, &[][..]),
            (PID_A, STREAM_TYPE_A, &lang_descriptor(b"eng", 0)),
        ],
    );
    demux.feed(&psi_packet(PMT_PID, 2, &good));
    let events = drain(&mut demux);
    let removed: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            DemuxEvent::TrackRemoved { track_id, .. } => Some(*track_id),
            _ => None,
        })
        .collect();
    assert_eq!(
        removed,
        vec![track_id_b],
        "the valid same-version section must still apply after the corrupt one \
         was dropped; got {events:?}"
    );
}

/// **C2 (CRITICAL), PAT side.** `process_packet` checks `pmt_reasm` *before*
/// `streams`, so a PID wrongly learned as a PMT PID shadows its elementary
/// stream for the rest of the run. A corrupt PAT naming a live ES PID as a
/// program's PMT PID must therefore be dropped outright.
#[test]
fn corrupt_crc_pat_cannot_hijack_a_live_es_pid() {
    let (mut demux, bootstrap_events) = bootstrap_three_tracks();
    let specs = track_added_specs(&bootstrap_events);
    let track_id_b = track_id_for_pid(&specs, PID_B);

    // A PAT that claims audioB's PID is program 2's PMT PID — with a bad CRC.
    let bad = build_pat_full(
        1,
        true,
        &[(PROGRAM_NUMBER, PMT_PID), (PROGRAM_NUMBER_2, PID_B)],
    );
    demux.feed(&psi_packet(PAT_PID, 1, &corrupt_crc(bad)));
    drain(&mut demux);

    // audioB must still be a normal elementary stream: keep feeding it and
    // its samples must keep arriving.
    for cc in 0u8..3 {
        demux.feed(&pes_ts_packets(PID_B, cc, &pes_bytes(b"still-audio")));
    }
    let events = drain(&mut demux);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, DemuxEvent::Sample { track_id, .. } if *track_id == track_id_b)),
        "a CRC-failing PAT must not divert a live ES PID into PMT reassembly; got {events:?}"
    );
}

/// A `current_next_indicator == 0` ("next") PAT is parsed and dropped, exactly
/// as a "next" PMT already was — it must not bind a PID that is not yet a PMT
/// PID today.
#[test]
fn next_pat_cni_zero_cannot_hijack_a_live_es_pid() {
    let (mut demux, bootstrap_events) = bootstrap_three_tracks();
    let specs = track_added_specs(&bootstrap_events);
    let track_id_b = track_id_for_pid(&specs, PID_B);

    let next = build_pat_full(
        1,
        false,
        &[(PROGRAM_NUMBER, PMT_PID), (PROGRAM_NUMBER_2, PID_B)],
    );
    demux.feed(&psi_packet(PAT_PID, 1, &next));
    drain(&mut demux);

    for cc in 0u8..3 {
        demux.feed(&pes_ts_packets(PID_B, cc, &pes_bytes(b"still-audio")));
    }
    let events = drain(&mut demux);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, DemuxEvent::Sample { track_id, .. } if *track_id == track_id_b)),
        "a current_next_indicator=0 PAT must never be applied; got {events:?}"
    );
}

/// **PAT remap.** The PAT-derived `program_number` used to be write-once
/// (`entry().or_insert_with()`), so a legitimate remap of a PMT PID to another
/// program made the `program_number` cross-check in `process_packet` reject
/// every PMT on that PID **forever** — a silent zero-track demux.
#[test]
fn pat_remap_of_a_pmt_pid_is_honoured() {
    let (mut demux, bootstrap_events) = bootstrap_three_tracks();
    let specs = track_added_specs(&bootstrap_events);
    let track_id_b = track_id_for_pid(&specs, PID_B);

    // The multiplex re-issues this PMT PID under program 2.
    demux.feed(&psi_packet(
        PAT_PID,
        1,
        &build_pat_full(1, true, &[(PROGRAM_NUMBER_2, PMT_PID)]),
    ));
    drain(&mut demux);

    // A PMT under the NEW program, dropping audioB. It must be applied.
    demux.feed(&psi_packet(
        PMT_PID,
        1,
        &build_pmt_for(
            PROGRAM_NUMBER_2,
            2,
            true,
            &[
                (PID_V, STREAM_TYPE_V, &[][..]),
                (PID_A, STREAM_TYPE_A, &lang_descriptor(b"eng", 0)),
            ],
        ),
    ));
    let events = drain(&mut demux);
    let removed: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            DemuxEvent::TrackRemoved { track_id, .. } => Some(*track_id),
            _ => None,
        })
        .collect();
    assert_eq!(
        removed,
        vec![track_id_b],
        "a PMT under the remapped program_number must be applied, not rejected \
         forever by a frozen PAT binding; got {events:?}"
    );
}

// ── Removal purge ───────────────────────────────────────────────────────────

/// **Removal purge.** `remove_track` used to leave the dropped PID's payload
/// flowing into `unattributed` — the *pre*-registration replay buffer — so
/// post-removal orphan traffic accumulated and was then replayed as the
/// **re-added** track's first samples, anchoring its `start_decode_time` in
/// the past.
#[test]
fn post_removal_payload_is_not_replayed_into_the_re_added_track() {
    /// 90 kHz PES ticks. The orphan burst is stamped an hour before the
    /// re-added track's real traffic, so a replay is unmistakable in the dts.
    const ORPHAN_PTS: u64 = 90_000;
    const REPLACEMENT_PTS: u64 = 90_000 * 3600;
    const PTS_STEP: u64 = 3_000;

    let (mut demux, bootstrap_events) = bootstrap_three_tracks();
    let specs = track_added_specs(&bootstrap_events);
    let old_track_id_b = track_id_for_pid(&specs, PID_B);

    // v2 drops audioB.
    demux.feed(&psi_packet(
        PMT_PID,
        1,
        &build_pmt(
            2,
            true,
            &[
                (PID_V, STREAM_TYPE_V, &[][..]),
                (PID_A, STREAM_TYPE_A, &lang_descriptor(b"eng", 0)),
            ],
        ),
    ));
    let removal = drain(&mut demux);
    assert!(
        removal
            .iter()
            .any(|e| matches!(e, DemuxEvent::TrackRemoved { track_id, .. } if *track_id == old_track_id_b)),
        "sanity: audioB must have been removed; got {removal:?}"
    );

    // Orphan traffic on the now-undeclared PID.
    for i in 0u8..4 {
        let pts = ORPHAN_PTS + u64::from(i) * PTS_STEP;
        demux.feed(&pes_ts_packets(
            PID_B,
            i * 4,
            &pes_bytes_with_pts(b"orphan", pts),
        ));
    }
    drain(&mut demux);

    // v3 re-adds the same PID.
    demux.feed(&psi_packet(
        PMT_PID,
        2,
        &build_pmt(
            3,
            true,
            &[
                (PID_V, STREAM_TYPE_V, &[][..]),
                (PID_A, STREAM_TYPE_A, &lang_descriptor(b"eng", 0)),
                (PID_B, STREAM_TYPE_B, &[][..]),
            ],
        ),
    ));
    // Accumulate from the re-add onwards: `TrackAdded` for the fresh track can
    // land in either drain depending on when its probe resolves, and the first
    // Sample assertion below must see the whole post-re-add event stream.
    let mut events = drain(&mut demux);

    for i in 0u8..3 {
        let pts = REPLACEMENT_PTS + u64::from(i) * PTS_STEP;
        demux.feed(&pes_ts_packets(
            PID_B,
            i * 4,
            &pes_bytes_with_pts(b"fresh", pts),
        ));
    }
    demux.finish();
    events.extend(drain(&mut demux));

    let new_track_id = events
        .iter()
        .find_map(|e| match e {
            DemuxEvent::TrackAdded(spec) if spec.source_pid == Some(PID_B) => Some(spec.track_id),
            _ => None,
        })
        .expect("the re-added PID must fire a fresh TrackAdded");
    assert_ne!(
        new_track_id, old_track_id_b,
        "a re-added PID is a new track, with a new track_id"
    );

    let first = events
        .iter()
        .find_map(|e| match e {
            DemuxEvent::Sample {
                track_id, sample, ..
            } if *track_id == new_track_id => Some(sample),
            _ => None,
        })
        .expect("the re-added track must produce samples");
    assert_eq!(
        first.data.as_ref(),
        b"fresh",
        "the re-added track's first sample must be post-re-add traffic, never a \
         replayed pre-removal orphan payload"
    );
    assert_eq!(
        first.dts,
        Some(REPLACEMENT_PTS as i64),
        "start_decode_time must not be anchored in the past by a replayed orphan"
    );
}

// ── Multi-program ES refcount ───────────────────────────────────────────────

/// An ES PID declared by **two** programs (legal, and routine for a shared
/// audio/subtitle component) must survive one program dropping it: `streams`
/// and `es_seen` are global while `applied_es` is per-PMT, so removal has to
/// be refcounted by declaring PMT. Only the last declarer's drop tears it down.
#[test]
fn es_pid_declared_by_two_programs_survives_one_program_dropping_it() {
    let mut demux = StreamingTsDemux::new();
    let mut boot = Vec::new();
    boot.extend_from_slice(&psi_packet(
        PAT_PID,
        0,
        &build_pat_full(
            0,
            true,
            &[(PROGRAM_NUMBER, PMT_PID), (PROGRAM_NUMBER_2, PMT_PID_2)],
        ),
    ));
    // Both programs declare the shared PID_A; program 1 also has its own PID_V.
    boot.extend_from_slice(&psi_packet(
        PMT_PID,
        0,
        &build_pmt_for(
            PROGRAM_NUMBER,
            1,
            true,
            &[
                (PID_V, STREAM_TYPE_V, &[][..]),
                (PID_A, STREAM_TYPE_A, &[][..]),
            ],
        ),
    ));
    boot.extend_from_slice(&psi_packet(
        PMT_PID_2,
        0,
        &build_pmt_for(
            PROGRAM_NUMBER_2,
            1,
            true,
            &[(PID_A, STREAM_TYPE_A, &[][..])],
        ),
    ));
    for (i, pid) in [PID_V, PID_A].iter().enumerate() {
        let cc = (i as u8) * 4;
        boot.extend_from_slice(&pes_ts_packets(*pid, cc, &pes_bytes(b"au1")));
        boot.extend_from_slice(&pes_ts_packets(*pid, cc + 1, &pes_bytes(b"au2")));
    }
    feed_locked(&mut demux, &boot);
    let bootstrap_events = drain(&mut demux);
    let specs = track_added_specs(&bootstrap_events);
    let shared_track_id = track_id_for_pid(&specs, PID_A);

    // Program 1 drops the shared PID — program 2 still declares it.
    demux.feed(&psi_packet(
        PMT_PID,
        1,
        &build_pmt_for(PROGRAM_NUMBER, 2, true, &[(PID_V, STREAM_TYPE_V, &[][..])]),
    ));
    let events = drain(&mut demux);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, DemuxEvent::TrackRemoved { .. })),
        "the shared PID is still declared by program 2 — no TrackRemoved yet; got {events:?}"
    );
    for cc in 0u8..3 {
        demux.feed(&pes_ts_packets(PID_A, cc, &pes_bytes(b"still-shared")));
    }
    let events = drain(&mut demux);
    assert!(
        events.iter().any(
            |e| matches!(e, DemuxEvent::Sample { track_id, .. } if *track_id == shared_track_id)
        ),
        "the surviving shared track must keep producing samples; got {events:?}"
    );

    // Now the last declarer drops it too.
    demux.feed(&psi_packet(
        PMT_PID_2,
        1,
        &build_pmt_for(PROGRAM_NUMBER_2, 2, true, &[]),
    ));
    let events = drain(&mut demux);
    let removed: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            DemuxEvent::TrackRemoved { track_id, .. } => Some(*track_id),
            _ => None,
        })
        .collect();
    assert_eq!(
        removed,
        vec![shared_track_id],
        "the last declaring PMT dropping the PID must remove it; got {events:?}"
    );
}
