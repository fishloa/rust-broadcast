//! End-to-end fixture tests: full captures through `SiDemux`.
//!
//! Two captures are exercised:
//! - `m6-single.ts`   — French TNT, M6 mux (HbbTV AIT + carousel; short clip)
//! - `tnt-5w-12732v-isi6-10s.ts` — 10 s satellite capture (ISI 6)
//!
//! For each capture the tests assert:
//! 1. Exact set of `AnyTable` variant names produced (discovery → pin).
//! 2. Version-gate proof: second pass through the SAME demux emits 0 new
//!    events (or a strictly bounded number for carousel churn — see comment).
//! 3. Decoded-JSON acceptance: a service_descriptor found in an SDT service
//!    serializes as camelCase JSON with a decoded non-empty service name string
//!    (EIT short_event is used for tnt-5w since that capture carries EIT; the
//!    M6 capture is too short to include SDT/EIT, so the JSON test is only run
//!    against the tnt-5w fixture).
//! 4. Pmt in the pinned set also proves PAT-follow (PMT PIDs are only discoverable via the PAT).
//! 5. Stats sanity: `crc_failures` and `malformed_packets` pinned to observed
//!    values.
//!
//! # Fixture summary (discovered on first `-- --nocapture` run)
//!
//! `m6-single.ts` (1264 packets / 237 632 bytes):
//!   PIDs: 0x0000 (PAT), 0x0064/0x0082–0x0084/0x008C (PMTs, from PAT-follow),
//!         0x00AA (AIT), 0x00AB (DSM-CC carousel).
//!   No SDT/NIT/EIT/TDT-TOT in this short clip.
//!   Table set with explicit PID watch: {Ait, DsmccSection, Pat, Pmt}.
//!   Stats: emitted=6, sections_completed=50, crc_failures=0, malformed=0.
//!
//! `tnt-5w-12732v-isi6-10s.ts` (13 515 packets / 2 540 820 bytes):
//!   Standard SI PIDs present: 0x0010 (NIT), 0x0011 (SDT), 0x0012 (EIT).
//!   Table set (default demux): {Eit, Nit, Pat, Pmt, Sdt}.
//!   Stats: emitted=237, sections_completed=484, crc_failures=0, malformed=0.

use std::collections::BTreeSet;

use dvb_si::demux::SiDemux;
use dvb_si::pid::Pid;
use dvb_si::tables::AnyTable;
use dvb_si::ts::TS_PACKET_SIZE;

// ─────────────────────────────────────── helpers ────────────────────────────

fn read_fixture(filename: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), filename);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// Feed every aligned 188-byte packet in `data` through `demux`.
/// Returns the collected table variant names and the events.
fn feed_all(
    demux: &mut SiDemux,
    data: &[u8],
) -> (BTreeSet<String>, Vec<dvb_si::demux::SectionEvent>) {
    let mut names = BTreeSet::new();
    let mut events = Vec::new();
    for chunk in data.chunks(TS_PACKET_SIZE) {
        if chunk.len() != TS_PACKET_SIZE {
            continue;
        }
        // Sync byte check — skip non-aligned packets.
        if chunk[0] != 0x47 {
            continue;
        }
        for ev in demux.feed(chunk) {
            let name = variant_name(ev.table().as_ref());
            names.insert(name);
            events.push(ev);
        }
    }
    (names, events)
}

