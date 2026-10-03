//! `media-doctor` CLI binary.

use std::fs;
use std::net::{SocketAddr, UdpSocket};
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;
use media_doctor::cli::{CheckArgs, CheckDashArgs, CheckHlsArgs, Cli, WatchArgs};
use media_doctor::metrics_server::{self, MetricsPublisher, MetricsServerConfig};
use media_doctor::{
    CcAnomalyCheck, CodecSignallingCheck, Diagnostic, FpsCadenceCheck, InterlaceCheck,
    ParamSetsCheck, PatPmtVersionCheck, PcrCheck, PtsCheck, Report, Scte35Check, SyncByteCheck,
    WatchState, check_container_codec, check_dash_mpd, check_hls_playlist, run_all,
};
use tokio_util::sync::CancellationToken;

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

/// `media-doctor watch` — the socket glue around [`media_doctor::WatchState`]
/// (issue #665). Ingest and accounting live in the library; HTTP framing is
/// hyper's (`metrics_server`), the Prometheus text is the exporter's
/// (`watch_metrics`), the UDP bind is socket2's (`udp`). This function owns
/// only the wiring: a blocking ingest loop on the main thread and a
/// current-thread tokio runtime on a `metrics` thread serving the latest body.
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
    let udp_config = args.udp_config()?;

    let socket = media_doctor::udp::bind_udp(udp_addr, &udp_config)?;
    if let (Some(want), Ok(got)) = (
        udp_config.recv_buffer,
        media_doctor::udp::recv_buffer_size(&socket),
    ) && got < want
    {
        eprintln!(
            "media-doctor watch: warning: SO_RCVBUF requested {want} bytes, kernel granted {got}"
        );
    }
    socket.set_read_timeout(Some(PUBLISH_INTERVAL))?;
    let listener = std::net::TcpListener::bind(metrics_addr)?;
    listener.set_nonblocking(true)?;
    eprintln!(
        "media-doctor watch: ingesting UDP {}, metrics on http://{}/metrics",
        socket.local_addr()?,
        listener.local_addr()?
    );

    let mut state = WatchState::new();
    let (mut publisher, receiver) = metrics_server::channel(&state, PUBLISH_INTERVAL);
    let shutdown = CancellationToken::new();
    let config = MetricsServerConfig::new(
        args.metrics_max_conns,
        Duration::from_millis(args.metrics_io_timeout_ms),
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let server_token = shutdown.clone();
    let server = thread::Builder::new()
        .name("metrics".into())
        .spawn(move || {
            runtime.block_on(async move {
                match tokio::net::TcpListener::from_std(listener) {
                    Ok(listener) => {
                        metrics_server::serve(listener, receiver, config, server_token).await;
                    }
                    Err(e) => eprintln!("media-doctor watch: metrics listener unavailable: {e}"),
                }
            });
        })?;

    let result = ingest_loop(&socket, &mut state, &mut publisher);
    shutdown.cancel();
    let _ = server.join();
    result
}

/// How often the exposition is re-rendered, and the socket read timeout that
/// guarantees a flush after the feed goes quiet.
const PUBLISH_INTERVAL: Duration = Duration::from_millis(250);

fn ingest_loop(
    socket: &UdpSocket,
    state: &mut WatchState,
    publisher: &mut MetricsPublisher,
) -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    let mut buf = [0u8; 65536];
    let mut dirty = false;
    loop {
        match socket.recv_from(&mut buf) {
            Ok((n, _src)) => {
                let clock = start.elapsed();
                state.feed_datagram(&buf[..n], clock);
                dirty = !publisher.maybe_publish(state, clock);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if dirty {
                    publisher.publish_now(state, start.elapsed());
                    dirty = false;
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
}
