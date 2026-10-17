//! The probes. Each returns `None` when it does not apply to the fixture, else
//! a JSON object that is compared field-by-field between the two trees.

use crate::{Cfg, Fixture, PidRaw, bump, hx, hx_opt, sha, ts_packets, variant_name};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

// ---------------------------------------------------------------------------
// transmux
// ---------------------------------------------------------------------------

fn codec_identity(cfg: &transmux::pipeline::CodecConfig) -> String {
    let kind = variant_name(&format!("{cfg:?}"));
    let rfc = transmux::dash::rfc6381_codec_string(cfg)
        .unwrap_or_else(|e| format!("err:{}", variant_name(&format!("{e:?}"))));
    format!("{kind}|{rfc}")
}

#[derive(Default)]
struct TrackAgg {
    samples: u64,
    bytes: u64,
    no_dts: u64,
    first_dts: Option<i64>,
    last_dts: Option<i64>,
    hasher: Vec<u8>,
}

pub fn transmux_stream(fx: &Fixture, _: &Cfg) -> Option<Value> {
    if !fx.is_ts_ext() {
        return None;
    }
    let packets = ts_packets(&fx.data)?;
    let mut d = transmux::ts_demux::StreamingTsDemux::new();
    let mut events: Vec<Value> = Vec::new();
    let mut agg: BTreeMap<String, TrackAgg> = BTreeMap::new();
    let mut counters: BTreeMap<String, u64> = BTreeMap::new();
    let mut drain = |d: &mut transmux::ts_demux::StreamingTsDemux, phase: &str| {
        while let Some(ev) = d.poll_event() {
            use transmux::ir::DemuxEvent;
            match ev {
                DemuxEvent::TrackAdded(spec) => events.push(json!({
                    "ev": "added", "phase": phase,
                    "track_id": format!("{:?}", spec.track_id),
                    "pid": hx_opt(spec.source_pid),
                    "program": format!("{:?}", spec.program_number),
                    "codec": codec_identity(&spec.config),
                    "timescale": spec.timescale,
                })),
                DemuxEvent::TrackUpdated(spec) => {
                    bump(&mut counters, format!("updated:{:?}", spec.track_id));
                }
                DemuxEvent::TrackRemoved { track_id, .. } => events.push(json!({
                    "ev": "removed", "phase": phase, "track_id": format!("{track_id:?}"),
                })),
                DemuxEvent::TrackAbandoned {
                    reason, provenance, ..
                } => events.push(json!({
                    "ev": "abandoned", "phase": phase,
                    "reason": reason.name(), "pid": hx_opt(provenance.pid),
                })),
                DemuxEvent::Sample {
                    track_id, sample, ..
                } => {
                    let a = agg.entry(format!("{track_id:?}")).or_default();
                    a.samples += 1;
                    a.bytes += sample.data.len() as u64;
                    a.hasher.extend_from_slice(&sample.data);
                    match sample.dts {
                        Some(t) => {
                            a.first_dts.get_or_insert(t);
                            a.last_dts = Some(t);
                        }
                        None => a.no_dts += 1,
                    }
                }
                DemuxEvent::ClockReference { .. } => bump(&mut counters, "clock_reference"),
                DemuxEvent::Discontinuity { kind, .. } => {
                    bump(&mut counters, format!("discontinuity:{}", kind.name()));
                }
                DemuxEvent::InputDegraded { kind, .. } => {
                    bump(&mut counters, format!("degraded:{}", kind.name()));
                }
                other => bump(
                    &mut counters,
                    format!("other:{}", variant_name(&format!("{other:?}"))),
                ),
            }
        }
    };
    for p in &packets {
        d.feed(p);
        drain(&mut d, "pre_finish");
    }
    d.finish();
    drain(&mut d, "at_finish");
    let tracks: Map<String, Value> = agg
        .into_iter()
        .map(|(k, a)| {
            (
                k,
                json!({
                    "samples": a.samples, "bytes": a.bytes, "no_dts": a.no_dts,
                    "first_dts": a.first_dts, "last_dts": a.last_dts,
                    "sha256": sha(&a.hasher),
                }),
            )
        })
        .collect();
    Some(json!({
        "packets": packets.len(),
        "events": events,
        "counters": counters,
        "tracks": tracks,
    }))
}