/// Return the variant name string from a `Result<&AnyTable>`.
fn variant_name(result: Result<&AnyTable<'_>, &dvb_si::error::Error>) -> String {
    match result {
        Err(_) => "ParseError".to_string(),
        Ok(t) => match t {
            AnyTable::Pat(_) => "Pat",
            AnyTable::Cat(_) => "Cat",
            AnyTable::Pmt(_) => "Pmt",
            AnyTable::Tsdt(_) => "Tsdt",
            AnyTable::DsmccSection(_) => "DsmccSection",
            AnyTable::Nit(_) => "Nit",
            AnyTable::Sdt(_) => "Sdt",
            AnyTable::Bat(_) => "Bat",
            AnyTable::Unt(_) => "Unt",
            AnyTable::Int(_) => "Int",
            AnyTable::Sat(_) => "Sat",
            AnyTable::Eit(_) => "Eit",
            AnyTable::Tdt(_) => "Tdt",
            AnyTable::Rst(_) => "Rst",
            AnyTable::St(_) => "St",
            AnyTable::Tot(_) => "Tot",
            AnyTable::Ait(_) => "Ait",
            AnyTable::Container(_) => "Container",
            AnyTable::Rct(_) => "Rct",
            AnyTable::Cit(_) => "Cit",
            AnyTable::MpeFec(_) => "MpeFec",
            AnyTable::Rnt(_) => "Rnt",
            AnyTable::MpeIfec(_) => "MpeIfec",
            AnyTable::ProtectionMessage(_) => "ProtectionMessage",
            AnyTable::DownloadableFontInfo(_) => "DownloadableFontInfo",
            AnyTable::Dit(_) => "Dit",
            AnyTable::Sit(_) => "Sit",
            AnyTable::MpeDatagram(_) => "MpeDatagram",
            AnyTable::Unknown { table_id, .. } => {
                return format!("Unknown(0x{table_id:02X})");
            }
            // a new AnyTable variant reached the capture — pin it in the expected set
            _ => "UNPINNED_NEW_VARIANT",
        }
        .to_string(),
    }
}

// ════════════════════════════════ m6-single.ts ═══════════════════════════════
//
// The M6 clip is 1 264 packets and only carries PAT, PMTs, AIT, and DSM-CC.
// There is no SDT/NIT/EIT/TDT-TOT in this short snippet.  The default demux
// (dvb_si_pids = true) watches well-known SI PIDs which are absent here.
// To observe AIT and DSM-CC we add those PIDs explicitly.

/// Build the M6 demux: standard SI PIDs + AIT (0x00AA) + DSM-CC (0x00AB).
fn m6_demux() -> SiDemux {
    SiDemux::builder()
        .pid(Pid::new(0x00AA)) // AIT
        .pid(Pid::new(0x00AB)) // DSM-CC HbbTV carousel
        .build()
}

/// Test 1 — m6-single.ts: exact table-set pin.
#[test]
fn m6_table_set() {
    let data = read_fixture("m6-single.ts");
    let mut demux = m6_demux();
    let (names, events) = feed_all(&mut demux, &data);

    println!("m6-single.ts table variants:");
    for n in &names {
        println!("  {n}");
    }
    println!(
        "m6-single.ts events={}, sections_completed={}, emitted={}, suppressed={}, \
         crc_failures={}, malformed_packets={}",
        events.len(),
        demux.stats().sections_completed,
        demux.stats().emitted,
        demux.stats().suppressed,
        demux.stats().crc_failures,
        demux.stats().malformed_packets,
    );

    // Pinned table set — discovered by first `-- --nocapture` run.
    // The m6 clip has no standard SI PIDs; Ait and DsmccSection come from the
    // explicit PID additions above.
    let expected: BTreeSet<String> = ["Ait", "DsmccSection", "Pat", "Pmt"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        names, expected,
        "m6-single.ts: table set changed — update the pin"
    );

    // Minimum event count: 1 PAT + ≥1 PMT + ≥1 AIT + ≥1 DSM-CC.
    assert!(
        events.len() >= 4,
        "expected at least 4 distinct section events, got {}",
        events.len()
    );

    // Stats sanity: clean capture, no CRC failures. Pin emitted/sections_completed.
    assert_eq!(
        demux.stats().emitted,
        6,
        "m6-single.ts: pinned emitted changed — verify fixture stability"
    );
    assert_eq!(
        demux.stats().sections_completed,
        50,
        "m6-single.ts: pinned sections_completed changed — verify fixture stability"
    );
    assert_eq!(
        demux.stats().crc_failures,
        0,
        "m6-single.ts: unexpected CRC failures"
    );
    assert_eq!(
        demux.stats().malformed_packets,
        0,
        "m6-single.ts: unexpected malformed_packets"
    );
}

