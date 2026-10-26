//! A real wire check against DASH-IF livesim2 (BSD-licensed, pinned): thirty seconds of live DASH
//! played through a `Session` over real HTTP on loopback, in six presentation shapes at once.
//!
//! The server is not part of this repository. The test finds the binary through `LIVESIM2_BIN`
//! (else `livesim2` on `PATH`) and its sample assets through `LIVESIM2_VODROOT` (else the module
//! cache of the pinned version). When either is missing the test SKIPS LOUDLY, and with
//! `REQUIRE_LIVESIM2=1` (the `dash-e2e` CI job) a missing server is a hard failure, never a skip,
//! the same rule as the private fixtures.

use std::collections::BTreeMap;
use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use broadcast_client::{Action, Config, DashClockPolicy, Event, Session};

const PINNED_VERSION: &str = "v1.15.2";
const MODULE_DIR: &str = "github.com/!dash-!industry-!forum/livesim2";
const ASSETS_UNDER_MODULE: &str = "cmd/livesim2/app/testdata/assets";
const VERSION_HEADER: &str = "Dash-If-Livesim2";
const READY_PROBE: &str = "/static/time.txt";
const REQUIRE_ENV: &str = "REQUIRE_LIVESIM2";
const BIN_ENV: &str = "LIVESIM2_BIN";
const VODROOT_ENV: &str = "LIVESIM2_VODROOT";
const BIN_NAME: &str = "livesim2";
const TOOL_DEADLINE: Duration = Duration::from_secs(20);
const READY_DEADLINE: Duration = Duration::from_secs(60);
const REQUEST_DEADLINE: Duration = Duration::from_secs(10);
const RUN: Duration = Duration::from_secs(30);
const IDLE_SLEEP: Duration = Duration::from_millis(15);
const MIN_SEGMENTS_PER_TRACK: usize = 10;
const SEGMENT_SECONDS: i64 = 2;
const MICROS: i64 = 1_000_000;
const STRICT_EARLY_TOLERANCE_US: i64 = 50_000;
const HOST_CLOCK_UNCERTAINTY_US: u64 = 5_000;
const MAX_ATTEMPTS_PER_SECOND: usize = 4;
const STATUS_TOO_EARLY: u16 = 425;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_GONE: u16 = 410;

struct Server {
    child: Child,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn require() -> bool {
    std::env::var(REQUIRE_ENV).is_ok_and(|v| v == "1")
}

fn bail(reason: &str) -> Option<Server> {
    assert!(
        !require(),
        "{REQUIRE_ENV}=1 and {reason}: a missing livesim2 is a failure here, not a skip"
    );
    eprintln!("SKIP e2e_livesim2: {reason} (set {REQUIRE_ENV}=1 to make this a failure)");
    None
}

fn find_binary() -> Option<PathBuf> {
    if let Ok(p) = std::env::var(BIN_ENV) {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(BIN_NAME))
        .find(|p| p.is_file())
}

fn module_cache_assets() -> Option<PathBuf> {
    let mut cmd = Command::new("go");
    cmd.args(["env", "GOMODCACHE"]);
    let out = test_bounded::output_bounded(&mut cmd, TOOL_DEADLINE).ok()?;
    let cache = String::from_utf8(out.stdout).ok()?;
    let dir = Path::new(cache.trim())
        .join(format!("{MODULE_DIR}@{PINNED_VERSION}"))
        .join(ASSETS_UNDER_MODULE);
    dir.is_dir().then_some(dir)
}

fn find_vodroot() -> Option<PathBuf> {
    if let Ok(p) = std::env::var(VODROOT_ENV) {
        let p = PathBuf::from(p);
        return p.is_dir().then_some(p);
    }
    module_cache_assets()
}

fn http() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(REQUEST_DEADLINE)
        .build()
        .expect("http client")
}

