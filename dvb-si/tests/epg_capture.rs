//! Real-capture validation of the EPG layer (`dvb_si::epg`).
//!
//! Fixture: `tests/fixtures/tnt-5w-12732v-isi6-10s.ts` — a 10 s satellite
//! capture (ISI 6) carrying standard SI, including a rich EIT (PID 0x0012).
//! This drives the full pipeline `SiDemux` → EIT section bytes →
//! `EpgStore::feed` → `now_and_next` on real broadcast data, not synthetic
//! vectors.
#![cfg(all(feature = "ts", feature = "chrono"))]

use dvb_si::demux::SiDemux;
use dvb_si::epg::EpgStore;
use dvb_si::ts::TS_PACKET_SIZE;

const EIT_PID: u16 = 0x0012;
const TS_SYNC: u8 = 0x47;

fn read_fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

#[test]
fn isi6_real_epg_build_and_now_next() {
    let data = read_fixture("tnt-5w-12732v-isi6-10s.ts");
    let mut demux = SiDemux::builder().build();
    let mut store = EpgStore::new();

    // Demux the capture; feed every EIT section into the EPG store.
    for chunk in data.chunks(TS_PACKET_SIZE) {
        if chunk.len() != TS_PACKET_SIZE || chunk[0] != TS_SYNC {
            continue;
        }
        for ev in demux.feed(chunk) {
            if u16::from(ev.pid()) == EIT_PID {
                // EIT sections feed the store; ignore non-fatal collect errors.
                let _ = store.feed(ev.bytes());
            }
        }
    }

    // Real present/following EIT populates one entry per service in the mux.
    // (Schedule EITs don't complete within this 10 s capture, so only p/f
    // events surface — which is the correct behaviour, not a gap.)
    assert!(
        store.service_count() >= 5,
        "real EIT should yield the mux's services, got {}",
        store.service_count()
    );
    assert!(
        store.event_count() >= 8,
        "real p/f EIT should yield present+following events, got {}",
        store.event_count()
    );

    // Pick a service that actually has time-stamped events and verify the
    // now/next contract at a known boundary: an event queried at its own
    // start time must be reported as "now".
    let key = store
        .services()
        .find(|k| {
            store
                .events(*k)
                .is_some_and(|evs| evs.iter().any(|e| e.start_time.is_some()))
        })
        .expect("a service with time-stamped events");

    let first = store
        .events(key)
        .unwrap()
        .into_iter()
        .find(|e| e.start_time.is_some())
        .cloned()
        .expect("an event with a start time");
    let start = first.start_time.expect("checked");

    let (now, _next) = store.now_and_next(key, start);
    assert_eq!(
        now.map(|e| e.event_id),
        Some(first.event_id),
        "the event covering its own start instant must be 'now'"
    );
}