/// Test 2 — m6-single.ts: version-gate proof.
///
/// Feed the capture once, record emitted and sections_completed.  Feed the
/// SAME capture again through the SAME demux.
///
/// DSM-CC DDB sections (0x3C) are excluded from the strict-zero assertion
/// because a carousel may cycle its version_number mid-capture, legitimately
/// causing a re-emission on the second pass.  All non-DSM-CC tables must be
/// fully suppressed.  The m6 clip is too short to span a version increment so
/// we also pin DSM-CC churn to zero; if this ever fires, investigate carousel
/// version cycling and update accordingly.
#[test]
fn m6_version_gate_suppresses_second_pass() {
    let data = read_fixture("m6-single.ts");
    let mut demux = m6_demux();

    let (_names, _events) = feed_all(&mut demux, &data);
    let first_emitted = demux.stats().emitted;
    let first_sections = demux.stats().sections_completed;
    let pre_suppressed = demux.stats().suppressed;
    println!(
        "m6 gate test: first pass emitted={first_emitted}, sections_completed={first_sections}"
    );

    // Second pass — collect and split by table_id category.
    let mut second_non_dsmcc: u64 = 0;
    let mut second_dsmcc: u64 = 0;
    for chunk in data.chunks(TS_PACKET_SIZE) {
        if chunk.len() != TS_PACKET_SIZE || chunk[0] != 0x47 {
            continue;
        }
        for ev in demux.feed(chunk) {
            if matches!(ev.table_id(), 0x3A..=0x3F) {
                second_dsmcc += 1;
            } else {
                second_non_dsmcc += 1;
            }
        }
    }
    println!("m6 gate test: second pass non-dsmcc={second_non_dsmcc}, dsmcc={second_dsmcc}");

    // Non-DSM-CC must be fully suppressed.
    assert_eq!(
        second_non_dsmcc, 0,
        "m6-single.ts: non-DSM-CC tables re-emitted on second pass (version gate broken)"
    );

    // DSM-CC: pin to 0 for this short clip.  If a carousel version increment
    // occurs mid-capture, second_dsmcc will be > 0; at that point, relax this
    // assertion to `second_dsmcc <= N` with an explanatory comment.
    assert_eq!(
        second_dsmcc, 0,
        "m6-single.ts: unexpected DSM-CC carousel churn on second pass — \
         investigate whether the capture spans a version increment"
    );

    // suppressed must have grown by at least first_sections (every section
    // from the second pass was gated).
    let post_suppressed = demux.stats().suppressed;
    assert!(
        post_suppressed >= pre_suppressed + first_sections,
        "m6-single.ts: suppressed growth ({} + {} = {}) but post_suppressed = {}",
        pre_suppressed,
        first_sections,
        pre_suppressed + first_sections,
        post_suppressed
    );

    assert_eq!(demux.stats().emitted, first_emitted);
}

// ══════════════════════════════ tnt-5w fixture ═══════════════════════════════
//
// 10-second satellite capture with full SI: PAT, NIT, SDT, EIT, and PMTs
// from PAT-follow.  No HbbTV carousel — all SI tables are stable.

/// Test 4 — tnt-5w-12732v-isi6-10s.ts: exact table-set pin.
#[test]
fn tnt_table_set() {
    let data = read_fixture("tnt-5w-12732v-isi6-10s.ts");
    let mut demux = SiDemux::builder().build();
    let (names, events) = feed_all(&mut demux, &data);

    println!("tnt-5w table variants:");
    for n in &names {
        println!("  {n}");
    }
    println!(
        "tnt-5w events={}, sections_completed={}, emitted={}, suppressed={}, \
         crc_failures={}, malformed_packets={}",
        events.len(),
        demux.stats().sections_completed,
        demux.stats().emitted,
        demux.stats().suppressed,
        demux.stats().crc_failures,
        demux.stats().malformed_packets,
    );

    // Pinned table set — discovered from `-- --nocapture` run.
    // Note: 0x0014 (TDT/TOT) is absent from this capture. 0x0013 (RST,
    // table_id 0x71) is watched by default but carries no sections here;
    // 0x0015 is NETWORK_SYNC and not SI.
    let expected: BTreeSet<String> = ["Eit", "Nit", "Pat", "Pmt", "Sdt"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        names, expected,
        "tnt-5w: table set changed — update the pin"
    );

    assert!(
        events.len() >= 5,
        "expected at least 5 distinct section events, got {}",
        events.len()
    );

    // Stats: clean satellite capture — zero failures observed. Pin emitted/sections_completed.
    assert_eq!(
        demux.stats().emitted,
        237,
        "tnt-5w: pinned emitted changed — verify fixture stability"
    );
    assert_eq!(
        demux.stats().sections_completed,
        484,
        "tnt-5w: pinned sections_completed changed — verify fixture stability"
    );
    assert_eq!(
        demux.stats().crc_failures,
        0,
        "tnt-5w: unexpected CRC failures"
    );
    assert_eq!(
        demux.stats().malformed_packets,
        0,
        "tnt-5w: unexpected malformed_packets"
    );
}