pub fn transmux_batch(fx: &Fixture, _: &Cfg) -> Option<Value> {
    if !fx.is_ts_ext() {
        return None;
    }
    let packets = ts_packets(&fx.data)?;
    let flat: Vec<u8> = packets.iter().flat_map(|p| p.iter().copied()).collect();
    let mut d = transmux::ts_demux::TsDemux::new();
    Some(match d.demux(&flat) {
        Err(e) => json!({ "err": format!("{e}") }),
        Ok(media) => {
            let tracks: Vec<Value> = media
                .tracks
                .iter()
                .map(|t| {
                    let mut all = Vec::new();
                    let mut bytes = 0u64;
                    for s in &t.samples {
                        bytes += s.data.len() as u64;
                        all.extend_from_slice(&s.data);
                    }
                    json!({
                        "track_id": format!("{:?}", t.spec.track_id),
                        "pid": hx_opt(t.spec.source_pid),
                        "program": format!("{:?}", t.spec.program_number),
                        "codec": codec_identity(&t.spec.config),
                        "samples": t.samples.len(), "bytes": bytes, "sha256": sha(&all),
                    })
                })
                .collect();
            json!({ "tracks": tracks, "pcr": media.pcr.len(), "skipped": media.skipped.len() })
        }
    })
}

pub fn transmux_run(fx: &Fixture, _: &Cfg) -> Option<Value> {
    use transmux::cli::{Opts, Output, OutputFormat, run_bytes};
    if !matches!(
        fx.ext.as_str(),
        "ts" | "m2ts" | "mpg" | "mp4" | "m4s" | "mkv" | "webm" | "flv" | "ps" | "mxf"
    ) {
        return None;
    }
    // A capture whose size makes six full repackagings pointless is still
    // covered by the streaming probe above.
    let container = match transmux::cli::detect_container(&fx.data) {
        Ok(c) => c.name().to_string(),
        Err(e) => {
            return Some(json!({ "container": format!("err:{e}") }));
        }
    };
    let mut out = Map::new();
    out.insert("container".into(), json!(container));
    for fmt in [
        OutputFormat::Cmaf,
        OutputFormat::Hls,
        OutputFormat::TsHls,
        OutputFormat::Dash,
        OutputFormat::Ts,
        OutputFormat::Progressive,
    ] {
        let opts = Opts {
            format: fmt,
            ..Opts::default()
        };
        let v = match run_bytes(&fx.data, &opts) {
            Err(e) => json!({ "err": format!("{e}") }),
            Ok(Output::Bytes(b)) => json!({ "bytes": b.len(), "sha256": sha(&b) }),
            Ok(Output::Manifest { text, segments }) => {
                let segs: Map<String, Value> = segments
                    .iter()
                    .map(|(n, b)| (n.clone(), json!({ "len": b.len(), "sha256": sha(b) })))
                    .collect();
                json!({ "manifest_sha256": sha(text.as_bytes()), "manifest_len": text.len(),
                        "segments": segs })
            }
            Ok(_) => json!({ "other_output": true }),
        };
        out.insert(fmt.name().into(), v);
    }
    Some(Value::Object(out))
}

// ---------------------------------------------------------------------------
// dvb-si
// ---------------------------------------------------------------------------

use broadcast_common::Serialize;
use dvb_si::demux::SiDemux;
use dvb_si::descriptors::{AnyDescriptor, DescriptorLoop, parse_loop};
use dvb_si::tables::AnyTableSection;

fn reserialize_section(t: &AnyTableSection<'_>) -> Option<Result<Vec<u8>, String>> {
    macro_rules! ser {
        ($t:expr) => {{
            let mut b = vec![0u8; $t.serialized_len()];
            Some(
                $t.serialize_into(&mut b)
                    .map(|_| b)
                    .map_err(|e| format!("{e:?}")),
            )
        }};
    }
    match t {
        AnyTableSection::PatSection(t) => ser!(t),
        AnyTableSection::CatSection(t) => ser!(t),
        AnyTableSection::PmtSection(t) => ser!(t),
        AnyTableSection::TsdtSection(t) => ser!(t),
        AnyTableSection::DsmccSection(t) => ser!(t),
        AnyTableSection::NitSection(t) => ser!(t),
        AnyTableSection::SdtSection(t) => ser!(t),
        AnyTableSection::BatSection(t) => ser!(t),
        AnyTableSection::UntSection(t) => ser!(t),
        AnyTableSection::IntSection(t) => ser!(t),
        AnyTableSection::SatSection(t) => ser!(t),
        AnyTableSection::EitSection(t) => ser!(t),
        AnyTableSection::TdtSection(t) => ser!(t),
        AnyTableSection::RstSection(t) => ser!(t),
        AnyTableSection::StSection(t) => ser!(t),
        AnyTableSection::TotSection(t) => ser!(t),
        AnyTableSection::AitSection(t) => ser!(t),
        AnyTableSection::ContainerSection(t) => ser!(t),
        AnyTableSection::RctSection(t) => ser!(t),
        AnyTableSection::CitSection(t) => ser!(t),
        AnyTableSection::MpeFecSection(t) => ser!(t),
        AnyTableSection::RntSection(t) => ser!(t),
        AnyTableSection::MpeIfecSection(t) => ser!(t),
        AnyTableSection::ProtectionMessage(t) => ser!(t),
        AnyTableSection::DownloadableFontInfo(t) => ser!(t),
        AnyTableSection::DitSection(t) => ser!(t),
        AnyTableSection::SitSection(t) => ser!(t),
        AnyTableSection::MpeDatagram(t) => ser!(t),
        _ => None,
    }
}

