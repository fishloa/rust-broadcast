//! TSDuck's own `tstabcomp --decompile` output of the UNT (test-060) and RNT
//! (test-115) reference tables, read with `quick-xml`, compared with what this
//! crate decodes from the same `.pack.ts`. The XML is produced by an
//! independent implementation, so a field both decoders read differently fails.
#![cfg(feature = "ts")]

use broadcast_common::Parse;
use dvb_si::descriptors::rnt::AnyRntDescriptor;
use dvb_si::descriptors::unt::AnyUntDescriptor;
use dvb_si::tables::rnt::RntSection;
use dvb_si::tables::unt::UntSection;
use quick_xml::Reader;
use quick_xml::events::Event;
use std::collections::BTreeMap;

type Attrs = BTreeMap<String, String>;

fn elements(xml: &str, table: &str) -> Vec<Vec<(String, Attrs)>> {
    let mut reader = Reader::from_str(xml);
    let mut tables: Vec<Vec<(String, Attrs)>> = Vec::new();
    let mut inside = false;
    loop {
        match reader.read_event().expect("decompile XML parses") {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) => {
                let name = e.name().as_ref().to_owned();
                let attrs: Attrs = e
                    .attributes()
                    .map(|a| {
                        let a = a.unwrap();
                        (
                            a.key.as_ref().to_owned(),
                            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .unwrap()
                                .into_owned(),
                        )
                    })
                    .collect();
                if name == table {
                    inside = true;
                    tables.push(Vec::new());
                }
                if inside {
                    tables.last_mut().unwrap().push((name, attrs));
                }
            }
            Event::End(e) if e.name().as_ref() == table => inside = false,
            _ => {}
        }
    }
    tables
}

fn hex(attrs: &Attrs, key: &str) -> u64 {
    let v = &attrs[key];
    u64::from_str_radix(v.trim_start_matches("0x"), 16)
        .unwrap_or_else(|_| panic!("{key}={v} is not hex"))
}

fn num(attrs: &Attrs, key: &str) -> u64 {
    attrs[key]
        .parse()
        .unwrap_or_else(|_| panic!("{key} is not decimal"))
}

fn flag(attrs: &Attrs, key: &str) -> bool {
    match attrs[key].as_str() {
        "true" => true,
        "false" => false,
        other => panic!("{key}={other} is not a bool"),
    }
}

fn sections_of(ts: &[u8], tid: u8) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut open: Vec<u8> = Vec::new();
    for p in ts.chunks_exact(188) {
        let afc = (p[3] >> 4) & 3;
        if p[0] != 0x47 || afc & 1 == 0 {
            continue;
        }
        let mut i = 4;
        if afc & 2 != 0 {
            i += 1 + usize::from(p[4]);
        }
        let Some(pl) = p.get(i..).filter(|s| !s.is_empty()) else {
            continue;
        };
        if p[1] & 0x40 != 0 {
            let ptr = usize::from(pl[0]);
            open = pl[(1 + ptr).min(pl.len())..].to_vec();
        } else {
            open.extend_from_slice(pl);
        }
        while open.len() >= 3 {
            let len = ((usize::from(open[1]) & 0x0F) << 8 | usize::from(open[2])) + 3;
            if open.len() < len {
                break;
            }
            let sec: Vec<u8> = open.drain(..len).collect();
            if sec[0] == tid
                && broadcast_common::crc32_mpeg2::compute(&sec) == 0
                && !out.contains(&sec)
            {
                out.push(sec);
            }
        }
    }
    out
}

fn ymd_hms(mjd: u16, h: Option<u8>, m: Option<u8>, s: Option<u8>) -> String {
    let (y, mo, d) = broadcast_common::time::mjd_to_ymd(mjd);
    format!(
        "{y:04}-{mo:02}-{d:02} {:02}:{:02}:{:02}",
        h.unwrap(),
        m.unwrap(),
        s.unwrap()
    )
}

fn date(t: &dvb_si::descriptors::mjd_time::MjdBcdTime) -> String {
    let (mjd, h, m, s) = t.parts();
    ymd_hms(mjd, h, m, s)
}