/// Test 3 — tnt-5w fixture: version-gate proof.
///
/// No carousel in this capture, so all SI tables are stable.  Strict zero
/// re-emissions expected on the second pass.
#[test]
fn tnt_version_gate_suppresses_second_pass() {
    let data = read_fixture("tnt-5w-12732v-isi6-10s.ts");
    let mut demux = SiDemux::builder().build();

    let (_names, _events) = feed_all(&mut demux, &data);
    let first_emitted = demux.stats().emitted;
    let first_sections = demux.stats().sections_completed;
    let pre_suppressed = demux.stats().suppressed;
    println!(
        "tnt gate test: first pass emitted={first_emitted}, sections_completed={first_sections}"
    );

    let mut second_emitted: u64 = 0;
    for chunk in data.chunks(TS_PACKET_SIZE) {
        if chunk.len() != TS_PACKET_SIZE || chunk[0] != 0x47 {
            continue;
        }
        for _ev in demux.feed(chunk) {
            second_emitted += 1;
        }
    }
    println!("tnt gate test: second pass emitted={second_emitted}");

    assert_eq!(
        second_emitted, 0,
        "tnt-5w: non-zero second-pass emissions ({second_emitted}) — version gate broken"
    );

    let post_suppressed = demux.stats().suppressed;
    assert!(
        post_suppressed > pre_suppressed,
        "tnt-5w: suppressed counter did not grow after second pass \
         ({pre_suppressed} → {post_suppressed})"
    );
    assert_eq!(demux.stats().emitted, first_emitted);
}