#[derive(Default)]
struct SecStat {
    sections: u64,
    parse_ok: u64,
    parse_err: u64,
    typed: u64,
    identical: u64,
    mismatch: u64,
    ser_err: u64,
    bad_crc: u64,
}

impl SecStat {
    fn to_json(&self) -> Value {
        json!({
            "sections": self.sections, "parse_ok": self.parse_ok, "parse_err": self.parse_err,
            "typed": self.typed, "identical": self.identical, "mismatch": self.mismatch,
            "ser_err": self.ser_err, "bad_crc": self.bad_crc,
        })
    }
}

#[derive(Default)]
struct DescStat {
    typed: u64,
    unknown: u64,
    err: u64,
    identical: u64,
    mismatch: u64,
    ser_err: u64,
}

fn desc_loops<'a>(t: &'a AnyTableSection<'a>) -> Vec<&'a DescriptorLoop<'a>> {
    let mut loops: Vec<&DescriptorLoop<'_>> = Vec::new();
    match t {
        AnyTableSection::PmtSection(p) => {
            loops.push(&p.program_info);
            loops.extend(p.streams.iter().map(|x| &x.es_info));
        }
        AnyTableSection::SdtSection(p) => loops.extend(p.services.iter().map(|x| &x.descriptors)),
        AnyTableSection::NitSection(p) => {
            loops.push(&p.network_descriptors);
            loops.extend(p.transport_streams.iter().map(|x| &x.descriptors));
        }
        AnyTableSection::BatSection(p) => {
            loops.push(&p.bouquet_descriptors);
            loops.extend(p.transport_streams.iter().map(|x| &x.descriptors));
        }
        AnyTableSection::EitSection(p) => loops.extend(p.events.iter().map(|x| &x.descriptors)),
        AnyTableSection::CatSection(p) => loops.push(&p.descriptors),
        AnyTableSection::TsdtSection(p) => loops.push(&p.descriptors),
        AnyTableSection::TotSection(p) => loops.push(&p.descriptors),
        _ => {}
    }
    loops
}

fn survey_descs(descs: &mut BTreeMap<String, DescStat>, t: &AnyTableSection<'_>) {
    for l in desc_loops(t) {
        for (tag, body) in l.raw_tags() {
            let mut raw = vec![tag, body.len() as u8];
            raw.extend_from_slice(body);
            let first = parse_loop(&raw).next();
            match first {
                Some(Ok(AnyDescriptor::Unknown { .. })) => {
                    descs
                        .entry(format!("0x{tag:02X} UNKNOWN"))
                        .or_default()
                        .unknown += 1;
                }
                Some(Ok(d)) => {
                    let key = if tag == 0x7F {
                        format!(
                            "0x{tag:02X}/ext 0x{:02X} {}",
                            body.first().copied().unwrap_or(0),
                            d.name()
                        )
                    } else {
                        format!("0x{tag:02X} {}", d.name())
                    };
                    let st = descs.entry(key).or_default();
                    st.typed += 1;
                    // `AnyDescriptor::reserialize` does not exist in the
                    // published baseline: identity counts are HEAD-only and
                    // reported under `head_only_*` (ignored by diff.py).
                    #[cfg(feature = "head")]
                    match d.reserialize() {
                        None => {}
                        Some(Err(_)) => st.ser_err += 1,
                        Some(Ok(v)) if v == raw => st.identical += 1,
                        Some(Ok(_)) => st.mismatch += 1,
                    }
                    #[cfg(feature = "baseline")]
                    let _ = &raw;
                }
                Some(Err(_)) | None => {
                    descs
                        .entry(format!("0x{tag:02X} PARSE_ERR"))
                        .or_default()
                        .err += 1;
                }
            }
        }
    }
}