fn start_server() -> Option<Server> {
    let Some(binary) = find_binary() else {
        return bail(&format!("no livesim2 binary ({BIN_ENV} or PATH)"));
    };
    let Some(vodroot) = find_vodroot() else {
        return bail(&format!(
            "no livesim2 assets ({VODROOT_ENV} or the Go module cache)"
        ));
    };
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("a free port")
        .port();
    let repdata = std::env::temp_dir().join(format!("livesim2-repdata-{}", std::process::id()));
    let child = Command::new(binary)
        .arg("--vodroot")
        .arg(&vodroot)
        .arg("--repdataroot")
        .arg(&repdata)
        .arg("--writemissingrepdata")
        .args(["--port", &port.to_string(), "--loglevel", "ERROR"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn livesim2");
    let server = Server { child, port };
    let client = http();
    let deadline = Instant::now() + READY_DEADLINE;
    loop {
        let probe = format!("http://127.0.0.1:{port}{READY_PROBE}");
        if let Ok(resp) = client.get(&probe).send() {
            let version = resp
                .headers()
                .get(VERSION_HEADER)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            assert_eq!(
                version, PINNED_VERSION,
                "the livesim2 under test is not the pinned version"
            );
            return Some(server);
        }
        assert!(Instant::now() < deadline, "livesim2 never became ready");
        thread::sleep(Duration::from_millis(100));
    }
}

struct Shape {
    name: &'static str,
    path: &'static str,
    prefer_host: bool,
    strict_availability: bool,
    /// livesim2's `startrel_` moves `availabilityStartTime` with every request, so each MPD
    /// refresh is a Reset (design 5.2): the stream is allowed, and expected, to rejoin.
    moves_start_time: bool,
}

const SHAPES: [Shape; 6] = [
    Shape {
        name: "host-clock",
        path: "utc_head/testpic_2s/Manifest.mpd",
        prefer_host: true,
        strict_availability: true,
        moves_start_time: false,
    },
    Shape {
        name: "utc-head",
        path: "utc_head/testpic_2s/Manifest.mpd",
        prefer_host: false,
        strict_availability: false,
        moves_start_time: false,
    },
    Shape {
        name: "utc-direct",
        path: "utc_direct/testpic_2s/Manifest.mpd",
        prefer_host: false,
        strict_availability: false,
        moves_start_time: false,
    },
    Shape {
        name: "timeline-number",
        path: "utc_head/segtimelinenr_1/testpic_2s/Manifest.mpd",
        prefer_host: false,
        strict_availability: false,
        moves_start_time: false,
    },
    Shape {
        name: "start-relative",
        path: "utc_head/startrel_-20/testpic_2s/Manifest.mpd",
        prefer_host: false,
        strict_availability: false,
        moves_start_time: true,
    },
    Shape {
        name: "periods",
        path: "utc_head/periods_20/testpic_2s/Manifest.mpd",
        prefer_host: false,
        strict_availability: false,
        moves_start_time: false,
    },
];

struct Pending {
    conn: u32,
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    range: Option<(u64, u64)>,
}

struct Sent {
    url: String,
    method: String,
    sent_unix_us: i64,
    status: Option<u16>,
}

#[derive(Default)]
struct Observed {
    requests: Vec<Sent>,
    samples_by_track: BTreeMap<u32, usize>,
    errors: Vec<String>,
    discontinuities: usize,
    tracks_announced: usize,
}

fn unix_us() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_micros(),
    )
    .expect("unix time fits")
}

fn config_for(shape: &Shape) -> Config {
    let mut config = Config::default();
    if shape.prefer_host {
        config.dash.clock_policy = DashClockPolicy::PreferHost;
        config.dash.host_clock_uncertainty_us = HOST_CLOCK_UNCERTAINTY_US;
    }
    config
}

