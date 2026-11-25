//! `media-doctor` CLI binary.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::net::{SocketAddr, UdpSocket};
use std::path::Path;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use broadcast_common::bounded::{Limit, read_file_bounded_string};
use clap::Parser;
use media_doctor::cli::{CheckArgs, CheckDashArgs, CheckHlsArgs, Cli, WatchArgs};
use media_doctor::metrics_server::{self, MetricsPublisher, MetricsServerConfig};
use media_doctor::{
    CcAnomalyCheck, CodecSignallingCheck, Diagnostic, DiagnosticRunner, FpsCadenceCheck,
    InterlaceCheck, ParamSetsCheck, PatPmtVersionCheck, PcrCheck, PmtStreamCollector, PtsCheck,
    Report, Scte35Check, SyncByteCheck, WatchState, check_dash_mpd, check_hls_playlist,
    check_mp4_file, looks_like_isobmff,
};
use tokio_util::sync::CancellationToken;

/// A whole HLS playlist: the ceiling `broadcast-hls` itself accepts.
const PLAYLIST_LIMIT: Limit = Limit::new(
    "HLS playlist",
    broadcast_hls::limits::MAX_PLAYLIST_BYTES as u64,
);
/// A whole MPD: the ceiling `broadcast-dash` itself accepts.
const MPD_LIMIT: Limit = Limit::new("MPD", broadcast_dash::limits::MAX_MPD_BYTES as u64);