pub fn dvb_si_sections(fx: &Fixture, _: &Cfg) -> Option<Value> {
    if !fx.is_ts_ext() {
        return None;
    }
    let packets = ts_packets(&fx.data)?;
    // Pass 1: PIDs that carry sections outside the well-known SI set.
    let mut extra: BTreeSet<u16> = BTreeSet::new();
    {
        let mut d = SiDemux::builder().build();
        for pkt in &packets {
            for ev in d.feed(pkt) {
                if let Ok(AnyTableSection::PmtSection(pmt)) = ev.table_section() {
                    for st in &pmt.streams {
                        if [0x05u8, 0x0A, 0x0B, 0x0C, 0x0D].contains(&st.stream_type.to_u8()) {
                            extra.insert(st.elementary_pid.raw());
                        }
                    }
                }
            }
        }
    }
    let mut out = Map::new();
    for (label, repeats) in [("gated", false), ("all", true)] {
        let mut b = SiDemux::builder().emit_repeats(repeats);
        for pid in &extra {
            b = b.pid(mpeg_ts::pid::Pid::new(*pid));
        }
        let mut demux = b.build();
        let mut secs: BTreeMap<String, SecStat> = BTreeMap::new();
        let mut descs: BTreeMap<String, DescStat> = BTreeMap::new();
        for pkt in &packets {
            for ev in demux.feed(pkt) {
                let orig: &[u8] = ev.bytes().as_ref();
                let key = format!("{}:{:02X}", hx(ev.pid()), ev.table_id());
                let st = secs.entry(key).or_default();
                st.sections += 1;
                if !ev.crc_ok() {
                    st.bad_crc += 1;
                }
                match ev.table_section() {
                    Err(_) => st.parse_err += 1,
                    Ok(t) => {
                        st.parse_ok += 1;
                        match reserialize_section(&t) {
                            None => {}
                            Some(r) => {
                                st.typed += 1;
                                match r {
                                    Err(_) => st.ser_err += 1,
                                    Ok(v) if v == orig => st.identical += 1,
                                    Ok(_) => st.mismatch += 1,
                                }
                            }
                        }
                        if repeats {
                            survey_descs(&mut descs, &t);
                        }
                    }
                }
            }
        }
        let secs: Map<String, Value> = secs.into_iter().map(|(k, v)| (k, v.to_json())).collect();
        out.insert(format!("sections_{label}"), Value::Object(secs));
        if repeats {
            let d: Map<String, Value> = descs
                .into_iter()
                .map(|(k, v)| {
                    (
                        k,
                        json!({ "typed": v.typed, "unknown": v.unknown, "err": v.err,
                                "head_only_identical": v.identical,
                                "head_only_mismatch": v.mismatch,
                                "head_only_ser_err": v.ser_err }),
                    )
                })
                .collect();
            out.insert("descriptors".into(), Value::Object(d));
        }
    }
    Some(Value::Object(out))
}

/// Standalone section `.bin` fixtures (a PMT, an AIT, ...): parse through the
/// dispatcher and re-serialize.
pub fn dvb_si_section_bin(fx: &Fixture, _: &Cfg) -> Option<Value> {
    if fx.ext != "bin" || fx.data.len() < 8 || fx.data.len() > 4096 {
        return None;
    }
    let sec_len = (usize::from(fx.data[1] & 0x0F) << 8) | usize::from(fx.data[2]);
    if sec_len + 3 != fx.data.len() {
        return None;
    }
    Some(match AnyTableSection::parse(&fx.data) {
        Err(e) => json!({ "parse_err": variant_name(&format!("{e:?}")) }),
        Ok(t) => {
            let ser = match reserialize_section(&t) {
                None => "n/a".to_string(),
                Some(Err(e)) => format!("ser_err:{}", variant_name(&e)),
                Some(Ok(v)) if v == fx.data => "identical".into(),
                Some(Ok(_)) => "mismatch".into(),
            };
            let mut descs: BTreeMap<String, DescStat> = BTreeMap::new();
            survey_descs(&mut descs, &t);
            let d: Map<String, Value> = descs
                .into_iter()
                .map(|(k, v)| {
                    (
                        k,
                        json!({ "typed": v.typed, "unknown": v.unknown, "err": v.err,
                                "head_only_identical": v.identical,
                                "head_only_mismatch": v.mismatch,
                                "head_only_ser_err": v.ser_err }),
                    )
                })
                .collect();
            json!({ "table": t.name(), "reserialize": ser, "descriptors": d })
        }
    })
}