#[test]
fn test_115_rnt_matches_tsduck_decompile() {
    let Some(xml) = test_fixtures::tsduck(
        "reference/test-115/test-115.decompile.xml",
        "tsduck_decompile_oracle",
    ) else {
        return;
    };
    let Some(ts) = test_fixtures::tsduck(
        "reference/test-115/test-115.pack.ts",
        "tsduck_decompile_oracle",
    ) else {
        return;
    };
    let xml = std::fs::read_to_string(xml).unwrap();
    let tables = elements(&xml, "RNT");
    assert_eq!(tables.len(), 1);
    let els = &tables[0];
    let header = &els[0].1;

    let secs = sections_of(&std::fs::read(ts).unwrap(), 0x79);
    assert_eq!(secs.len(), 1);
    let rnt = RntSection::parse(&secs[0]).unwrap();
    assert_eq!(u64::from(rnt.version_number), num(header, "version"));
    assert_eq!(rnt.current_next_indicator, flag(header, "current"));
    assert_eq!(u64::from(rnt.context_id), hex(header, "context_id"));
    assert_eq!(
        u64::from(rnt.context_id_type.to_u8()),
        hex(header, "context_id_type")
    );

    let mut dvb = els
        .iter()
        .filter(|(n, _)| n == "RAR_over_DVB_stream_descriptor");
    let mut ip = els.iter().filter(|(n, _)| n == "RAR_over_IP_descriptor");
    let mut refs = els.iter().filter(|(n, _)| n == "RNT_reference");
    let (mut n_dvb, mut n_ip, mut n_ref) = (0, 0, 0);
    for d in rnt.common_descriptors.iter() {
        match d.unwrap() {
            AnyRntDescriptor::RarOverDvbStream(d) => {
                let (_, a) = dvb
                    .next()
                    .expect("XML has a matching DVB-stream descriptor");
                assert_eq!(date(&d.first_valid_date), a["first_valid_date"]);
                assert_eq!(date(&d.last_valid_date), a["last_valid_date"]);
                assert_eq!(u64::from(d.weighting), num(a, "weighting"));
                assert_eq!(d.complete_flag, flag(a, "complete_flag"));
                assert_eq!(
                    u64::from(d.transport_stream_id),
                    hex(a, "transport_stream_id")
                );
                assert_eq!(
                    u64::from(d.original_network_id),
                    hex(a, "original_network_id")
                );
                assert_eq!(u64::from(d.service_id), hex(a, "service_id"));
                assert_eq!(u64::from(d.component_tag), hex(a, "component_tag"));
                match (&d.schedule, a.contains_key("download_start_time")) {
                    (Some(s), true) => {
                        assert_eq!(date(&s.download_start_time), a["download_start_time"]);
                        assert_eq!(
                            u64::from(s.download_period_duration),
                            num(a, "download_period_duration")
                        );
                        assert_eq!(
                            u64::from(s.download_cycle_time),
                            num(a, "download_cycle_time")
                        );
                    }
                    (None, false) => {}
                    (ours, xml_has) => {
                        panic!("schedule mismatch: ours {ours:?}, XML has {xml_has}")
                    }
                }
                n_dvb += 1;
            }
            AnyRntDescriptor::RarOverIp(d) => {
                let (_, a) = ip.next().expect("XML has a matching IP descriptor");
                assert_eq!(date(&d.first_valid_date), a["first_valid_date"]);
                assert_eq!(date(&d.last_valid_date), a["last_valid_date"]);
                assert_eq!(u64::from(d.weighting), num(a, "weighting"));
                assert_eq!(d.complete_flag, flag(a, "complete_flag"));
                assert_eq!(d.url.decode(), a["url"]);
                n_ip += 1;
            }
            AnyRntDescriptor::RntScan(d) => {
                for e in &d.entries {
                    let (_, a) = refs.next().expect("XML has a matching RNT_reference");
                    assert_eq!(
                        u64::from(e.transport_stream_id),
                        hex(a, "transport_stream_id")
                    );
                    assert_eq!(
                        u64::from(e.original_network_id),
                        hex(a, "original_network_id")
                    );
                    assert_eq!(u64::from(e.scan_weighting), num(a, "scan_weighting"));
                    n_ref += 1;
                }
            }
            other => panic!("unexpected RNT descriptor {other:?}"),
        }
    }
    assert!(dvb.next().is_none() && ip.next().is_none() && refs.next().is_none());
    assert_eq!((n_dvb, n_ip, n_ref), (2, 1, 2));
}

#[test]
fn test_060_unt_matches_tsduck_decompile() {
    let Some(xml) = test_fixtures::tsduck(
        "reference/test-060/test-060.decompile.xml",
        "tsduck_decompile_oracle",
    ) else {
        return;
    };
    let Some(ts) = test_fixtures::tsduck(
        "reference/test-060/test-060.pack.ts",
        "tsduck_decompile_oracle",
    ) else {
        return;
    };
    let xml = std::fs::read_to_string(xml).unwrap();
    let tables = elements(&xml, "UNT");
    let secs = sections_of(&std::fs::read(ts).unwrap(), 0x4B);
    assert_eq!(secs.len(), 2);
    assert_eq!(tables.len(), 2);
    let mut checked_descriptors = 0;
    for sec in &secs {
        let unt = UntSection::parse(sec).unwrap();
        let xml_table = tables
            .iter()
            .find(|t| hex(&t[0].1, "OUI") == u64::from(unt.oui))
            .unwrap_or_else(|| panic!("no XML UNT with OUI {:#X}", unt.oui));
        let h = &xml_table[0].1;
        assert_eq!(u64::from(unt.version_number), num(h, "version"));
        assert_eq!(unt.current_next_indicator, flag(h, "current"));
        assert_eq!(u64::from(unt.action_type.to_u8()), hex(h, "action_type"));
        assert_eq!(u64::from(unt.processing_order), hex(h, "processing_order"));

        let xml_uri = xml_table.iter().find(|(n, _)| n == "SSU_uri_descriptor");
        let xml_updates: Vec<&Attrs> = xml_table
            .iter()
            .filter(|(n, _)| n == "update_descriptor")
            .map(|(_, a)| a)
            .collect();
        let mut updates = 0;
        for d in unt.common_descriptors.iter() {
            match d.unwrap() {
                AnyUntDescriptor::SsuUri(u) => {
                    let (_, a) = xml_uri.expect("XML has the SSU_uri_descriptor");
                    assert_eq!(u64::from(u.max_holdoff_time), num(a, "max_holdoff_time"));
                    assert_eq!(
                        u64::from(u.min_polling_interval),
                        num(a, "min_polling_interval")
                    );
                    assert_eq!(u.uri.decode(), a["uri"]);
                    checked_descriptors += 1;
                }
                AnyUntDescriptor::Update(u) => {
                    let a = xml_updates[updates];
                    assert_eq!(u64::from(u.update_flag.to_u8()), num(a, "update_flag"));
                    assert_eq!(u64::from(u.update_method.to_u8()), num(a, "update_method"));
                    assert_eq!(u64::from(u.update_priority), num(a, "update_priority"));
                    updates += 1;
                    checked_descriptors += 1;
                }
                _ => {}
            }
        }
        assert_eq!(updates, xml_updates.len());
    }
    assert_eq!(
        checked_descriptors, 3,
        "SSU_uri + two update descriptors in the common loop"
    );
}
