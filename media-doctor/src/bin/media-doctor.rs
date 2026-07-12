//! `media-doctor` CLI binary.

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::process;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use clap::Parser;
use media_doctor::cli::{CheckArgs, Cli, WatchArgs};
use media_doctor::{
    CcAnomalyCheck, CodecSignallingCheck, Diagnostic, FpsCadenceCheck, InterlaceCheck,
    ParamSetsCheck, PatPmtVersionCheck, PcrCheck, PtsCheck, Report, Scte35Check, SyncByteCheck,
    WatchState, check_container_codec, run_all,
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
        Cli::Watch(args) => {
            if let Err(e) = run_watch(&args) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
    }
}

/// A minimal MPEG-2 TS sniff: sync byte `0x47` at both packet 0 and packet 1
/// (ISO/IEC 13818-1 §2.4.3.2). Used only to pick which diagnostic set the
/// CLI runs — an ISOBMFF/CMAF file's first bytes essentially never match
/// this by chance, and running the TS-only diagnostics against MP4 bytes
/// (or vice versa) produces meaningless noise (e.g. a `sync-byte` error per
/// "packet") rather than a crash, so this is a UX choice, not a correctness
/// requirement.
fn looks_like_ts(bytes: &[u8]) -> bool {
    const TS_PACKET_SIZE: usize = 188;
    bytes.len() >= 2 * TS_PACKET_SIZE && bytes[0] == 0x47 && bytes[TS_PACKET_SIZE] == 0x47
}

fn run_check(args: &CheckArgs) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = fs::read(&args.input)?;
    let mut report = Report::new();

    if looks_like_ts(&bytes) {
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
        run_all(&bytes, diagnostics, &mut report);
    } else {
        check_container_codec(&bytes, &mut report);
    }

    if args.json {
        #[cfg(feature = "serde")]
        {
            let json = serde_json::to_string_pretty(&report)?;
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
    thread::spawn(move || serve_metrics(listener, &http_state));

    let start = Instant::now();
    let mut buf = [0u8; 65536];
    loop {
        let (n, _src) = socket.recv_from(&mut buf)?;
        let clock = start.elapsed();
        let mut guard = state.lock().expect("watch state mutex poisoned");
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

/// Serve `GET /metrics` (and anything else — this is a single-endpoint
/// probe) as Prometheus text exposition format, one connection at a time.
/// No HTTP crate: real broadcast/monitoring tooling only ever sends a
/// single-line `GET /metrics HTTP/1.1` request here, so a hand-rolled
/// accept loop is simpler and keeps this dependency-light, matching the
/// rest of the workspace's CLIs.
fn serve_metrics(listener: TcpListener, state: &Arc<Mutex<WatchState>>) {
    for incoming in listener.incoming() {
        let Ok(stream) = incoming else { continue };
        if let Err(e) = handle_metrics_request(stream, state) {
            eprintln!("media-doctor watch: metrics request error: {e}");
        }
    }
}

fn handle_metrics_request(
    mut stream: TcpStream,
    state: &Arc<Mutex<WatchState>>,
) -> std::io::Result<()> {
    // We only serve one endpoint, so the request doesn't need to be parsed
    // beyond draining it: any well-formed HTTP/1.1 request gets the same
    // response. A generously-sized single read covers the request line +
    // headers most clients send for a bodyless GET.
    let mut request = [0u8; 4096];
    let _ = stream.read(&mut request)?;

    let body = {
        let guard = state.lock().expect("watch state mutex poisoned");
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
    stream.write_all(response.as_bytes())
}