// ---------------------------------------------------------------------------
// dvb-t2mi
// ---------------------------------------------------------------------------

#[cfg(feature = "head")]
fn mkpid(v: u16) -> mpeg_ts::pid::Pid {
    mpeg_ts::pid::Pid::new(v)
}
#[cfg(feature = "baseline")]
fn mkpid(v: u16) -> u16 {
    v
}

pub fn dvb_t2mi(fx: &Fixture, _: &Cfg) -> Option<Value> {
    if !fx.is_ts_ext() {
        return None;
    }
    let packets = ts_packets(&fx.data)?;
    // PIDs seen, by packet count; try every PID with enough packets.
    let mut counts: BTreeMap<u16, u64> = BTreeMap::new();
    for p in &packets {
        *counts
            .entry((u16::from(p[1] & 0x1F) << 8) | u16::from(p[2]))
            .or_insert(0) += 1;
    }
    let mut out = Map::new();
    for (&pid, &n) in &counts {
        if n < 4 || pid == 0x1FFF || pid == 0 {
            continue;
        }
        let mut pump = dvb_t2mi::pump::T2miPump::new(mkpid(pid));
        let mut types: BTreeMap<String, u64> = BTreeMap::new();
        let (mut payload_ok, mut payload_err, mut header_err) = (0u64, 0u64, 0u64);
        let mut ind_addr = 0u64;
        for p in &packets {
            if ((u16::from(p[1] & 0x1F) << 8) | u16::from(p[2])) != pid {
                continue;
            }
            for ev in pump.feed_ts(p) {
                bump(&mut types, format!("{:02X}", ev.packet_type()));
                if ev.header().is_err() {
                    header_err += 1;
                }
                match ev.payload() {
                    Ok(pl) => {
                        payload_ok += 1;
                        if variant_name(&format!("{pl:?}")) == "IndividualAddressing" {
                            ind_addr += 1;
                        }
                    }
                    Err(_) => payload_err += 1,
                }
            }
        }
        let s = pump.stats();
        if s.t2mi_packets == 0 && s.crc_failures == 0 {
            continue;
        }
        // Inner TS recovery on the same PID.
        let mut rec = dvb_t2mi::inner_ts::InnerTsRecovery::new(mkpid(pid));
        let mut inner = Vec::new();
        for p in &packets {
            if ((u16::from(p[1] & 0x1F) << 8) | u16::from(p[2])) != pid {
                continue;
            }
            for ip in rec.feed(p) {
                inner.extend_from_slice(ip);
            }
        }
        #[cfg(feature = "head")]
        let plps: Vec<u8> = rec.plps_seen().collect();
        #[cfg(feature = "baseline")]
        let plps: Vec<u8> = Vec::new();
        let rs = rec.stats();
        out.insert(
            format!("pid_{pid:04X}"),
            json!({
                "ts_packets": s.ts_packets, "t2mi_packets": s.t2mi_packets,
                "crc_failures": s.crc_failures, "malformed": s.malformed_packets,
                "types": types, "payload_ok": payload_ok, "payload_err": payload_err,
                "header_err": header_err, "individual_addressing": ind_addr,
                "inner_bytes": inner.len(), "inner_sha256": sha(&inner),
                "head_only_plps": plps, "inner_t2mi_packets": rs.t2mi_packets,
                "inner_crc_failures": rs.crc_failures, "inner_malformed": rs.malformed_packets,
            }),
        );
    }
    Some(Value::Object(out))
}

// ---------------------------------------------------------------------------
// mpeg-ts
// ---------------------------------------------------------------------------

