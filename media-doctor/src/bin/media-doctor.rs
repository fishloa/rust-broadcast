//! `media-doctor` CLI binary.

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;
use media_doctor::cli::{CheckArgs, CheckDashArgs, CheckHlsArgs, Cli, WatchArgs};
use media_doctor::{
    CcAnomalyCheck, CodecSignallingCheck, Diagnostic, FpsCadenceCheck, InterlaceCheck,
    ParamSetsCheck, PatPmtVersionCheck, PcrCheck, PtsCheck, Report, Scte35Check, SyncByteCheck,
    WatchState, check_container_codec, check_dash_mpd, check_hls_playlist, run_all,
};

fn main() {
    let cli = Cli::parse();
    match cli {
        Cli::Check(args) => {
            if let Err(e) = run_check(&args) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        Cli::CheckHls(args) => {
            if let Err(e) = run_check_hls(&args) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        Cli::CheckDash(args) => {
            if let Err(e) = run_check_dash(&args) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        Cli::Watch(args) => {
            if let Err(e) = run_watch(&args) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        // `Cli` is `#[non_exhaustive]`, but this binary is the only caller
        // of `Cli::parse()` and clap can only ever produce `Check`/`Watch`.
        _ => unreachable!("clap only ever produces a known Cli subcommand"),
    }
}

/// The 188-byte MPEG-2 TS packet length (ISO/IEC 13818-1 §2.4.3.2), from the
/// workspace's owner of the constant rather than a local literal.
const TS_PACKET_SIZE: usize = mpeg_ts::ts::TS_PACKET_SIZE;

/// Which diagnostic set the CLI runs for `bytes`.
///
/// Detection is delegated to `container-probe`, which scores every format it
/// knows (a stride×phase lattice search across 188/192/204/208-byte TS
/// framings, so a capture that does not start on a packet boundary, an M2TS
/// file and a 204-byte RS-framed stream all identify as TS — none of which
/// the previous "`0x47` at byte 0 and byte 188" check could see). The old
/// check routed all of those to the container path, which bailed at its
/// ISOBMFF sniff and printed "No issues found." for a TS full of errors
/// (audit MD-W7).
enum InputKind {
    /// MPEG-2 TS in any of the framings `container-probe` recognises, already
    /// resynchronised to 188-byte packets.
    TransportStream(Vec<u8>),
    /// A container `container_probe` named, or an input it could not classify
    /// — in both cases the container diagnostics get their say.
    Container,
}

/// Classify `bytes` and, for a TS, return the extracted 188-byte packet
/// stream.
///
/// `container-probe` reports the lattice it matched: the packet `stride`
/// (188 plain TS, 192 M2TS, 204 RS-framed, 208 M2TS-over-RS) and the byte
/// offset of the first sync byte. Extraction takes each stride-aligned
/// record's leading `stride - 188` *wrapper* bytes off and keeps the 188-byte
/// packet, which covers a leading-junk capture, a mid-packet cut and every
/// wrapped framing without a second detection pass.
///
/// Anything that is not *decidedly* a TS goes to the container path: the TS
/// diagnostics read the input as a raw packet lattice and produce meaningless
/// noise on a non-TS, so an undecided input must not take that path.
fn classify_input(bytes: &[u8]) -> InputKind {
    let (stride, phase) = match container_probe::probe(bytes) {
        container_probe::Probe::Identified {
            format: container_probe::Format::MpegTs,
            detail: container_probe::Detail::Ts { stride, phase, .. },
            ..
        } => (usize::from(stride), usize::from(phase)),
        // A tie that includes TS is still worth the TS diagnostics — they are
        // the ones that would otherwise have been skipped. `Detail` only ever
        // carries the lattice for a TS candidate.
        container_probe::Probe::Ambiguous { candidates, .. } => {
            let Some((stride, phase)) = candidates.iter().find_map(|c| match c.detail {
                container_probe::Detail::Ts { stride, phase, .. } => {
                    Some((usize::from(stride), usize::from(phase)))
                }
                _ => None,
            }) else {
                return InputKind::Container;
            };
            (stride, phase)
        }
        // `Probe` is `#[non_exhaustive]`: any future outcome that is not a
        // decided TS goes to the container path.
        _ => return InputKind::Container,
    };

    // `phase` is the offset of the first **sync byte** (container-probe's
    // lattice search walks `phase + n*stride` and tests for `0x47` at each
    // position), so it already sits past any 4-byte M2TS `TP_extra_header`.
    // The 188 bytes starting there are the packet itself, whatever the
    // wrapper around it is — so extract exactly 188 from `off`, not
    // `off + (stride - 188)`.
    if !(TS_PACKET_SIZE..=MAX_WRAPPED_STRIDE).contains(&stride) {
        return InputKind::Container;
    }

    let mut out = Vec::new();
    let mut off = phase;
    while off
        .checked_add(TS_PACKET_SIZE)
        .is_some_and(|end| end <= bytes.len())
    {
        out.extend_from_slice(&bytes[off..off + TS_PACKET_SIZE]);
        match off.checked_add(stride) {
            Some(next) => off = next,
            None => break,
        }
    }
    if out.is_empty() {
        return InputKind::Container;
    }
    InputKind::TransportStream(out)
}

/// The longest wrapped TS record `container-probe` recognises: 188-byte
/// packet + 16-byte RS parity + 4-byte M2TS `TP_extra_header`
/// (ETSI EN 300 421 §5.2.2 / BDAV).
const MAX_WRAPPED_STRIDE: usize = 208;

fn run_check(args: &CheckArgs) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = fs::read(&args.input)?;
    let mut report = Report::new();

    match classify_input(&bytes) {
        InputKind::TransportStream(realigned) => run_ts_checks(&realigned, &mut report),
        // Borrows `bytes` directly — the container path needs no realigned
        // copy, so this never clones the input.
        InputKind::Container => check_container_codec(&bytes, &mut report),
    }

    emit_check_report(&report, args.json)
}

/// Run every transport-stream diagnostic over already-aligned 188-byte
/// packets.
fn run_ts_checks(ts: &[u8], report: &mut Report) {
    let diagnostics: &[&dyn Diagnostic] = &[
        &SyncByteCheck,
        &PatPmtVersionCheck,
        &CcAnomalyCheck,
        &PcrCheck,
        &PtsCheck,
        &Scte35Check,
        &CodecSignallingCheck,
        &FpsCadenceCheck,
        &ParamSetsCheck,
        &InterlaceCheck,
    ];
    run_all(ts, diagnostics, report);
}

fn emit_check_report(report: &Report, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    if json {
        #[cfg(feature = "serde")]
        {
            let json = serde_json::to_string_pretty(report)?;
            println!("{json}");
        }
        #[cfg(not(feature = "serde"))]
        {
            // Should not happen: cli feature implies serde, but be safe.
            eprintln!("JSON output requires the `serde` feature.");
            process::exit(1);
        }
    } else {
        println!("{report}");
    }
    Ok(())
}

fn run_check_hls(args: &CheckHlsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let text = fs::read_to_string(&args.input)?;
    let mut report = Report::new();
    check_hls_playlist(&text, &mut report);
    emit_report(&report, args.json)
}

fn run_check_dash(args: &CheckDashArgs) -> Result<(), Box<dyn std::error::Error>> {
    let text = fs::read_to_string(&args.input)?;
    let mut report = Report::new();
    check_dash_mpd(&text, &mut report);
    emit_report(&report, args.json)
}

fn emit_report(report: &Report, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    if json {
        #[cfg(feature = "serde")]
        {
            let json = serde_json::to_string_pretty(report)?;
            println!("{json}");
        }
        #[cfg(not(feature = "serde"))]
        {
            eprintln!("JSON output requires the `serde` feature.");
            process::exit(1);
        }
    } else {
        println!("{report}");
    }
    Ok(())
}

/// `media-doctor watch` — the socket/thread glue around
/// [`media_doctor::WatchState`] (issue #665). All the actual ingest/metrics
/// logic lives in the library (`media_doctor::watch`, tested without any
/// socket); this function only opens a `UdpSocket` (joining the multicast
/// group when the address calls for it) and a `TcpListener` for the metrics
/// endpoint, and wires them to a shared `WatchState` from two threads.
fn run_watch(args: &WatchArgs) -> Result<(), Box<dyn std::error::Error>> {
    let udp_addr: SocketAddr = args
        .udp
        .parse()
        .map_err(|e| format!("invalid --udp address {:?}: {e}", args.udp))?;
    let metrics_addr: SocketAddr = args.metrics_addr.parse().map_err(|e| {
        format!(
            "invalid --metrics-addr address {:?}: {e}",
            args.metrics_addr
        )
    })?;

    let socket = bind_udp(udp_addr)?;
    let listener = TcpListener::bind(metrics_addr)?;
    eprintln!(
        "media-doctor watch: ingesting UDP {udp_addr}, metrics on http://{metrics_addr}/metrics"
    );

    let state = Arc::new(Mutex::new(WatchState::new()));

    // The metrics HTTP responder runs on its own (detached) thread; the
    // ingest loop below runs on the main thread. Both share `state` under a
    // mutex — this program only ever needs "two things happening at once",
    // not real high-concurrency, so a couple of `std::thread`s are enough
    // (no async runtime).
    let http_state = Arc::clone(&state);
    let metrics_config = MetricsConfig {
        max_conns: args.metrics_max_conns,
        io_timeout: core::time::Duration::from_millis(args.metrics_io_timeout_ms),
    };
    // `thread::spawn` panics if the OS refuses a thread, which would take the
    // process down at startup. The metrics responder is auxiliary, so a
    // failure there is reported and the ingest loop carries on.
    if let Err(e) = thread::Builder::new()
        .name("metrics".into())
        .spawn(move || serve_metrics(listener, &http_state, metrics_config))
    {
        eprintln!("media-doctor watch: metrics responder unavailable: {e}");
    }

    let start = Instant::now();
    let mut buf = [0u8; 65536];
    loop {
        let (n, _src) = socket.recv_from(&mut buf)?;
        let clock = start.elapsed();
        // A 24/7 probe must not crash on a poisoned lock (some unrelated
        // panic elsewhere while the mutex was held) -- recover the data
        // rather than unwrap/expect, since the mutex-protected state itself
        // is still valid even after a poisoning panic.
        let mut guard = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.feed_datagram(&buf[..n], clock);
    }
}

/// Bind a UDP socket for `addr`, joining the IPv4 multicast group when `addr`
/// falls in the multicast range (224.0.0.0-239.255.255.255).
fn bind_udp(addr: SocketAddr) -> std::io::Result<UdpSocket> {
    match addr {
        SocketAddr::V4(v4) if v4.ip().is_multicast() => {
            // Multicast: bind to the port on all interfaces, then join the
            // group, rather than binding the group address itself.
            let bind_addr = SocketAddr::new(
                std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
                v4.port(),
            );
            let socket = UdpSocket::bind(bind_addr)?;
            socket.join_multicast_v4(v4.ip(), &std::net::Ipv4Addr::UNSPECIFIED)?;
            Ok(socket)
        }
        _ => UdpSocket::bind(addr),
    }
}

/// Knobs for the metrics responder.
#[derive(Debug, Clone, Copy)]
struct MetricsConfig {
    /// Maximum connections served concurrently.
    max_conns: usize,
    /// Total deadline per connection, accept to response written.
    io_timeout: core::time::Duration,
}

/// Serve `GET /metrics` (and anything else — this is a single-endpoint
/// probe) as Prometheus text exposition format.
///
/// Each connection is handled on its own thread, bounded by three things
/// (audit MD-W9):
///
/// - a **concurrency cap** ([`MetricsConfig::max_conns`]): past it a
///   connection is answered `503` and closed without spawning, so a flood of
///   idle clients cannot spawn unbounded threads;
/// - [`thread::Builder::spawn`], whose `Err` drops the connection instead of
///   panicking the accept loop;
/// - a **total deadline** ([`MetricsConfig::io_timeout`]) enforced across the
///   whole exchange, not per read, so a slowloris dribbling one byte at a
///   time still gets dropped.
fn serve_metrics(listener: TcpListener, state: &Arc<Mutex<WatchState>>, config: MetricsConfig) {
    let live = Arc::new(AtomicUsize::new(0));
    for incoming in listener.incoming() {
        let stream = match incoming {
            Ok(stream) => stream,
            Err(e) => {
                // `incoming()` is an infinite iterator of `Result`: a per-
                // connection failure (EMFILE/ENFILE under a connection
                // storm, ECONNABORTED from a peer that vanished) must not
                // spin the accept loop at 100% CPU. Back off briefly, then
                // keep serving — the alternative is a hot loop that starves
                // the ingest thread too.
                eprintln!("media-doctor watch: metrics accept error: {e}");
                thread::sleep(ACCEPT_ERROR_BACKOFF);
                continue;
            }
        };

        // Reserve a slot before spawning. A connection over the cap is
        // refused here rather than queued, so the accept loop keeps draining
        // the backlog for well-behaved clients.
        if !reserve_connection(&live, config.max_conns) {
            if let Err(e) = refuse_connection(stream, config.io_timeout) {
                eprintln!("media-doctor watch: metrics refusal error: {e}");
            }
            continue;
        }

        let conn_state = Arc::clone(state);
        let conn_live = Arc::clone(&live);
        let spawned = thread::Builder::new()
            .name("metrics-conn".into())
            .spawn(move || {
                let _guard = ConnectionGuard(&conn_live);
                if let Err(e) = handle_metrics_request(stream, &conn_state, config.io_timeout) {
                    eprintln!("media-doctor watch: metrics request error: {e}");
                }
            });
        if spawned.is_err() {
            // The thread never started, so its guard never runs: release the
            // slot here. The connection is simply dropped.
            live.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

/// How long to pause after an `accept()` failure before trying again.
///
/// Long enough that an fd-exhausted server cannot spin, short enough that a
/// transient failure (a peer that aborted between `connect` and `accept`)
/// costs nothing noticeable.
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(50);

/// Answer a connection refused by the concurrency cap, and close it cleanly.
///
/// The write is bounded by a **total** deadline and the write half is shut
/// down afterwards, so a peer that never reads cannot hold the accept loop
/// on this response, and a peer that has already gone cannot turn the
/// refusal into an error that masks the reason.
fn refuse_connection(mut stream: TcpStream, io_timeout: Duration) -> std::io::Result<()> {
    stream.set_write_timeout(Some(io_timeout))?;
    let result = stream.write_all(SERVICE_UNAVAILABLE_RESPONSE.as_bytes());
    // Half-close so the client sees EOF immediately after the response; the
    // read half is dropped with `stream` when this returns.
    let _ = stream.shutdown(std::net::Shutdown::Write);
    result
}

/// Smallest socket timeout we will set. `Duration::ZERO` means "block
/// forever" on some platforms, so a deadline that has already passed must
/// still be expressed as a non-zero timeout.
const MIN_SOCKET_TIMEOUT: Duration = Duration::from_millis(1);

/// Bytes that terminate an HTTP/1.1 request head.
const HEADER_TERMINATOR: &[u8] = b"\r\n\r\n";

/// Upper bound on a metrics request head, so a peer that never sends the
/// terminator cannot make the server buffer without bound before the total
/// deadline fires.
const MAX_REQUEST_BYTES: usize = 16 * 1024;

/// `503` body sent when the concurrency cap is reached.
const SERVICE_UNAVAILABLE_RESPONSE: &str = concat!(
    "HTTP/1.1 503 Service Unavailable\r\n",
    "Content-Type: text/plain\r\n",
    "Content-Length: 0\r\n",
    "Connection: close\r\n",
    "\r\n",
);

/// Take one slot under `cap`, returning `false` when none is free.
fn reserve_connection(live: &AtomicUsize, cap: usize) -> bool {
    // A `fetch_update` rather than load-then-add: the check and the increment
    // must be one atomic step, or two connections racing at the cap both
    // slip through.
    live.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
        (n < cap).then_some(n + 1)
    })
    .is_ok()
}

/// Releases a connection slot on drop, including on panic.
struct ConnectionGuard<'a>(&'a AtomicUsize);

impl Drop for ConnectionGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn handle_metrics_request(
    mut stream: TcpStream,
    state: &Arc<Mutex<WatchState>>,
    io_timeout: core::time::Duration,
) -> std::io::Result<()> {
    // A **total** deadline, enforced by shrinking the socket timeout to the
    // time still remaining before each blocking call. A per-call timeout
    // alone lets a peer that dribbles one byte per second hold a connection
    // (and its thread) open indefinitely (slowloris).
    // `checked_add` rather than `+`: `--metrics-io-timeout-ms` is
    // caller-supplied and an absurd value would otherwise panic the
    // connection thread. A deadline that cannot be represented means "no
    // deadline", which the `saturating_duration_since` below already turns
    // into the full budget.
    let deadline = Instant::now().checked_add(io_timeout);
    let remaining = || {
        let budget = match deadline {
            Some(deadline) => deadline.saturating_duration_since(Instant::now()),
            None => io_timeout,
        };
        // A zero timeout means "block forever" on some platforms, so floor it.
        Some(budget.max(MIN_SOCKET_TIMEOUT))
    };
    // Read until the end of the request headers. We only serve one endpoint,
    // so the request is not parsed: any complete HTTP/1.1 head gets the same
    // response. Waiting for the terminator (rather than responding after the
    // first `read`) is what makes a slowloris visibly wrong — it dribbles a
    // request that never ends, and the total deadline below is what stops it.
    stream.set_read_timeout(remaining())?;
    let mut request: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        // Re-arm against what is left of the **total** deadline on every
        // iteration: a fixed per-read timeout would be reset by every byte a
        // slowloris sends, which is exactly the attack this defends against.
        stream.set_read_timeout(remaining())?;
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                request.extend_from_slice(&chunk[..n]);
                if request
                    .windows(HEADER_TERMINATOR.len())
                    .any(|w| w == HEADER_TERMINATOR)
                {
                    break;
                }
                // A peer sending more than a request head is not a metrics
                // scraper; stop reading rather than buffer without bound.
                if request.len() > MAX_REQUEST_BYTES {
                    break;
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                // The total deadline expired with the request still
                // incomplete: drop the connection.
                return Ok(());
            }
            Err(e) => return Err(e),
        }
    }

    let body = {
        // See the ingest loop's matching comment: recover from a poisoned
        // lock instead of crashing a 24/7 probe.
        let guard = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.render_prometheus()
    };

    let response = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/plain; version=0.0.4\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len(),
    );
    // Re-arm the write timeout against what is left of the total deadline.
    stream.set_write_timeout(remaining())?;
    stream.write_all(response.as_bytes())
}