fn main() {
    let cli = Cli::parse();
    match cli {
        Cli::Check(args) => {
            let registry = match media_doctor::payload::registry(&args.plugin) {
                Ok(registry) => registry,
                Err(message) => {
                    clap::Error::raw(clap::error::ErrorKind::InvalidValue, message).exit()
                }
            };
            if let Err(e) = run_check(&args, &registry) {
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
/// Size of one read from the input stream.
const STREAM_CHUNK_LEN: usize = 65_536;

/// How a bounded leading prefix routes the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Routed {
    /// Stream the TS diagnostics over the file.
    Ts,
    /// Stream the container checks over the file.
    Mp4,
    /// Not an MP4: the container checks have nothing to say.
    Other,
}

/// Largest prefix read to route an input: 4 MiB.
const PROBE_PREFIX_MAX: usize = 4 << 20;

/// Longest prefix `container_probe::probe` examines, so a prefix this long
/// reproduces a whole-file probe without reading the whole file.
const PROBE_READ_LEN: usize = container_probe::DEFAULT_BUDGET;

/// The longest wrapped TS record `container-probe` recognises: 188-byte
/// packet + 16-byte RS parity + 4-byte M2TS `TP_extra_header`
/// (ETSI EN 300 421 §5.2.2 / BDAV).
const MAX_WRAPPED_STRIDE: usize = 208;

/// The TS lattice `prefix` decides: the packet `stride` (188 plain TS, 192
/// M2TS, 204 RS-framed, 208 M2TS-over-RS) and the byte offset of the first
/// sync byte.
///
/// `container-probe` reports the lattice it matched, and `phase` is the offset
/// of the first **sync byte** (its lattice search walks `phase + n*stride` and
/// tests for `0x47` at each position), so it already sits past any 4-byte M2TS
/// `TP_extra_header`. The 188 bytes starting there are the packet itself,
/// whatever the wrapper around it is.
///
/// Anything that is not *decidedly* a TS returns `None`: the TS diagnostics
/// read the input as a raw packet lattice and produce meaningless noise on a
/// non-TS, so an undecided input must not take that path.
fn ts_lattice(prefix: &[u8]) -> Option<(usize, usize)> {
    let (stride, phase) = match container_probe::probe(prefix) {
        container_probe::Probe::Identified {
            format: container_probe::Format::MpegTs,
            detail: container_probe::Detail::Ts { stride, phase, .. },
            ..
        } => (usize::from(stride), usize::from(phase)),
        // A tie that includes TS is still worth the TS diagnostics — they are
        // the ones that would otherwise have been skipped. `Detail` only ever
        // carries the lattice for a TS candidate.
        container_probe::Probe::Ambiguous { candidates, .. } => {
            candidates.iter().find_map(|c| match c.detail {
                container_probe::Detail::Ts { stride, phase, .. } => {
                    Some((usize::from(stride), usize::from(phase)))
                }
                _ => None,
            })?
        }
        // `Probe` is `#[non_exhaustive]`: any future outcome that is not a
        // decided TS goes to the container path.
        _ => return None,
    };
    (TS_PACKET_SIZE..=MAX_WRAPPED_STRIDE)
        .contains(&stride)
        .then_some((stride, phase))
}

/// Read the leading prefix used to decide the TS lattice.
fn read_probe_prefix(file: &mut File, len: u64) -> io::Result<Vec<u8>> {
    let want = usize::try_from(len)
        .unwrap_or(usize::MAX)
        .min(PROBE_READ_LEN);
    file.seek(SeekFrom::Start(0))?;
    let mut prefix = vec![0u8; want];
    let read = read_full(file, &mut prefix)?;
    prefix.truncate(read);
    Ok(prefix)
}

/// Feed every 188-byte packet of the capture at `path` to `on_packet`.
///
/// Records are `stride` bytes apart starting at `phase`; only the leading 188
/// bytes of each are the packet (the rest is the wrapper the framing adds).
/// Reads through a [`std::io::BufReader`] of [`STREAM_CHUNK_LEN`], so a
/// multi-gigabyte capture never needs that much memory.
fn for_each_packet(
    path: &Path,
    stride: usize,
    phase: usize,
    mut on_packet: impl FnMut(&[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let mut reader = io::BufReader::with_capacity(STREAM_CHUNK_LEN, File::open(path)?);
    reader.seek(SeekFrom::Start(phase as u64))?;
    let mut record = vec![0u8; stride];
    loop {
        let mut filled = 0usize;
        while filled < record.len() {
            let read = reader.read(&mut record[filled..])?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        if filled < TS_PACKET_SIZE {
            return Ok(());
        }
        on_packet(&record[..TS_PACKET_SIZE])?;
        if filled < record.len() {
            return Ok(());
        }
    }
}

/// Run the TS diagnostics and the payload plug-ins over `path`, one packet at
/// a time.
///
/// Two bounded passes: the first collects the PMT-declared stream set the
/// codec diagnostics are judged against, the second feeds every packet to the
/// diagnostics and the payload decoders together.
fn run_ts_stream(
    path: &Path,
    stride: usize,
    phase: usize,
    registry: &media_doctor::payload::PayloadRegistry,
    report: &mut Report,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut collector = PmtStreamCollector::new();
    for_each_packet(path, stride, phase, |packet| {
        collector.feed(packet);
        Ok(())
    })?;
    let pmt = collector.finish();

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
    let mut runner = DiagnosticRunner::new(diagnostics, &pmt);
    let mut emit = |sample: media_doctor::payload::DecodedSample<'_>| emit_decoded(sample, json);
    let mut decoder = (!registry.is_empty())
        .then(|| media_doctor::payload::TsPayloadDecoder::new(registry, &mut emit));
    for_each_packet(path, stride, phase, |packet| {
        runner.feed(packet);
        if let Some(decoder) = decoder.as_mut() {
            decoder.feed(packet);
        }
        Ok(())
    })?;
    runner.finish(report);
    if let Some(decoder) = decoder.as_mut() {
        decoder.finish();
    }
    Ok(())
}

/// The container path: an ISOBMFF input is checked by streaming its byte
/// ranges through the demux stage; any other input has nothing for these
/// checks to say.
fn run_container_input(
    file: &mut File,
    len: u64,
    prefix: &[u8],
    registry: &media_doctor::payload::PayloadRegistry,
    report: &mut Report,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if !looks_like_isobmff(prefix) {
        return Ok(());
    }
    check_mp4_input(file, len, report)?;
    if !registry.is_empty() {
        media_doctor::payload::decode_mp4(file, len, registry, &mut |sample| {
            emit_decoded(sample, json)
        })?;
    }
    Ok(())
}

fn run_check(
    args: &CheckArgs,
    registry: &media_doctor::payload::PayloadRegistry,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = File::open(&args.input)?;
    let len = file.metadata()?.len();
    let routed = match probe_file(&mut file, len)? {
        Some(probe) => route_probe(&probe),
        None => Routed::Other,
    };
    let prefix = read_probe_prefix(&mut file, len)?;
    let mut report = Report::new();

    match routed {
        Routed::Ts => match ts_lattice(&prefix) {
            Some((stride, phase)) if len >= phase as u64 + TS_PACKET_SIZE as u64 => {
                run_ts_stream(
                    Path::new(&args.input),
                    stride,
                    phase,
                    registry,
                    &mut report,
                    args.json,
                )?;
            }
            _ => run_container_input(&mut file, len, &prefix, registry, &mut report, args.json)?,
        },
        // An MP4 is checked by streaming its byte ranges through the demux
        // stage; the file is never read whole.
        Routed::Mp4 => {
            check_mp4_input(&mut file, len, &mut report)?;
            if !registry.is_empty() {
                media_doctor::payload::decode_mp4(&mut file, len, registry, &mut |sample| {
                    emit_decoded(sample, args.json)
                })?;
            }
        }
        Routed::Other => {
            run_container_input(&mut file, len, &prefix, registry, &mut report, args.json)?;
        }
    }

    emit_check_report(&report, args.json)
}

/// Print one decoded data-track sample: the header line, then its values.
///
/// With `--json` the report on stdout must stay a single JSON document, so the
/// decoded values go to stderr instead.
fn emit_decoded(sample: media_doctor::payload::DecodedSample<'_>, json: bool) {
    if json {
        eprintln!(
            "track {} sample {} [{}]",
            sample.track, sample.sample, sample.plugin
        );
        for value in sample.values {
            eprintln!("  {}: {}", value.key, scalar_text(&value.value));
        }
    } else {
        println!(
            "track {} sample {} [{}]",
            sample.track, sample.sample, sample.plugin
        );
        for value in sample.values {
            println!("  {}: {}", value.key, scalar_text(&value.value));
        }
    }
}

/// Render one decoded value as text.
fn scalar_text(value: &transmux::payload::PayloadScalar) -> String {
    use transmux::payload::PayloadScalar;
    match value {
        PayloadScalar::Text(text) => text.clone(),
        PayloadScalar::Integer(v) => v.to_string(),
        PayloadScalar::Unsigned(v) => v.to_string(),
        PayloadScalar::Float(v) => v.to_string(),
        PayloadScalar::Bytes(bytes) => broadcast_common::hex::hex_encode(bytes),
        PayloadScalar::Timestamp(micros) => format!("{micros} us"),
        PayloadScalar::Boolean(flag) => flag.to_string(),
        _ => String::from("<unrecognised value>"),
    }
}

/// The routing decision a probe verdict names.
fn route_probe(probe: &container_probe::Probe) -> Routed {
    use container_probe::{Format, Probe};
    match probe {
        Probe::Identified {
            format: Format::MpegTs,
            ..
        } => Routed::Ts,
        Probe::Identified {
            format: Format::Isobmff,
            ..
        } => Routed::Mp4,
        // A tie that includes TS keeps the TS diagnostics, as the whole-file
        // classifier does; any other tie reads whole.
        Probe::Ambiguous { candidates, .. }
            if candidates.iter().any(|c| c.format == Format::MpegTs) =>
        {
            Routed::Ts
        }
        _ => Routed::Other,
    }
}

/// Probe the file's leading prefix, growing it while the probe asks for more
/// and the file has more to give.
fn probe_file(file: &mut File, len: u64) -> io::Result<Option<container_probe::Probe>> {
    let limit = usize::try_from(len)
        .unwrap_or(usize::MAX)
        .min(PROBE_PREFIX_MAX);
    if limit == 0 {
        return Ok(None);
    }
    let mut want = limit.clamp(1, container_probe::DEFAULT_BUDGET);
    loop {
        file.seek(SeekFrom::Start(0))?;
        let mut prefix = vec![0u8; want];
        let read = read_full(file, &mut prefix)?;
        prefix.truncate(read);
        let probe = container_probe::probe_with_budget(&prefix, prefix.len());
        match &probe {
            container_probe::Probe::Insufficient { need_at_least, .. } if prefix.len() < limit => {
                let next = (*need_at_least).min(limit);
                if next <= prefix.len() {
                    return Ok(Some(probe));
                }
                want = next;
            }
            _ => return Ok(Some(probe)),
        }
    }
}

/// Read until `buffer` is full or the file ends.
fn read_full(file: &mut File, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0usize;
    while filled < buffer.len() {
        let read = file.read(&mut buffer[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}

/// Run the container codec checks over an MP4 by driving the seekable demux
/// stage with bounded reads.
fn check_mp4_input(file: &mut File, len: u64, report: &mut Report) -> io::Result<()> {
    check_mp4_file(file, len, report)
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
    let text = read_file_bounded_string(Path::new(&args.input), PLAYLIST_LIMIT)?;
    let mut report = Report::new();
    check_hls_playlist(&text, &mut report);
    emit_report(&report, args.json)
}

fn run_check_dash(args: &CheckDashArgs) -> Result<(), Box<dyn std::error::Error>> {
    let text = read_file_bounded_string(Path::new(&args.input), MPD_LIMIT)?;
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
    let udp_config = args
        .udp_config()
        .map_err(|e| format!("invalid --udp-interface: {e}"))?;

    let socket = media_doctor::udp::bind_udp(udp_addr, &udp_config)?;
    if let (Some(want), Ok(got)) = (
        udp_config.recv_buffer_bytes,
        media_doctor::udp::recv_buffer_size(&socket),
    ) && got < want.get() as usize
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
    let config = MetricsServerConfig::new(args.metrics_max_conns, args.metrics_io_timeout);
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
    let mut buf = [0u8; STREAM_CHUNK_LEN];
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