pub fn mpeg_ts(fx: &Fixture, _: &Cfg) -> Option<Value> {
    if !fx.is_ts_ext() {
        return None;
    }
    let packets = ts_packets(&fx.data)?;
    let (mut err, mut tei, mut pusi, mut af_err, mut pcrs) = (0u64, 0u64, 0u64, 0u64, 0u64);
    let (mut first_pcr, mut last_pcr) = (None, None);
    let mut pids: BTreeMap<String, u64> = BTreeMap::new();
    let mut cc_gaps = 0u64;
    let mut last_cc: BTreeMap<String, u8> = BTreeMap::new();
    for raw in &packets {
        let pkt = match mpeg_ts::ts::TsPacket::parse(raw) {
            Ok(p) => p,
            Err(_) => {
                err += 1;
                continue;
            }
        };
        let key = hx(pkt.header.pid);
        bump(&mut pids, key.clone());
        tei += u64::from(pkt.header.tei);
        pusi += u64::from(pkt.header.pusi);
        if pkt.header.has_payload {
            if let Some(&prev) = last_cc.get(&key) {
                if (prev + 1) & 0x0F != pkt.header.continuity_counter {
                    cc_gaps += 1;
                }
            }
            last_cc.insert(key, pkt.header.continuity_counter);
        }
        if let Some(af) = pkt.adaptation_field() {
            match af {
                Err(_) => af_err += 1,
                Ok(af) => {
                    if let Some(p) = af.pcr {
                        pcrs += 1;
                        first_pcr.get_or_insert(p.as_27mhz());
                        last_pcr = Some(p.as_27mhz());
                    }
                }
            }
        }
    }
    let flat: Vec<u8> = packets.iter().flat_map(|p| p.iter().copied()).collect();
    let mut resync_counts = Vec::new();
    for chunk in [flat.len().max(1), 7, 1000] {
        let mut r = mpeg_ts::resync::TsResync::new();
        let mut n = 0usize;
        for c in flat.chunks(chunk) {
            n += r.feed(c).len();
        }
        resync_counts.push(n);
    }
    Some(json!({
        "packets": packets.len(), "parse_err": err, "tei": tei, "pusi": pusi,
        "af_err": af_err, "pcr_count": pcrs, "first_pcr": first_pcr, "last_pcr": last_pcr,
        "cc_gaps": cc_gaps, "pids": pids, "resync_counts": resync_counts,
    }))
}

// ---------------------------------------------------------------------------
// scte35
// ---------------------------------------------------------------------------

fn scte35_one(bytes: &[u8], stats: &mut BTreeMap<String, u64>) {
    use broadcast_common::Parse;
    match scte35_splice::SpliceInfoSection::parse(bytes) {
        Err(e) => bump(stats, format!("err:{}", variant_name(&format!("{e:?}")))),
        Ok(s) => {
            bump(stats, "ok");
            let mut b = vec![0u8; s.serialized_len()];
            match s.serialize_into(&mut b) {
                Ok(_) if b == bytes => bump(stats, "identical"),
                Ok(_) => bump(stats, "mismatch"),
                Err(_) => bump(stats, "ser_err"),
            }
        }
    }
}

pub fn scte35(fx: &Fixture, _: &Cfg) -> Option<Value> {
    let mut stats: BTreeMap<String, u64> = BTreeMap::new();
    if fx.is_ts_ext() {
        let packets = ts_packets(&fx.data)?;
        let mut pids: BTreeSet<u16> = BTreeSet::new();
        {
            let mut d = SiDemux::builder().build();
            for pkt in &packets {
                for ev in d.feed(pkt) {
                    if let Ok(AnyTableSection::PmtSection(pmt)) = ev.table_section() {
                        for st in &pmt.streams {
                            if st.stream_type.to_u8() == 0x86 {
                                pids.insert(st.elementary_pid.raw());
                            }
                        }
                    }
                }
            }
        }
        if pids.is_empty() {
            return None;
        }
        let mut b = SiDemux::builder().emit_repeats(true);
        for p in &pids {
            b = b.pid(mpeg_ts::pid::Pid::new(*p));
        }
        let mut d = b.build();
        for pkt in &packets {
            for ev in d.feed(pkt) {
                if ev.table_id() == 0xFC {
                    scte35_one(ev.bytes().as_ref(), &mut stats);
                }
            }
        }
    } else if fx.ext == "bin" && fx.data.first() == Some(&0xFC) {
        scte35_one(&fx.data, &mut stats);
    } else {
        return None;
    }
    Some(json!(stats))
}

// ---------------------------------------------------------------------------
// st291
// ---------------------------------------------------------------------------

pub fn st291(fx: &Fixture, _: &Cfg) -> Option<Value> {
    if fx.ext != "bin" || !fx.rel.contains("st291/") {
        return None;
    }
    if fx.rel.ends_with("anc_rtp.bin") {
        return Some(match st291::AncRtpPayload::parse_rtp_packet(&fx.data) {
            Ok((_, p)) => json!({ "rtp": "ok", "payload": variant_name(&format!("{p:?}")) }),
            Err(e) => json!({ "rtp": format!("err:{}", variant_name(&format!("{e:?}"))) }),
        });
    }
    Some(match st291::AncDataPacket::parse(&fx.data) {
        Err(e) => json!({ "anc": format!("err:{}", variant_name(&format!("{e:?}"))) }),
        Ok(p) => {
            let mut b = vec![0u8; p.serialized_len()];
            let ser = match p.serialize_into(&mut b) {
                Ok(n) if b[..n] == fx.data[..] => "identical",
                Ok(_) => "mismatch",
                Err(_) => "ser_err",
            };
            json!({ "anc": "ok", "reserialize": ser })
        }
    })
}

