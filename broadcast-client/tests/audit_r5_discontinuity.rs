//! Round-5 finding F6: a discontinuity must not shrink the announced track set
//! or cost samples beyond the pre-discontinuity tail.

use std::collections::{BTreeMap, VecDeque};

use broadcast_client::{Action, Config, Event, Session};

const TS_PACKET: usize = 188;
const SEGMENTS: usize = 7;
const DISCONTINUITY_AT: usize = 4;
const WINDOW: usize = 4;
const START_VISIBLE: usize = 3;
const SEGMENT_US: u64 = 3_500_000;
const TICK_US: u64 = 100_000;
const RUN_US: u64 = 60_000_000;
const CHUNK: usize = 64 * 1024;
const PAT_PID: u16 = 0;
const PID_HIGH_MASK: u8 = 0x1f;
const PUSI_MASK: u8 = 0x40;

#[derive(Default, Debug)]
struct Outcome {
    last_track_set: Vec<u32>,
    pid_of: BTreeMap<u32, u16>,
    announced_pids: std::collections::BTreeSet<u16>,
    samples: usize,
    per_track: BTreeMap<u16, usize>,
    discontinuities: usize,
    errors: Vec<String>,
}

fn split_at_pat(b: &[u8], n: usize) -> Vec<Vec<u8>> {
    let pk = b.len() / TS_PACKET;
    let pats: Vec<usize> = (0..pk)
        .filter(|&i| {
            let p = &b[i * TS_PACKET..];
            u16::from(p[1] & PID_HIGH_MASK) << 8 | u16::from(p[2]) == PAT_PID
                && p[1] & PUSI_MASK != 0
        })
        .collect();
    let mut cuts = vec![0usize];
    for k in 1..n {
        let target = pk * k / n;
        if let Some(&c) = pats.iter().min_by_key(|&&c| c.abs_diff(target)) {
            if c > *cuts.last().unwrap() {
                cuts.push(c);
            }
        }
    }
    cuts.push(pk);
    cuts.windows(2)
        .map(|w| b[w[0] * TS_PACKET..w[1] * TS_PACKET].to_vec())
        .collect()
}

fn run(segs: &[Vec<u8>], disc_at: Option<usize>) -> Outcome {
    let n = segs.len();
    let playlist = |upto: usize, end: bool, first: usize| {
        let mut pl = format!(
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:{first}\n"
        );
        for i in first..upto {
            if Some(i) == disc_at {
                pl.push_str("#EXT-X-DISCONTINUITY\n");
            }
            pl.push_str(&format!("#EXTINF:3.5,\nhttp://h/s/seg{i}.ts\n"));
        }
        if end {
            pl.push_str("#EXT-X-ENDLIST\n");
        }
        pl
    };
    let mut s = Session::open("http://h/p/index.m3u8", Config::default(), 0).unwrap();
    let mut out = Outcome::default();
    let mut now = 0u64;
    while now <= RUN_US {
        let visible = (START_VISIBLE + (now / SEGMENT_US) as usize).min(n);
        let pl = playlist(visible, visible == n, visible.saturating_sub(WINDOW));
        let mut pending: VecDeque<(u32, String)> = VecDeque::new();
        loop {
            while let Some(a) = s.poll(now) {
                if let Action::HttpRequest { conn, url, .. } = a {
                    pending.push_back((conn, url));
                }
            }
            let Some((c, u)) = pending.pop_front() else {
                break;
            };
            let body: Vec<u8> = if u.ends_with(".m3u8") {
                pl.clone().into_bytes()
            } else {
                let i: usize = u
                    .trim_end_matches(".ts")
                    .rsplit("seg")
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap();
                segs[i].clone()
            };
            s.on_response(
                c,
                200,
                &[("Content-Length".into(), body.len().to_string())],
                now,
            );
            for chunk in body.chunks(CHUNK) {
                s.on_data(c, chunk, now);
            }
            s.on_closed(c, None, now);
            drain(&mut s, &mut out);
        }
        drain(&mut s, &mut out);
        now += TICK_US;
    }
    out
}

fn drain(s: &mut Session, out: &mut Outcome) {
    while let Some(e) = s.next_event() {
        match &e {
            Event::Tracks(v) => {
                out.last_track_set = v.iter().map(|t| t.track_id.value()).collect();
                out.pid_of = v
                    .iter()
                    .map(|t| (t.track_id.value(), t.source_pid.map_or(0, |p| p.value())))
                    .collect();
                out.announced_pids.extend(out.pid_of.values());
            }
            Event::Sample { track, .. } => {
                out.samples += 1;
                let pid = out.pid_of.get(&track.value()).copied().unwrap_or(0);
                *out.per_track.entry(pid).or_default() += 1;
            }
            Event::Discontinuity => out.discontinuities += 1,
            Event::Error { .. } => out.errors.push(format!("{e:?}")),
            _ => {}
        }
    }
}

const SEAM_TAIL_FRAMES: usize = 8;

fn assert_seam_costs_only_the_tail(bytes: &[u8], what: &str) {
    let b = &bytes[..bytes.len() / TS_PACKET * TS_PACKET];
    let segs = split_at_pat(b, SEGMENTS);
    assert!(
        segs.len() > DISCONTINUITY_AT + 1,
        "{what}: {} segments",
        segs.len()
    );
    let plain = run(&segs, None);
    let cut = run(&segs, Some(DISCONTINUITY_AT));
    assert!(cut.errors.is_empty(), "{what}: {:?}", cut.errors);
    assert_eq!(cut.discontinuities, 1, "{what}");
    assert!(
        !plain.announced_pids.is_empty(),
        "{what}: nothing announced"
    );
    assert_eq!(
        cut.announced_pids, plain.announced_pids,
        "{what}: the discontinuity changed the announced track set"
    );
    assert_eq!(
        cut.last_track_set.len(),
        plain.last_track_set.len(),
        "{what}: the final track set shrank"
    );
    for (pid, &want) in &plain.per_track {
        let got = cut.per_track.get(pid).copied().unwrap_or(0);
        assert!(
            want.abs_diff(got) <= SEAM_TAIL_FRAMES,
            "{what}: pid {pid:#x} delivered {got} samples across the seam, {want} without"
        );
    }
}

#[test]
fn a_discontinuity_costs_one_pes_tail_not_a_track_france_tnt() {
    let Some(b) = test_fixtures::require_bytes(
        "ts/france-tnt-uhf32-video-head-6MB.ts",
        "discontinuity_france_tnt",
    ) else {
        return;
    };
    assert_seam_costs_only_the_tail(&b, "france-tnt-uhf32-video-head-6MB");
}

#[test]
fn a_discontinuity_costs_one_pes_tail_not_a_track_france2() {
    assert_seam_costs_only_the_tail(&test_fixtures::read("ts/france2.ts"), "france2");
}

#[test]
fn a_discontinuity_in_a_two_pid_stream_loses_nothing() {
    let b = test_fixtures::read("ts/h264_aac_40s.ts");
    let segs = split_at_pat(&b[..b.len() / TS_PACKET * TS_PACKET], SEGMENTS);
    let plain = run(&segs, None);
    let cut = run(&segs, Some(DISCONTINUITY_AT));
    assert_eq!(cut.per_track, plain.per_track);
    assert_eq!(cut.announced_pids, plain.announced_pids);
}
