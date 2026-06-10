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

// ---------------------------------------------------------------------------
// _with_registry path: register a private descriptor, push through
// EitCollector, verify it surfaces as AnyDescriptor::Other.
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
struct MyPrivateDesc {
    val: u8,
}

impl<'a> dvb_common::Parse<'a> for MyPrivateDesc {
    type Error = dvb_si::Error;
    fn parse(bytes: &'a [u8]) -> dvb_si::Result<Self> {
        if bytes.len() < 3 {
            return Err(dvb_si::Error::BufferTooShort {
                need: 3,
                have: bytes.len(),
                what: "MyPrivateDesc",
            });
        }
        Ok(Self { val: bytes[2] })
    }
}

impl<'a> dvb_si::traits::DescriptorDef<'a> for MyPrivateDesc {
    const TAG: u8 = MY_PRIVATE_TAG;
    const NAME: &'static str = "MY_PRIVATE";
}

const MY_PRIVATE_TAG: u8 = 0xA7;

#[test]
fn eit_with_registry_surfaces_private_descriptor() {
    use dvb_si::collect::EitCollector;
    use dvb_si::descriptors::{AnyDescriptor, DescriptorRegistry};

    // Build a minimal EIT p/f section with one event containing a private
    // descriptor tag 0xA7 and payload byte 0x42.
    let private_desc = {
        // tag 0xA7, length 1, payload 0x42
        [MY_PRIVATE_TAG, 0x01, 0x42u8]
    };

    let eit_bytes = {
        let table_id: u8 = 0x4E; // EIT p/f actual
        let service_id: u16 = 100;
        let ts_id: u16 = 1;
        let on_id: u16 = 1;
        let event_id: u16 = 1;
        let start_raw: [u8; 5] = [0; 5];
        let dur_raw: [u8; 3] = [0; 3];

        // Event: 12 + descriptors.len()
        let ev_len = 12 + private_desc.len();
        let section_length = 5 + 6 + ev_len + 4;
        let total = 3 + section_length;

        let mut buf = vec![0u8; total];
        buf[0] = table_id;
        buf[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        buf[3..5].copy_from_slice(&service_id.to_be_bytes());
        buf[5] = 0xC1; // version=0, cni=1
        buf[6] = 0; // section_number
        buf[7] = 0; // last_section_number
        buf[8..10].copy_from_slice(&ts_id.to_be_bytes());
        buf[10..12].copy_from_slice(&on_id.to_be_bytes());
        buf[12] = 0; // segment_last_section_number
        buf[13] = 0x5F; // last_table_id

        let ev_off = 14;
        buf[ev_off..ev_off + 2].copy_from_slice(&event_id.to_be_bytes());
        buf[ev_off + 2..ev_off + 7].copy_from_slice(&start_raw);
        buf[ev_off + 7..ev_off + 10].copy_from_slice(&dur_raw);
        let dll = private_desc.len() as u16;
        buf[ev_off + 10] = ((dll >> 8) as u8) & 0x0F;
        buf[ev_off + 11] = (dll & 0xFF) as u8;
        buf[ev_off + 12..ev_off + 12 + private_desc.len()].copy_from_slice(&private_desc);

        let crc_pos = total - 4;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..].copy_from_slice(&crc.to_be_bytes());
        buf
    };

    // Without registry: the private descriptor is Unknown.
    {
        let mut collector = EitCollector::new();
        let completed = collector.push_section(&eit_bytes).unwrap().unwrap();
        let tables = completed.tables().unwrap();
        let ev = &tables[0].events[0];
        let descs = ev.descriptors.descriptors();
        assert_eq!(descs.len(), 1);
        match &descs[0] {
            Ok(AnyDescriptor::Unknown { tag, .. }) => {
                assert_eq!(*tag, MY_PRIVATE_TAG);
            }
            other => panic!("expected Unknown (no registry), got {:?}", other),
        }
    }

    // With registry: the private descriptor surfaces as Other.
    {
        let mut reg = DescriptorRegistry::new();
        reg.register::<MyPrivateDesc>();

        let mut collector = EitCollector::new();
        let completed = collector.push_section(&eit_bytes).unwrap().unwrap();
        let tables = completed.tables_with_registry(Some(&reg)).unwrap();
        let ev = &tables[0].events[0];
        let descs = ev.descriptors.descriptors();
        assert_eq!(descs.len(), 1);
        match &descs[0] {
            Ok(AnyDescriptor::Other { tag, value }) => {
                assert_eq!(*tag, MY_PRIVATE_TAG);
                let concrete = value.downcast_ref::<MyPrivateDesc>().unwrap();
                assert_eq!(concrete.val, 0x42);
            }
            other => panic!("expected Other (registry), got {:?}", other),
        }
    }
}