// ---------------------------------------------------------------------------
// media-doctor
// ---------------------------------------------------------------------------

fn findings_json(report: &media_doctor::Report) -> Value {
    let mut by_rule: BTreeMap<String, u64> = BTreeMap::new();
    let mut lines: Vec<String> = Vec::new();
    for f in report.findings() {
        bump(&mut by_rule, format!("{}:{}", f.severity.name(), f.rule_id));
        lines.push(format!(
            "{}|{}|{}|{}|{}",
            f.severity.name(),
            f.rule_id,
            f.location.packet,
            f.location.pid,
            f.message
        ));
    }
    json!({ "total": report.len(), "by_rule": by_rule, "findings_sha256": sha(lines.join("\n").as_bytes()),
            "first": lines.iter().take(5).collect::<Vec<_>>() })
}

pub fn media_doctor(fx: &Fixture, _: &Cfg) -> Option<Value> {
    use media_doctor::*;
    if fx.is_ts_ext() {
        let packets = ts_packets(&fx.data)?;
        let flat: Vec<u8> = packets.iter().flat_map(|p| p.iter().copied()).collect();
        let mut report = Report::default();
        let diags: [&dyn Diagnostic; 10] = [
            &SyncByteCheck,
            &PcrCheck,
            &PtsCheck,
            &CcAnomalyCheck,
            &Scte35Check,
            &PatPmtVersionCheck,
            &ParamSetsCheck,
            &InterlaceCheck,
            &FpsCadenceCheck,
            &CodecSignallingCheck,
        ];
        run_all(&flat, &diags, &mut report);
        return Some(findings_json(&report));
    }
    if matches!(fx.ext.as_str(), "mp4" | "m4s") {
        let mut report = Report::default();
        check_container_codec(&fx.data, &mut report);
        return Some(findings_json(&report));
    }
    if fx.ext == "m3u8" {
        let text = std::str::from_utf8(&fx.data).ok()?;
        let mut report = Report::default();
        check_hls_playlist(text, &mut report);
        return Some(findings_json(&report));
    }
    if fx.ext == "mpd" {
        let text = std::str::from_utf8(&fx.data).ok()?;
        let mut report = Report::default();
        check_dash_mpd(text, &mut report);
        return Some(findings_json(&report));
    }
    None
}

// ---------------------------------------------------------------------------
// HLS / DASH syntax
// ---------------------------------------------------------------------------

pub fn hls(fx: &Fixture, _: &Cfg) -> Option<Value> {
    if fx.ext != "m3u8" {
        return None;
    }
    let text = std::str::from_utf8(&fx.data).ok()?;
    let is_master = text.contains("#EXT-X-STREAM-INF");
    Some(if is_master {
        match broadcast_hls::MasterPlaylist::parse(text) {
            Err(e) => json!({ "kind": "master", "err": variant_name(&format!("{e:?}")) }),
            Ok(p) => match p.to_m3u8() {
                Err(e) => json!({ "kind": "master", "ser_err": variant_name(&format!("{e:?}")) }),
                Ok(s) => {
                    json!({ "kind": "master", "identical": s == text, "rendered_sha256": sha(s.as_bytes()) })
                }
            },
        }
    } else {
        match broadcast_hls::MediaPlaylist::parse(text) {
            Err(e) => json!({ "kind": "media", "err": variant_name(&format!("{e:?}")) }),
            Ok(p) => match p.to_m3u8() {
                Err(e) => json!({ "kind": "media", "ser_err": variant_name(&format!("{e:?}")) }),
                Ok(s) => {
                    json!({ "kind": "media", "identical": s == text, "rendered_sha256": sha(s.as_bytes()) })
                }
            },
        }
    })
}

pub fn dash(fx: &Fixture, _: &Cfg) -> Option<Value> {
    if fx.ext != "mpd" {
        return None;
    }
    let text = std::str::from_utf8(&fx.data).ok()?;
    Some(match transmux::Mpd::parse(text) {
        Err(e) => json!({ "err": variant_name(&format!("{e:?}")) }),
        Ok(m) => json!({ "ok": true, "periods": m.periods.len() }),
    })
}

