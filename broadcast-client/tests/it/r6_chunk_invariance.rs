use crate::support;

use broadcast_client::{Action, BufferEvent, Config, Event, Session};
use support::replay::TS_PACKET;

const SOAK_FIXED_CHUNKS: &[usize] = &[
    TS_PACKET,
    7 * TS_PACKET,
    4096,
    16 * 1024,
    64 * 1024,
    256 * 1024,
];
const FIXED_CHUNKS: &[usize] = &[TS_PACKET, 7 * TS_PACKET, 4096, 64 * 1024, 256 * 1024];
const SMALL_FILE_EXTRA_CHUNKS: &[usize] = &[1, 1316];
const SMALL_FILE: &str = "ts/h264_aac.ts";
const SOAK_RANDOM_SEEDS: &[u64] = &[1, 2, 3];
const RANDOM_SEEDS: &[u64] = &[1, 2];
const MAX_RANDOM_PACKETS: u64 = 1500;
const MICROS: u64 = 1_000_000;
const FAST_LINK_BPS: u64 = 40_000_000;
const LIVE_RATE_BPS: u64 = 6_000_000;

fn soak() -> bool {
    std::env::var_os("RUST_BROADCAST_SOAK").is_some()
}

// Chunk-invariant: the Sample sequence (track, pts, dts, join, length) and the position of every
// Discontinuity among the samples, for every read size and for arrival clocks fast enough that no
// tshttp timer fires. Deliberately excluded because they depend on when bytes arrive, not on what
// they contain: Buffer events (policy timing), Status/Starved (timers), and the grouping and timing
// of Tracks / DisplayChanged (the tracks hold releases at the first read boundary at which its
// condition holds, so a larger read can announce a later, larger set at once; the final set and
// the rule that every sample follows an announcement naming its track are still asserted). A clock
// slower than the stream's first sample trips the 15 s start timeout, which reconnects at a
// read-quantised byte offset; that is inherent and not exercised here.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Rec {
    Tracks(Vec<u32>),
    Sample {
        track: u32,
        pts: Option<i64>,
        dts: Option<i64>,
        join: String,
        len: usize,
    },
    Discontinuity,
    Buffer(&'static str),
    Other(String),
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

enum Clock {
    Fixed,
    Paced(u64),
}

fn drain(s: &mut Session, conn: &mut Option<u32>, now: u64, out: &mut Vec<Rec>) {
    while let Some(a) = s.poll(now) {
        if let Action::HttpRequest { conn: c, .. } = a {
            *conn = Some(c);
            s.on_response(
                c,
                200,
                &[("Content-Type".to_string(), "video/MP2T".to_string())],
                now,
            );
        }
    }
    while let Some(e) = s.next_event() {
        out.push(match e {
            Event::Tracks(t) => Rec::Tracks(t.iter().map(|x| x.track_id.value()).collect()),
            Event::Sample {
                track,
                sample,
                join,
            } => Rec::Sample {
                track: track.value(),
                pts: sample.pts,
                dts: sample.dts,
                join: format!("{join:?}"),
                len: sample.data.len(),
            },
            Event::Discontinuity => Rec::Discontinuity,
            Event::Buffer(b) => Rec::Buffer(match b {
                BufferEvent::Start { .. } => "start",
                BufferEvent::Pause { .. } => "pause",
                BufferEvent::Resume { .. } => "resume",
                _ => "other",
            }),
            other => Rec::Other(format!("{other:?}")),
        });
    }
}

fn run(bytes: &[u8], sizes: &mut dyn FnMut() -> usize, clock: &Clock) -> Vec<Rec> {
    let mut s = Session::open("http://h/live.ts", Config::default(), 0).unwrap();
    let mut conn = None;
    let mut out = Vec::new();
    let mut now = 0u64;
    drain(&mut s, &mut conn, now, &mut out);
    let mut p = 0usize;
    while p < bytes.len() {
        let n = sizes().min(bytes.len() - p);
        if let Clock::Paced(bps) = clock {
            now += (n as u64 * 8 * MICROS / bps).max(1);
        }
        s.on_data(conn.unwrap(), &bytes[p..p + n], now);
        p += n;
        drain(&mut s, &mut conn, now, &mut out);
    }
    s.on_closed(conn.unwrap(), None, now);
    drain(&mut s, &mut conn, now, &mut out);
    out
}

fn invariant_part(v: &[Rec]) -> Vec<Rec> {
    v.iter()
        .filter(|r| matches!(r, Rec::Sample { .. } | Rec::Discontinuity))
        .cloned()
        .collect()
}

fn samples_follow_their_announcement(v: &[Rec]) -> Result<(), String> {
    let mut current: Option<&Vec<u32>> = None;
    for (i, r) in v.iter().enumerate() {
        match r {
            Rec::Tracks(t) => current = Some(t),
            Rec::Sample { track, .. } if !current.is_some_and(|c| c.contains(track)) => {
                return Err(format!("sample on track {track} at {i} not in {current:?}"));
            }
            _ => {}
        }
    }
    Ok(())
}

fn final_tracks(v: &[Rec]) -> Option<&Vec<u32>> {
    v.iter().rev().find_map(|r| match r {
        Rec::Tracks(t) => Some(t),
        _ => None,
    })
}

fn first_difference(a: &[Rec], b: &[Rec]) -> String {
    let i = a.iter().zip(b).position(|(x, y)| x != y);
    match i {
        Some(i) => format!(
            "at {i}: {:?} vs {:?} (len {} vs {})",
            a[i],
            b[i],
            a.len(),
            b.len()
        ),
        None => format!("prefix equal, len {} vs {}", a.len(), b.len()),
    }
}

fn check(name: &str, bytes: &[u8]) {
    let bytes = &bytes[..bytes.len() / TS_PACKET * TS_PACKET];
    let soak = soak();
    let clocks: Vec<Clock> = if soak {
        vec![
            Clock::Fixed,
            Clock::Paced(FAST_LINK_BPS),
            Clock::Paced(LIVE_RATE_BPS),
        ]
    } else {
        vec![Clock::Fixed, Clock::Paced(LIVE_RATE_BPS)]
    };
    let mut fixed: Vec<usize> = if soak {
        SOAK_FIXED_CHUNKS
    } else {
        FIXED_CHUNKS
    }
    .to_vec();
    if name == SMALL_FILE {
        fixed.extend_from_slice(SMALL_FILE_EXTRA_CHUNKS);
    }
    let seeds = if soak {
        SOAK_RANDOM_SEEDS
    } else {
        RANDOM_SEEDS
    };
    for clock in clocks {
        let mut reference: Option<(String, Vec<Rec>)> = None;
        let mut cases: Vec<(String, Box<dyn FnMut() -> usize>)> = Vec::new();
        for &c in &fixed {
            cases.push((format!("fixed {c}"), Box::new(move || c)));
        }
        for &seed in seeds {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            cases.push((
                format!("random seed {seed}"),
                Box::new(move || {
                    (TS_PACKET as u64 * (1 + rng.next() % MAX_RANDOM_PACKETS)) as usize
                }),
            ));
        }
        for (label, mut sizes) in cases {
            let got = run(bytes, &mut sizes, &clock);
            let Some((rl, r)) = &reference else {
                reference = Some((label, got));
                continue;
            };
            let (ri, gi) = (invariant_part(r), invariant_part(&got));
            assert_eq!(
                ri,
                gi,
                "{name}: {rl} vs {label}: {}",
                first_difference(&ri, &gi)
            );
            assert_eq!(
                final_tracks(r),
                final_tracks(&got),
                "{name}: {rl} vs {label}: final track set"
            );
            if let Err(e) = samples_follow_their_announcement(&got) {
                panic!("{name}: {label}: {e}");
            }
        }
    }
}

macro_rules! private_capture {
    ($test:ident, $file:expr) => {
        #[test]
        fn $test() {
            let Some(bytes) = test_fixtures::require_bytes($file, "r6_chunk_invariance") else {
                return;
            };
            check($file, &bytes);
        }
    };
}

macro_rules! public_capture {
    ($test:ident, $file:expr) => {
        #[test]
        fn $test() {
            check($file, &test_fixtures::read($file));
        }
    };
}

private_capture!(tf1_is_chunk_invariant, "zenith-live/tf-1-8MB.ts");
private_capture!(arte_is_chunk_invariant, "zenith-live/arte-8MB.ts");
private_capture!(rai_gulp_is_chunk_invariant, "zenith-live/rai-gulp-8MB.ts");
public_capture!(h264_aac_is_chunk_invariant, "ts/h264_aac.ts");
public_capture!(h264_aac_40s_is_chunk_invariant, "ts/h264_aac_40s.ts");
public_capture!(france2_is_chunk_invariant, "ts/france2.ts");
public_capture!(gulli_opengop_is_chunk_invariant, "ts/gulli-opengop.ts");
public_capture!(
    france_pcr_discontinuity_is_chunk_invariant,
    "ts/france-pcr-discontinuity.ts"
);
