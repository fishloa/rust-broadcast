//! Play a dynamic MPD with `DashClient` against a loopback origin and a
//! simulated clock, and print what a host receives: the clock the client
//! settled on, the join, the samples and the requests it made.
//!
//! The "network" is a synthetic `http://fixtures.invalid/` origin backed by the
//! MPD's directory and read with `std::fs`; a segment answers `404`
//! until the instant the MPD says it exists, computed here from the plan. The
//! clock is the simulation's: `--skew-ms` is how far the host's wall clock is
//! from true time (the client is told with `set_wall_clock`), `--seconds`
//! how long to run. No wall clock is read anywhere.
//!
//! # Usage
//!
//! ```bash
//! cargo run -p dash-runtime --example play_live
//! cargo run -p dash-runtime --example play_live -- --mpd fixtures/dash/live-timeline/live.mpd --seconds 4
//! cargo run -p dash-runtime --example play_live -- --skew-ms 1500
//! ```

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

use dash_runtime::address::{PlanOptions, plan_representation};
use dash_runtime::client::{
    Action, DashClient, DashConfig, Error, Failure, FailureClass, Output, PullLimits, ResourceId,
};
use dash_runtime::mpd::Mpd;
use dash_runtime::selection::RepRef;
use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use url::Url;

const DEFAULT_MPD: &str = "../fixtures/dash/live-number/live.mpd";
const DEFAULT_SECONDS: u64 = 3;
const START_AFTER_PUBLISH_US: i64 = 300_000;
const MICROS: u64 = 1_000_000;
const RTT_US: u64 = 40_000;
const ORIGIN: &str = "http://fixtures.invalid";

fn origin_path(root: &Path, url: &str) -> Option<PathBuf> {
    let url = Url::parse(url).ok()?;
    if url.scheme() != "http" || url.host_str() != Some("fixtures.invalid") {
        return None;
    }
    let mut path = root.to_path_buf();
    for segment in url.path_segments()? {
        let segment = percent_decode_str(segment).decode_utf8().ok()?;
        if segment.is_empty() || segment == "." || segment == ".." || segment.contains(['/', '\\'])
        {
            return None;
        }
        path.push(&*segment);
    }
    Some(path)
}

const MAX_STEPS: usize = 200_000;

struct Args {
    mpd: PathBuf,
    seconds: u64,
    skew_ms: i64,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        mpd: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_MPD),
        seconds: DEFAULT_SECONDS,
        skew_ms: 0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--mpd" => args.mpd = it.next().ok_or("--mpd needs a path")?.into(),
            "--seconds" => {
                args.seconds = it
                    .next()
                    .ok_or("--seconds needs a number")?
                    .parse()
                    .map_err(|e| format!("--seconds: {e}"))?;
            }
            "--skew-ms" => {
                args.skew_ms = it
                    .next()
                    .ok_or("--skew-ms needs a number")?
                    .parse()
                    .map_err(|e| format!("--skew-ms: {e}"))?;
            }
            other => {
                return Err(format!(
                    "unknown argument {other} (use --mpd PATH, --seconds N, --skew-ms N)"
                ));
            }
        }
    }
    Ok(args)
}

fn read(root: &Path, url: &str) -> Result<Vec<u8>, Failure> {
    let path = origin_path(root, url).ok_or(Failure::Status(404))?;
    std::fs::read(path).map_err(|_| Failure::Status(404))
}