fn perform(
    client: &reqwest::blocking::Client,
    session: &mut Session,
    start: Instant,
    action: Pending,
    observed: &mut Observed,
) {
    let Pending {
        conn,
        method,
        url,
        headers,
        range,
    } = action;
    let mono = || u64::try_from(start.elapsed().as_micros()).expect("mono fits");
    let mut request = client.request(method.parse().expect("method"), &url);
    for (name, value) in &headers {
        request = request.header(name, value);
    }
    if let Some((offset, length)) = range {
        request = request.header(
            "Range",
            format!("bytes={offset}-{}", offset + length.saturating_sub(1)),
        );
    }
    let sent_unix_us = unix_us();
    let response = request.send();
    let mut record = Sent {
        url,
        method,
        sent_unix_us,
        status: None,
    };
    match response {
        Ok(mut resp) => {
            record.status = Some(resp.status().as_u16());
            let headers: Vec<(String, String)> = resp
                .headers()
                .iter()
                .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
                .collect();
            session.on_response(conn, resp.status().as_u16(), &headers, mono());
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                match resp.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => session.on_data(conn, &buf[..n], mono()),
                    Err(_) => {
                        session.on_closed(conn, Some("read".to_owned()), mono());
                        observed.requests.push(record);
                        return;
                    }
                }
            }
            session.on_closed(conn, None, mono());
        }
        Err(_) => session.on_closed(conn, Some("connect".to_owned()), mono()),
    }
    observed.requests.push(record);
}

fn drain(session: &mut Session, observed: &mut Observed) {
    while let Some(event) = session.next_event() {
        match event {
            Event::Sample { track, .. } => {
                *observed.samples_by_track.entry(track.value()).or_default() += 1;
            }
            Event::Tracks(tracks) => observed.tracks_announced += tracks.len(),
            Event::Discontinuity => observed.discontinuities += 1,
            Event::Error { kind, fatal } => observed.errors.push(format!("{kind:?} fatal={fatal}")),
            _ => {}
        }
    }
}

fn play(port: u16, shape: &Shape) -> Observed {
    let url = format!("http://127.0.0.1:{port}/livesim2/{}", shape.path);
    let start = Instant::now();
    let mono = || u64::try_from(start.elapsed().as_micros()).expect("mono fits");
    let mut session = Session::open(&url, config_for(shape), mono()).expect("open");
    session.set_wall_clock(mono(), u64::try_from(unix_us()).expect("unix"));
    let client = http();
    let mut observed = Observed::default();
    while start.elapsed() < RUN {
        let now = mono();
        let mut pending = Vec::new();
        let mut wake: Option<u64> = None;
        while let Some(action) = session.poll(now) {
            match action {
                Action::HttpRequest {
                    conn,
                    method,
                    url,
                    headers,
                    byte_range,
                } => pending.push(Pending {
                    conn,
                    method,
                    url,
                    headers,
                    range: byte_range,
                }),
                Action::WakeAt(t) => wake = Some(wake.map_or(t, |w| w.min(t))),
                _ => {}
            }
        }
        for action in pending {
            perform(&client, &mut session, start, action, &mut observed);
        }
        drain(&mut session, &mut observed);
        let sleep = wake
            .map(|t| Duration::from_micros(t.saturating_sub(mono())))
            .unwrap_or(IDLE_SLEEP)
            .min(IDLE_SLEEP);
        thread::sleep(sleep);
    }
    session.close();
    drain(&mut session, &mut observed);
    observed
}

fn segment_key(url: &str) -> Option<(String, u64)> {
    let path = url.split('?').next()?;
    let mut parts = path.rsplit('/');
    let file = parts.next()?;
    let rep = parts.next()?.to_owned();
    let number = file.strip_suffix(".m4s")?.parse().ok()?;
    Some((rep, number))
}

