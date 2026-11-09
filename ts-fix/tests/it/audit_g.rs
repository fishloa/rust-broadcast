use mpeg_ts::resync::TsResync;
use ts_fix::{Stuffing, TsFix};

fn payload_packet(pid: u16, cc: u8, fill: u8) -> [u8; 188] {
    let mut p = [fill; 188];
    p[0] = 0x47;
    p[1] = ((pid >> 8) as u8) & 0x1F;
    p[2] = (pid & 0xFF) as u8;
    p[3] = 0x10 | (cc & 0x0F);
    p
}

fn run(engine: &mut TsFix, input: &[[u8; 188]]) -> Vec<[u8; 188]> {
    let mut out = Vec::new();
    for p in input {
        engine.push(p, |o| out.push(o.try_into().unwrap())).unwrap();
    }
    engine.finish(|o| out.push(o.try_into().unwrap()));
    out
}

#[test]
fn legal_duplicate_after_repaired_packet_is_not_renumbered_into_new_payload() {
    let a = payload_packet(0x100, 0, 0xA0);
    let b = payload_packet(0x100, 1, 0xA1);
    let c = payload_packet(0x100, 2, 0xA2);
    let d = payload_packet(0x100, 5, 0xD5);
    let d_dup = d;
    let e = payload_packet(0x100, 6, 0xE6);
    let mut engine = TsFix::builder().repair_continuity().build().unwrap();
    let out = run(&mut engine, &[a, b, c, d, d_dup, e]);
    let ccs: Vec<u8> = out.iter().map(|p| p[3] & 0x0F).collect();
    assert_eq!(
        ccs,
        vec![0, 1, 2, 3, 3, 4],
        "the byte-identical repeat of the repaired packet must stay a legal duplicate (same CC), not become a second distinct packet carrying the same payload"
    );
}

#[test]
fn pad_to_infinite_rate_terminates() {
    let mut engine = TsFix::builder()
        .stuffing(Stuffing::pad_to(f64::INFINITY))
        .build()
        .unwrap();
    let mut n = 0u64;
    engine
        .push(&payload_packet(0x100, 0, 0x11), |_| {
            n += 1;
            assert!(
                n < 100_000,
                "pad_to(inf) emitted {n} packets for one input packet and never terminates"
            );
        })
        .unwrap();
}

#[test]
fn resync_does_not_drop_aligned_tail_packets_at_end_of_input() {
    let mut input = Vec::new();
    for i in 0..10u8 {
        input.extend_from_slice(&payload_packet(0x100, i, 0x20 + i));
    }
    let mut bad = payload_packet(0x100, 10, 0x30);
    bad[0] = 0x00;
    input.extend_from_slice(&bad);
    for i in 11..14u8 {
        input.extend_from_slice(&payload_packet(0x100, i, 0x20 + i));
    }
    let mut r = TsResync::new();
    let pkts = r.feed(&input);
    r.finish();
    assert_eq!(
        pkts.len(),
        13,
        "10 good + 3 aligned good packets after one corrupt-sync packet; got {} (dropped_bytes={})",
        pkts.len(),
        r.stats().dropped_bytes
    );
}

#[test]
fn resync_emits_a_short_clean_stream() {
    let mut input = Vec::new();
    for i in 0..4u8 {
        input.extend_from_slice(&payload_packet(0x100, i, 0x20 + i));
    }
    let mut r = TsResync::new();
    let pkts = r.feed(&input);
    r.finish();
    assert_eq!(pkts.len(), 4, "a 4-packet aligned stream is emitted whole");
}
