//! Local-archive evidence for reserved-bit defaults (audit §3 / #1197 item d).
//!
//! Keyed on the LOCAL-ONLY `.test-streams/` archive (the TSDuck BSD-2 clone's
//! live-captured streams are never committed). When a capture is absent the
//! test skips loudly, like `downloaded_lcn.rs`.
//!
//! * SSU `system_software_update_info` selector (TS 102 006 Table 4): the spec
//!   gives NO value for the two `reserved` runs (verified in the V1.4.1 text,
//!   `private/specs-md/etsi_ts_102_006_*.md`). A real PMT (tsduck `test-002.ts`)
//!   carries SSU descriptors whose reserved runs are ZERO - so parse must
//!   preserve them and re-serialize byte-identically (the default for a
//!   hand-built entry stays the all-ones DVB analogy; the real data is why
//!   preservation, not the default, is load-bearing here).
//! * `logical_channel_descriptor` v1 (EACEM, tag 0x83): the 1-bit `reserved`
//!   between `visible_service_flag` and the 14-bit LCN is `1` in every real
//!   capture in the archive (59 entries in the French TNT mux alone), so the
//!   `V1_RESERVED_DEFAULT = 1` default is evidence-backed.
#![cfg(feature = "ts")]

use broadcast_common::{Parse, Serialize};
use dvb_si::descriptors::data_broadcast_id::{
    DATA_BROADCAST_ID_SSU, DataBroadcastIdDescriptor, IdSelector,
};
use std::path::{Path, PathBuf};

fn archive(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".test-streams")
        .join(rel)
}

/// Complete PSI sections of `pid` (all packets of the file, PUSI-reassembled).
fn sections(ts: &[u8], pid: u16) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut cur: Option<Vec<u8>> = None;
    for pkt in ts.chunks_exact(188) {
        if pkt[0] != 0x47 {
            continue;
        }
        let p = (u16::from(pkt[1] & 0x1F) << 8) | u16::from(pkt[2]);
        if p != pid {
            continue;
        }
        let pusi = pkt[1] & 0x40 != 0;
        let afc = (pkt[3] >> 4) & 3;
        let mut off = 4;
        if afc == 3 {
            off += 1 + usize::from(pkt[4]);
        }
        if afc & 1 == 0 || off >= 188 {
            continue;
        }
        let pl = &pkt[off..];
        if pusi {
            let ptr = usize::from(pl[0]);
            if let Some(mut c) = cur.take() {
                c.extend_from_slice(&pl[1..1 + ptr]);
                out.push(c);
            }
            cur = Some(pl[1 + ptr..].to_vec());
        } else if let Some(c) = cur.as_mut() {
            c.extend_from_slice(pl);
        }
        if let Some(c) = cur.as_ref()
            && c.len() >= 3
        {
            let sl = (usize::from(c[1] & 0x0F) << 8) | usize::from(c[2]);
            if c.len() >= sl + 3 {
                let mut done = cur.take().unwrap();
                done.truncate(sl + 3);
                out.push(done);
            }
        }
    }
    out
}

#[test]
fn ssu_selector_reserved_runs_are_preserved_from_a_real_pmt() {
    let path = archive("tsduck-test/input/test-002.ts");
    let Ok(ts) = std::fs::read(&path) else {
        eprintln!("local_archive_reserved_defaults: SKIPPED - {path:?} not present");
        return;
    };
    let mut ssu_seen = 0;
    let mut zero_runs = 0;
    for sec in sections(&ts, 0x57).into_iter().filter(|s| s[0] == 0x02) {
        // Walk the whole section for data_broadcast_id descriptors (tag 0x66,
        // id 0x000A); a PMT has them in the ES loop.
        let mut i = 12;
        while i + 4 < sec.len().saturating_sub(4) {
            if sec[i] == 0x66 && sec[i + 2] == 0x00 && sec[i + 3] == 0x0A {
                let len = usize::from(sec[i + 1]);
                let bytes = &sec[i..i + 2 + len];
                if let Ok(d) = DataBroadcastIdDescriptor::parse(bytes)
                    && d.data_broadcast_id == DATA_BROADCAST_ID_SSU
                    && let Ok(IdSelector::Ssu(sel)) = d.id_selector_decoded()
                    && !sel.oui_entries.is_empty()
                {
                    ssu_seen += 1;
                    let mut out = vec![0u8; d.serialized_len()];
                    d.serialize_into(&mut out).unwrap();
                    assert_eq!(out, bytes, "SSU descriptor must round-trip byte-exact");
                    // Typed selector view re-emits the reserved runs verbatim.
                    let mut sel_bytes = vec![0u8; sel.serialized_len()];
                    sel.serialize_into(&mut sel_bytes).unwrap();
                    assert_eq!(sel_bytes, d.id_selector);
                    for e in &sel.oui_entries {
                        if e.reserved_4 == 0 && e.reserved_2 == 0 {
                            zero_runs += 1;
                        }
                    }
                }
                i += 2 + len;
            } else {
                i += 1;
            }
        }
    }
    assert!(ssu_seen > 0, "test-002.ts must carry SSU descriptors");
    assert!(
        zero_runs > 0,
        "the real SSU selector writes zero reserved runs; if this fails the \
         archive changed and the audit note must be re-derived"
    );
}

#[test]
fn logical_channel_v1_reserved_bit_is_one_in_real_captures() {
    let path = archive("france-tnt-uhf32.ts");
    let Ok(ts) = std::fs::read(&path) else {
        eprintln!("local_archive_reserved_defaults: SKIPPED - {path:?} not present");
        return;
    };
    let mut entries = 0usize;
    for sec in sections(&ts, 0x10)
        .into_iter()
        .filter(|s| s[0] == 0x40 || s[0] == 0x41)
    {
        let nl = (usize::from(sec[8] & 0x0F) << 8) | usize::from(sec[9]);
        let mut pos = 10 + nl;
        let tsl = (usize::from(sec[pos] & 0x0F) << 8) | usize::from(sec[pos + 1]);
        pos += 2;
        let end = (pos + tsl).min(sec.len() - 4);
        while pos + 6 <= end {
            let dl = (usize::from(sec[pos + 4] & 0x0F) << 8) | usize::from(sec[pos + 5]);
            let body = &sec[pos + 6..(pos + 6 + dl).min(sec.len())];
            let mut j = 0;
            while j + 2 <= body.len() {
                let (tag, l) = (body[j], usize::from(body[j + 1]));
                if tag == 0x83 {
                    for e in body[j + 2..(j + 2 + l).min(body.len())].chunks_exact(4) {
                        entries += 1;
                        assert_eq!((e[2] >> 6) & 1, 1, "v1 LCN reserved bit");
                    }
                }
                j += 2 + l;
            }
            pos += 6 + dl;
        }
    }
    assert!(entries > 0, "france-tnt-uhf32.ts NIT carries LCN entries");
}