fn availability(mpd: &Mpd, base: &Url) -> BTreeMap<String, i64> {
    let mut out = BTreeMap::new();
    for (pi, p) in mpd.periods.iter().enumerate() {
        for (si, s) in p.adaptation_sets.iter().enumerate() {
            for ri in 0..s.representations.len() {
                let Ok(planned) =
                    plan_representation(mpd, RepRef::at(pi, si, ri), &PlanOptions::default())
                else {
                    continue;
                };
                for seg in planned.plan.iter().take(10_000) {
                    let Some(url) = planned.candidates[0].resolve(Some(base), &seg.reference)
                    else {
                        continue;
                    };
                    if let Some(from) = seg.available_from_us {
                        out.insert(url, i64::try_from(from).unwrap_or(i64::MAX));
                    }
                }
            }
        }
    }
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    let path = std::fs::canonicalize(&args.mpd)?;
    let root = path
        .parent()
        .ok_or("MPD path has no directory")?
        .to_path_buf();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("MPD file name is not UTF-8")?;
    let url = Url::parse(&format!(
        "{ORIGIN}/{}",
        utf8_percent_encode(name, NON_ALPHANUMERIC)
    ))?;
    let mpd = Mpd::parse(&std::fs::read(&path)?)?;
    let publish = mpd.publish_time_us.ok_or("the MPD has no publishTime")?;
    let avail = availability(&mpd, &url);
    let true_start = publish + START_AFTER_PUBLISH_US;

    let mut client = DashClient::new(url.as_str(), DashConfig::default(), PullLimits::default());
    let host_unix = |elapsed: u64| {
        u64::try_from(true_start + i64::try_from(elapsed).unwrap_or(0) + args.skew_ms * 1_000)
            .unwrap_or(0)
    };
    client.set_wall_clock(0, host_unix(0));

    let mut pending: VecDeque<(u64, Action)> = VecDeque::new();
    let mut samples: BTreeMap<u32, usize> = BTreeMap::new();
    let mut requests = 0usize;
    let mut elapsed = 0u64;
    let end = args.seconds * MICROS;

    for _ in 0..MAX_STEPS {
        while let Some(action) = client.poll(elapsed) {
            pending.push_back((elapsed + RTT_US, action));
        }
        while let Some(output) = client.next_output() {
            match output {
                Output::ClockSynced {
                    source,
                    uncertainty_us,
                } => println!("clock: {source}, +-{uncertainty_us} us"),
                Output::PeriodChanged { id, .. } => {
                    println!("period {}", id.as_deref().unwrap_or("-"));
                }
                Output::Samples {
                    track_id,
                    samples: s,
                } => *samples.entry(track_id.value()).or_default() += s.len(),
                Output::Discontinuity => println!("discontinuity at {elapsed} us"),
                Output::Fatal(kind) => return Err(format!("fatal: {kind}").into()),
                Output::EndOfStream => break,
                _ => {}
            }
        }
        let due = pending.front().map(|p| p.0);
        let wake = client.next_wake();
        let next = [due, wake].into_iter().flatten().min().unwrap_or(end);
        if next >= end && due.is_none() {
            break;
        }
        elapsed = next.max(elapsed);
        let Some(&(at, _)) = pending.front() else {
            continue;
        };
        if at > elapsed {
            continue;
        }
        let Some((_, action)) = pending.pop_front() else {
            continue;
        };
        requests += 1;
        let true_utc = true_start + i64::try_from(elapsed).unwrap_or(0);
        let too_early = avail.get(action.url()).is_some_and(|a| true_utc < *a);
        let body = if too_early {
            Err(Failure::Status(404))
        } else {
            read(&root, action.url())
        };
        match (action.id(), body) {
            (None, Ok(b)) => {
                if let Err(e) = client.on_mpd(elapsed, &b) {
                    return Err(format!("MPD rejected: {e}").into());
                }
            }
            (None, Err(f)) => {
                client.on_mpd_failure(elapsed, f);
                client.retry_mpd(elapsed);
            }
            (Some(id), Err(f)) => {
                if let Ok(FailureClass::Retry { .. }) = client.on_failure(elapsed, id, f) {
                    client.retry(elapsed, id);
                }
            }
            (Some(id), Ok(b)) => {
                let r: Result<(), Error> = match id {
                    ResourceId::Init { .. } => client.on_init(elapsed, id, &b),
                    ResourceId::Index { .. } => client.on_index(elapsed, id, &b),
                    _ => client.on_segment(elapsed, id, &b),
                };
                if let Err(Error::Rejected { id, class, .. }) = r
                    && let FailureClass::Retry { .. } = class
                {
                    client.retry(elapsed, id);
                }
            }
        }
        if elapsed >= end {
            break;
        }
    }
    println!(
        "{requests} request(s) in {:.1} s of simulated time",
        elapsed as f64 / MICROS as f64
    );
    for (track, n) in &samples {
        println!("  track {track}: {n} samples");
    }
    Ok(())
}