fn verify(shape: &Shape, observed: &Observed) {
    let name = shape.name;
    assert!(
        observed.errors.is_empty(),
        "{name}: errors {:?}",
        observed.errors
    );
    assert!(
        observed.tracks_announced >= 2,
        "{name}: tracks announced {}",
        observed.tracks_announced
    );
    assert!(
        observed.samples_by_track.len() >= 2 && observed.samples_by_track.values().all(|n| *n > 0),
        "{name}: samples per track {:?}",
        observed.samples_by_track
    );
    if shape.moves_start_time {
        assert!(
            observed.discontinuities > 0,
            "{name}: a moving availabilityStartTime was not announced as a discontinuity"
        );
    } else {
        assert_eq!(
            observed.discontinuities, 0,
            "{name}: the stream was not continuous"
        );
    }

    let mut delivered: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    let mut early = 0usize;
    for r in &observed.requests {
        let Some(status) = r.status else { continue };
        if let Some((rep, n)) = segment_key(&r.url) {
            match status {
                200 | 206 => delivered.entry(rep).or_default().push(n),
                STATUS_TOO_EARLY | STATUS_NOT_FOUND if !shape.strict_availability => early += 1,
                STATUS_TOO_EARLY | STATUS_NOT_FOUND | STATUS_GONE => {
                    panic!("{name}: segment {} answered {status}", r.url)
                }
                other => panic!("{name}: segment {} answered {other}", r.url),
            }
        }
    }
    assert!(
        delivered.len() >= 2,
        "{name}: reps fetched {:?}",
        delivered.keys()
    );
    for (rep, numbers) in &mut delivered {
        let total = numbers.len();
        assert!(
            total >= MIN_SEGMENTS_PER_TRACK,
            "{name}: {rep} delivered only {total} segments"
        );
        if shape.moves_start_time {
            continue;
        }
        let in_order = numbers.windows(2).all(|w| w[1] > w[0]);
        assert!(
            in_order,
            "{name}: {rep} segments out of order or repeated: {numbers:?}"
        );
        let contiguous = numbers.windows(2).all(|w| w[1] == w[0] + 1);
        assert!(contiguous, "{name}: {rep} has a gap: {numbers:?}");
    }

    let mpd_requests = observed
        .requests
        .iter()
        .filter(|r| r.url.contains(".mpd") && r.method == "GET")
        .count();
    assert!(
        mpd_requests >= 3,
        "{name}: only {mpd_requests} MPD requests (two refreshes expected)"
    );

    let mut per_second: BTreeMap<(String, i64), usize> = BTreeMap::new();
    for r in &observed.requests {
        *per_second
            .entry((r.url.clone(), r.sent_unix_us / MICROS))
            .or_default() += 1;
    }
    let worst = per_second.values().max().copied().unwrap_or(0);
    assert!(
        worst <= MAX_ATTEMPTS_PER_SECOND,
        "{name}: {worst} requests to one URL inside a second"
    );

    if shape.strict_availability {
        assert_eq!(
            early, 0,
            "{name}: segments were requested before they existed"
        );
        let started = observed
            .requests
            .iter()
            .filter(|r| r.url.contains("/V300/") && r.url.ends_with(".m4s"))
            .filter_map(|r| segment_key(&r.url).map(|(_, n)| (n, r.sent_unix_us)));
        for (n, sent) in started {
            let available = (i64::try_from(n).expect("number") + 1) * SEGMENT_SECONDS * MICROS;
            assert!(
                sent >= available - STRICT_EARLY_TOLERANCE_US,
                "{name}: segment {n} requested {} ms before it was available",
                (available - sent) / 1_000
            );
        }
    }
}

#[test]
fn thirty_seconds_of_live_dash_against_livesim2() {
    let Some(server) = start_server() else {
        return;
    };
    let port = server.port;
    let runs: Vec<_> = SHAPES
        .iter()
        .map(|shape| thread::spawn(move || (shape, play(port, shape))))
        .collect();
    let mut failures = Vec::new();
    for run in runs {
        let (shape, observed) = run.join().expect("play thread");
        println!(
            "{}: {} requests, samples {:?}",
            shape.name,
            observed.requests.len(),
            observed.samples_by_track
        );
        if let Err(panic) = std::panic::catch_unwind(|| verify(shape, &observed)) {
            let text = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_default();
            failures.push(text);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_missing_server_skips_without_the_require_flag() {
    if require() {
        return;
    }
    let found = find_binary().is_some() && find_vodroot().is_some();
    if found {
        return;
    }
    assert!(start_server().is_none());
}
