//! Play a static MPD from the file system with `DashClient` and print what a
//! host would receive: the selection, the tracks, per-track sample counts and
//! durations, Period changes and discontinuities.
//!
//! The "network" is `file:` URLs read with `std::fs` (byte ranges honoured),
//! so the example runs against any fixture directory with no server.
//!
//! # Usage
//!
//! ```bash
//! cargo run -p dash-runtime --example play_static
//! cargo run -p dash-runtime --example play_static -- --mpd fixtures/dash/multiperiod-restart/multiperiod.mpd
//! cargo run -p dash-runtime --example play_static -- --mpd fixtures/dash/ondemand-sidx/ondemand.mpd --window 2
//! ```

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;

use dash_runtime::client::{
    Action, DashClient, DashConfig, Error, Failure, FailureClass, Output, ResourceId,
};
use url::Url;

const DEFAULT_MPD: &str = "../fixtures/dash/timeline-gpac/timeline.mpd";
const MAX_STEPS: usize = 100_000;
const MICROS: f64 = 1_000_000.0;

struct Args {
    mpd: PathBuf,
    window: usize,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        mpd: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_MPD),
        window: DashConfig::default().max_inflight_segments,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--mpd" => args.mpd = it.next().ok_or("--mpd needs a path")?.into(),
            "--window" => {
                args.window = it
                    .next()
                    .ok_or("--window needs a number")?
                    .parse()
                    .map_err(|e| format!("--window: {e}"))?;
            }
            other => {
                return Err(format!(
                    "unknown argument {other} (use --mpd PATH, --window N)"
                ));
            }
        }
    }
    Ok(args)
}

fn read(url: &str, range: Option<(u64, u64)>) -> Result<Vec<u8>, Failure> {
    let path = Url::parse(url)
        .ok()
        .and_then(|u| u.to_file_path().ok())
        .ok_or(Failure::Status(404))?;
    let bytes = std::fs::read(path).map_err(|_| Failure::Status(404))?;
    match range {
        None => Ok(bytes),
        Some((offset, len)) => {
            let start = usize::try_from(offset).map_err(|_| Failure::Status(416))?;
            let end = start.saturating_add(usize::try_from(len).unwrap_or(usize::MAX));
            bytes
                .get(start..end.min(bytes.len()))
                .map(<[u8]>::to_vec)
                .ok_or(Failure::Status(416))
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    let path = std::fs::canonicalize(&args.mpd)?;
    let url = Url::from_file_path(&path).map_err(|()| "MPD path is not absolute")?;
    let mut config = DashConfig::default();
    config.max_inflight_segments = args.window;
    config.validate()?;
    let mut client = DashClient::new(url.as_str(), config);

    let mut pending: VecDeque<Action> = VecDeque::new();
    let mut samples: BTreeMap<u32, (usize, f64)> = BTreeMap::new();
    let mut discontinuities = 0usize;
    let mut now = 0u64;

    for _ in 0..MAX_STEPS {
        while let Some(action) = client.poll(now) {
            pending.push_back(action);
        }
        while let Some(output) = client.next_output() {
            match output {
                Output::Manifest(m) => println!(
                    "period {}: {} video variant(s), {} audio set(s)",
                    m.period,
                    m.variants.len(),
                    m.audio.len()
                ),
                Output::PeriodChanged {
                    index,
                    id,
                    start_us,
                    continuous,
                } => println!(
                    "  Period {index} ({}) starts at {:.3}s, continuous: {continuous}",
                    id.as_deref().unwrap_or("-"),
                    start_us as f64 / MICROS
                ),
                Output::Tracks(tracks) => {
                    for t in &tracks {
                        println!("  track {} timescale {}", t.track_id, t.timescale);
                    }
                }
                Output::Samples {
                    track_id,
                    samples: s,
                } => {
                    let e = samples.entry(track_id.value()).or_default();
                    e.0 += s.len();
                    e.1 += s
                        .iter()
                        .map(|x| f64::from(x.duration.unwrap_or(0)))
                        .sum::<f64>();
                }
                Output::Discontinuity => discontinuities += 1,
                Output::TrackDropped {
                    period, set, cause, ..
                } => println!("  dropped set {set} of period {period}: {cause}"),
                Output::Fatal(kind) => return Err(format!("fatal: {kind}").into()),
                Output::EndOfStream => {
                    println!("end of stream, {discontinuities} discontinuity marker(s)");
                    for (id, (count, ticks)) in &samples {
                        println!("  track {id}: {count} samples, {ticks} ticks");
                    }
                    return Ok(());
                }
                _ => {}
            }
        }
        let Some(action) = pending.pop_front() else {
            break;
        };
        now += 1_000;
        let outcome: Result<(), Error> = match action.id() {
            None => match read(action.url(), None) {
                Ok(body) => client.on_mpd(now, &body),
                Err(_) => return Err("the MPD could not be read".into()),
            },
            Some(id) => match read(action.url(), action.byte_range()) {
                Ok(body) => match id {
                    ResourceId::Init { .. } => client.on_init(now, id, &body),
                    ResourceId::Index { .. } => client.on_index(now, id, &body),
                    _ => client.on_segment(now, id, &body),
                },
                Err(failure) => {
                    if let FailureClass::Retry { .. } = client.on_failure(now, id, failure)? {
                        client.retry(now, id);
                    }
                    Ok(())
                }
            },
        };
        if let Err(Error::Rejected { id, class, .. }) = &outcome
            && let FailureClass::Retry { .. } = class
        {
            client.retry(now, *id);
        } else {
            outcome?;
        }
    }
    Err("the presentation did not finish".into())
}