// ---------------------------------------------------------------------------
// CLI binaries (subprocess), only when --bin-dir is given
// ---------------------------------------------------------------------------

/// Run `bin` with `cwd` as its working directory. Inputs/outputs are passed as
/// bare relative names so the process id (different for the two sweeps) never
/// leaks into the captured output.
fn run_bin(
    bin: &std::path::Path,
    cwd: &std::path::Path,
    args: &[String],
    timeout: std::time::Duration,
) -> Value {
    use std::process::{Command, Stdio};
    let so = cwd.join("stdout.txt");
    let se = cwd.join("stderr.txt");
    let (Ok(fo), Ok(fe)) = (std::fs::File::create(&so), std::fs::File::create(&se)) else {
        return json!({ "harness_error": "tmp files" });
    };
    let child = Command::new(bin)
        .current_dir(cwd)
        .args(args)
        .stdout(Stdio::from(fo))
        .stderr(Stdio::from(fe))
        .spawn();
    let Ok(mut child) = child else {
        return json!({ "harness_error": "spawn" });
    };
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
            Err(_) => break None,
        }
    };
    let stdout = std::fs::read(&so).unwrap_or_default();
    let stderr = std::fs::read(&se).unwrap_or_default();
    match status {
        None => json!({ "timeout": true }),
        Some(s) => json!({
            "exit": s.code(), "stdout_len": stdout.len(), "stdout_sha256": sha(&stdout),
            "stderr_first": String::from_utf8_lossy(&stderr).lines().next().unwrap_or("").to_string(),
        }),
    }
}

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("sweep-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::create_dir_all(&d);
    d
}

fn exe(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let p = dir.join(name);
    p.is_file().then_some(p)
}

pub fn cli_dvb_tools(fx: &Fixture, cfg: &Cfg) -> Option<Value> {
    if !fx.is_ts_ext() || fx.data.len() > 30_000_000 {
        return None;
    }
    let bin = exe(cfg.bin_dir.as_deref()?, "dvb-tools")?;
    let cwd = scratch_dir("dvb-tools");
    std::fs::write(cwd.join("input.ts"), &fx.data).ok()?;
    let path = "input.ts".to_string();
    let mut out = Map::new();
    for (name, args) in [
        ("dump_json", vec!["dump", &path, "--json"]),
        ("services", vec!["services", &path]),
        ("epg", vec!["epg", &path]),
        ("pids", vec!["pids", &path]),
        ("t2mi", vec!["t2mi", &path]),
    ] {
        let args: Vec<String> = args.into_iter().map(String::from).collect();
        out.insert(
            name.into(),
            run_bin(&bin, &cwd, &args, std::time::Duration::from_secs(120)),
        );
    }
    let _ = std::fs::remove_dir_all(&cwd);
    Some(Value::Object(out))
}

pub fn cli_ts_fix(fx: &Fixture, cfg: &Cfg) -> Option<Value> {
    if !fx.is_ts_ext() || fx.data.len() > 30_000_000 {
        return None;
    }
    let bin = exe(cfg.bin_dir.as_deref()?, "ts-fix")?;
    let cwd = scratch_dir("ts-fix");
    let output = cwd.join("output.ts");
    std::fs::write(cwd.join("input.ts"), &fx.data).ok()?;
    let mut out = Map::new();
    let variants: [(&str, &[&str]); 5] = [
        ("repair_continuity", &["--repair-continuity"]),
        ("regen_psi", &["--regen-psi"]),
        ("drop_nulls", &["--drop-nulls"]),
        (
            "combo",
            &["--repair-continuity", "--regen-psi", "--drop-nulls"],
        ),
        ("restamp_interpolate", &["--restamp-pcr-interpolate"]),
    ];
    for (name, flags) in variants {
        let _ = std::fs::remove_file(&output);
        let mut args: Vec<String> = vec![
            "--input".into(),
            "input.ts".into(),
            "--output".into(),
            "output.ts".into(),
        ];
        args.extend(flags.iter().map(|s| (*s).to_string()));
        let mut v = run_bin(&bin, &cwd, &args, std::time::Duration::from_secs(120));
        if let Ok(bytes) = std::fs::read(&output) {
            if let Value::Object(m) = &mut v {
                m.insert("output_len".into(), json!(bytes.len()));
                m.insert("output_packets".into(), json!(bytes.len() / 188));
                m.insert("output_sha256".into(), json!(sha(&bytes)));
            }
        }
        out.insert(name.into(), v);
    }
    let _ = std::fs::remove_dir_all(&cwd);
    Some(Value::Object(out))
}