/// Test 4 — tnt-5w fixture: decoded-JSON acceptance (issue #16).
///
/// The tnt-5w capture carries EIT p/f (table_id 0x4E). Find an EIT event
/// with a short_event_descriptor, run `parse_loop` over its descriptor loop,
/// serialize to JSON with `serde_json::to_value`, and pin:
/// - The camelCase external-tagged key `"shortEvent"`.
/// - The decoded `event_name` string (real captured text).
/// - The `language_code` as a 3-char string.
///
/// SDT service_name is also pinned (no EIT dependency) to cover the Service
/// descriptor JSON path as well.
#[cfg(feature = "serde")]
#[test]
fn tnt_decoded_json_acceptance() {
    use dvb_common::Parse;
    use dvb_si::tables::eit::Eit;
    use dvb_si::tables::sdt::Sdt;

    let data = read_fixture("tnt-5w-12732v-isi6-10s.ts");
    let mut demux = SiDemux::builder().build();
    let (_names, events) = feed_all(&mut demux, &data);

    // ── SDT service_name acceptance (typed loop, no manual parse_loop) ────
    // Serialize the whole SDT TABLE with serde_json::to_value. The
    // SdtService.descriptors field is a DescriptorLoop that serializes as the
    // typed descriptor sequence, so the decoded service_name "TF1" must appear
    // inside the table JSON without any manual descriptor walking.
    {
        let sdt_event = events
            .iter()
            .find(|ev| matches!(ev.table_id(), 0x42 | 0x46))
            .expect("no SDT event found in tnt-5w");

        let sdt = Sdt::parse(sdt_event.bytes()).expect("SDT parse");
        let table_json = serde_json::to_value(&sdt).expect("serialize SDT table");

        // Find the first service descriptor's decoded service_name in the
        // typed loop emitted by serde — purely by walking the JSON value.
        let mut found_name: Option<String> = None;
        'sdt: for svc in table_json["services"].as_array().expect("services array") {
            for entry in svc["descriptors"].as_array().expect("typed loop array") {
                if let Some(name) = entry
                    .get("service")
                    .and_then(|s| s.get("service_name"))
                    .and_then(|v| v.as_str())
                {
                    if !name.is_empty() {
                        println!("tnt-5w SDT service_name (via typed loop JSON): {name:?}");
                        found_name = Some(name.to_string());
                        break 'sdt;
                    }
                }
            }
        }

        let svc_name = found_name.expect("no non-empty service_name in tnt-5w SDT table JSON");

        // Pinned: first service with a non-empty name in the first SDT section.
        assert_eq!(
            svc_name, "TF1",
            "tnt-5w: pinned SDT service_name changed — re-discover with --nocapture"
        );
    }

    // ── EIT short_event_descriptor JSON acceptance ───────────────────────
    // Scan ALL EIT section events for any event carrying a short_event
    // descriptor (tag 0x4D).  Note: `AnyDescriptor` serializes variant keys
    // as camelCase ("shortEvent") but the struct FIELD names remain snake_case
    // ("event_name", "language_code") because only the enum derive has
    // `rename_all = "camelCase"`, not the individual descriptor structs.
    // This matches the existing descriptor_loop test in the test suite.
    {
        let mut found: Option<(String, String, serde_json::Value)> = None;
        'outer: for eit_event in events
            .iter()
            .filter(|ev| matches!(ev.table_id(), 0x4E..=0x6F))
        {
            let eit = match Eit::parse(eit_event.bytes()) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for ev in &eit.events {
                if ev.descriptors.is_empty() {
                    continue;
                }
                let items: Vec<_> = ev.descriptors.iter().filter_map(|r| r.ok()).collect();
                let json = serde_json::to_value(&items).expect("serde_json");
                for entry in json.as_array().unwrap() {
                    if let Some(se) = entry.get("shortEvent") {
                        // Field names inside the struct are snake_case.
                        let lang = se
                            .get("language_code")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = se
                            .get("event_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        println!("tnt-5w EIT short_event: lang={lang:?} event_name={name:?}");
                        // Accept any short_event, even with empty name (valid per spec).
                        // We pin the lang_code which is always present.
                        if !lang.is_empty() {
                            found = Some((name, lang, json.clone()));
                            break 'outer;
                        }
                    }
                }
            }
        }

        let (event_name, lang_code, descriptors_json) = found
            .expect("no short_event_descriptor with non-empty language_code found in tnt-5w EIT");

        // ── Structural assertions (issue #16 acceptance) ──────────────────
        //
        // 1. Variant key is camelCase ("shortEvent"), not snake_case.
        assert!(
            descriptors_json
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e.get("shortEvent").is_some()),
            "tnt-5w: no 'shortEvent' camelCase variant key in EIT descriptor JSON"
        );
        let se_obj = descriptors_json
            .as_array()
            .unwrap()
            .iter()
            .find_map(|e| e.get("shortEvent"))
            .unwrap();

        // 2. Struct field names are snake_case (only the enum variant key is
        //    camelCase-renamed; the inner struct derives do not add rename_all).
        assert!(
            se_obj.get("event_name").is_some(),
            "expected snake_case 'event_name' in short_event JSON, got: {se_obj}"
        );
        assert!(
            se_obj.get("language_code").is_some(),
            "expected snake_case 'language_code' in short_event JSON, got: {se_obj}"
        );

        // 3. language_code serializes as a 3-char decoded string.
        assert_eq!(
            lang_code.len(),
            3,
            "language_code must be 3 chars, got {lang_code:?}"
        );

        // 4. event_name is a decoded string (may be empty — that is valid per
        //    EN 300 468; the pinned value below is what the capture actually has).
        let _ = event_name; // used in pin below

        // ── Pinned string assertions ───────────────────────────────────────
        // Discovered by running with `-- --nocapture`; update if fixture changes.
        // Language code "fre" is the ISO 639-2/B (bibliographic) code for French.
        assert_eq!(
            lang_code, "fre",
            "tnt-5w: pinned language_code changed — re-discover with --nocapture"
        );
        assert_eq!(
            event_name, "Téléfoot. \"Téléfoot n°33\"...",
            "tnt-5w: pinned EIT event_name changed — re-discover with --nocapture"
        );
    }
}
