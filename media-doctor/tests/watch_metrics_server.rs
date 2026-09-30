//! Audit MD-W9: the `watch` metrics server must not let one idle client block
//! every later scraper.
//!
//! The responder used to serve connections one at a time on a single thread
//! with a blocking `stream.read` and no timeout, so a TCP client that
//! connected and sent nothing (a port scanner, a half-open health check) held
//! the accept loop until it gave up — with `--metrics-addr 0.0.0.0:9090`, the
//! documented deployment, that is any peer on the network.
//!
//! These tests drive the real binary over real sockets: a second client must
//! get a complete `/metrics` response while an earlier client is still idle.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Concurrent-connection cap for the tests that are not about the cap.
const DEFAULT_TEST_MAX_CONNS: usize = 8;
/// Total per-connection deadline the tests run the server with. Small enough
/// that an abandoned connection is reclaimed quickly, large enough that a
/// correct server never trips it.
const TEST_IO_TIMEOUT_MS: u64 = 3_000;

/// A free port.
///
/// Bind-then-drop is inherently racy (another process can take the port in
/// the gap), so retry a few times; and because the server itself is started
/// with the port we chose, a lost race shows up as the server failing to
/// bind, which the readiness poll then times out on. Retrying the whole
/// allocation makes that vanishingly unlikely rather than flaky.
fn free_port() -> u16 {
    for _ in 0..16 {
        let Ok(listener) = TcpListener::bind("127.0.0.1:0") else {
            continue;
        };
        let Ok(addr) = listener.local_addr() else {
            continue;
        };
        // Hold the listener until the address is read, then release it.
        drop(listener);
        if addr.port() != 0 {
            return addr.port();
        }
    }
    panic!("could not obtain a free ephemeral port after 16 attempts");
}

/// Spawn `media-doctor watch` with explicit metrics limits and wait until it
/// is accepting connections.
///
/// The binary prints a startup line on stderr once both sockets are bound; we
/// poll the metrics port instead of parsing it, so the test does not depend on
/// the message's wording.
///
/// Both timeouts are passed in, so the tests use small margins (hundreds of
/// milliseconds) rather than competing with the production 5 s default under
/// load.
fn spawn_watch(max_conns: usize, io_timeout_ms: u64) -> (ChildGuard, u16) {
    let exe = env!("CARGO_BIN_EXE_media-doctor");
    assert!(Path::new(exe).exists(), "cargo must build the binary");

    let udp_port = free_port();
    let metrics_port = free_port();
    let child = Command::new(exe)
        .args([
            "watch",
            "--udp",
            &format!("127.0.0.1:{udp_port}"),
            "--metrics-addr",
            &format!("127.0.0.1:{metrics_port}"),
            "--metrics-max-conns",
            &max_conns.to_string(),
            "--metrics-io-timeout-ms",
            &io_timeout_ms.to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn media-doctor watch");
    let child = ChildGuard(child);

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", metrics_port)).is_ok() {
            return (child, metrics_port);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("media-doctor watch never started listening on {metrics_port}");
}

/// A complete `/metrics` response, read to EOF.
fn scrape(port: u16, timeout: Duration) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to metrics");
    stream
        .set_read_timeout(Some(timeout))
        .expect("set read timeout");
    stream
        .write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .expect("write request");
    let mut buf = String::new();
    let _ = stream.read_to_string(&mut buf);
    buf
}

/// Kills and reaps the `watch` process however the test ends, including a
/// panic — a leaked UDP/TCP listener would fail every later run on the same
/// ephemeral ports.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// An idle connection that has sent nothing must not delay a later scraper.
#[test]
fn idle_client_does_not_block_a_second_scraper() {
    let (_guard, port) = spawn_watch(DEFAULT_TEST_MAX_CONNS, TEST_IO_TIMEOUT_MS);

    // Open a connection and deliberately send nothing, leaving it open.
    let idle = TcpStream::connect(("127.0.0.1", port)).expect("connect idle client");

    // A second, well-behaved scraper must still get a full response quickly.
    let start = Instant::now();
    let response = scrape(port, Duration::from_secs(5));
    let elapsed = start.elapsed();

    assert!(
        response.contains("200 OK") && response.contains("media_doctor_packets_total"),
        "the second scraper must get a complete /metrics response; got {response:?}",
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "the second scraper must not wait on the idle connection (took {elapsed:?})",
    );

    drop(idle);
}

/// Several concurrent scrapers all get a response, and a half-open client
/// left behind does not wedge the accept loop for any of them.
#[test]
fn repeated_scrapes_all_succeed_with_a_stalled_client_outstanding() {
    let (_guard, port) = spawn_watch(DEFAULT_TEST_MAX_CONNS, TEST_IO_TIMEOUT_MS);

    let mut idle_clients = Vec::new();
    for _ in 0..3 {
        idle_clients.push(TcpStream::connect(("127.0.0.1", port)).expect("connect idle"));
    }

    for round in 0..3 {
        let response = scrape(port, Duration::from_secs(5));
        assert!(
            response.contains("200 OK"),
            "scrape {round} must succeed with idle clients outstanding; got {response:?}",
        );
    }
    drop(idle_clients);
}

/// More idle connections than the cap are refused **promptly** — observable
/// directly, so the mechanism is pinned rather than inferred. A server with
/// no cap would hold the extra connection until its own deadline instead.
#[test]
fn connections_beyond_the_cap_are_refused() {
    const CAP: usize = 3;
    let (_guard, port) = spawn_watch(CAP, TEST_IO_TIMEOUT_MS);

    // Fill the cap with idle connections and keep them open.
    let mut held = Vec::new();
    for _ in 0..CAP {
        held.push(TcpStream::connect(("127.0.0.1", port)).expect("connect"));
    }
    // Give the accept loop time to take each slot before probing.
    std::thread::sleep(Duration::from_millis(750));

    // The next connection must be refused promptly: either an explicit 503 or
    // an immediate close. The read timeout is far shorter than the server's
    // own total deadline, so a held slot would time out here.
    let mut over = TcpStream::connect(("127.0.0.1", port)).expect("connect over cap");
    over.set_read_timeout(Some(Duration::from_millis(750)))
        .expect("set read timeout");
    let mut buf = [0u8; 256];
    match over.read(&mut buf) {
        Ok(0) => {} // closed without a response — acceptable refusal
        Ok(n) => {
            let text = String::from_utf8_lossy(&buf[..n]);
            assert!(
                text.starts_with("HTTP/1.1 503"),
                "a connection over the cap must be refused with 503 or closed, got {text:?}",
            );
        }
        Err(e) => panic!(
            "the over-cap connection must be refused promptly, got {e} — a server with no cap holds it instead"
        ),
    }
    drop(held);
}

/// A flood of idle connections beyond the cap must not wedge the server:
/// once the flood is dropped, a fresh scrape succeeds.
#[test]
fn connection_flood_beyond_the_cap_still_serves_after_it_drains() {
    const CAP: usize = 4;
    let (_guard, port) = spawn_watch(CAP, TEST_IO_TIMEOUT_MS);

    let mut flood = Vec::new();
    for _ in 0..(CAP * 6) {
        if let Ok(stream) = TcpStream::connect(("127.0.0.1", port)) {
            flood.push(stream);
        }
    }
    assert!(
        !flood.is_empty(),
        "the flood must have connected at least once",
    );
    drop(flood);

    // Generous, timing-independent margins: poll rather than sleep.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut last = String::new();
    while Instant::now() < deadline {
        last = scrape(port, Duration::from_secs(3));
        if last.contains("200 OK") && last.contains("media_doctor_packets_total") {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("no scrape succeeded within 30s of the flood draining; last response: {last:?}");
}

/// A client that connects and then dribbles one byte at a time must not hold
/// a connection past the server's **total** deadline. A per-read timeout
/// alone does not stop this: each byte resets it, so a slowloris holds a
/// slot indefinitely.
///
/// The server only answers a *complete* request head, so a dribbler never
/// gets a response — the only way this test sees the connection end is the
/// server enforcing its deadline. The client probes with a short read
/// timeout in a loop, so the measurement is the server's deadline, not the
/// client's patience.
#[test]
fn dribbling_client_is_dropped_at_the_total_deadline() {
    const IO_TIMEOUT_MS: u64 = 1_000;
    let (_guard, port) = spawn_watch(DEFAULT_TEST_MAX_CONNS, IO_TIMEOUT_MS);

    let slow = TcpStream::connect(("127.0.0.1", port)).expect("connect slow client");
    // Short client-side reads: the point is to notice the server closing,
    // not to wait on it.
    slow.set_read_timeout(Some(Duration::from_millis(100)))
        .expect("set read timeout");

    // Dribble a byte every 200 ms from a clone, so each write resets any
    // per-read timeout the server might be using.
    let mut writer = slow.try_clone().expect("clone socket");
    let dribbler = std::thread::spawn(move || {
        for _ in 0..100 {
            if writer.write_all(b"G").is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    });

    let start = Instant::now();
    let mut buf = [0u8; 64];
    let mut closed = false;
    let mut slow_reader = slow;
    while start.elapsed() < Duration::from_secs(15) {
        match slow_reader.read(&mut buf) {
            // EOF: the server closed on us.
            Ok(0) => {
                closed = true;
                break;
            }
            // A reset also counts as the server having dropped it.
            Err(e)
                if e.kind() != std::io::ErrorKind::WouldBlock
                    && e.kind() != std::io::ErrorKind::TimedOut =>
            {
                closed = true;
                break;
            }
            // A response would mean the server answered an incomplete
            // request, which it must not do.
            Ok(n) => panic!(
                "the server must not answer an incomplete request head, got {:?}",
                String::from_utf8_lossy(&buf[..n]),
            ),
            Err(_) => {}
        }
    }
    assert!(
        closed,
        "a dribbling client must be dropped at the total deadline ({IO_TIMEOUT_MS} ms), not held for the connection's lifetime",
    );
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(10),
        "the drop must happen at the deadline, not by the test timing out (took {elapsed:?})",
    );
    let _ = dribbler.join();
}

/// The refusal response must be valid HTTP/1.1: CRLF line endings and a
/// blank CRLF-terminated line before the body (`\n`-only framing is not
/// HTTP/1.1 and some clients reject it outright).
#[test]
fn refusal_response_uses_crlf_framing() {
    const CAP: usize = 1;
    let (_guard, port) = spawn_watch(CAP, TEST_IO_TIMEOUT_MS);

    // Fill the single slot.
    let held = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    std::thread::sleep(Duration::from_millis(750));

    let mut over = TcpStream::connect(("127.0.0.1", port)).expect("connect over cap");
    over.set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set read timeout");
    let mut buf = Vec::new();
    let mut chunk = [0u8; 256];
    // The refusal closes the connection right after the response, so read to
    // EOF (or at least through the header block).
    loop {
        match over.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(e) => panic!("reading the refusal failed: {e}"),
        }
    }
    drop(held);

    assert!(
        !buf.is_empty(),
        "the over-cap connection must receive a refusal, not silence",
    );
    let text = String::from_utf8(buf.clone()).expect("refusal is ASCII");
    assert!(
        text.starts_with("HTTP/1.1 503 Service Unavailable\r\n"),
        "the status line must use CRLF; got {text:?}",
    );
    assert!(
        buf.windows(4).any(|w| w == b"\r\n\r\n"),
        "the header block must end with a blank CRLF line; got {text:?}",
    );
    assert!(
        !buf.windows(2).any(|w| w == b"\n\n"),
        "no bare-LF framing may appear; got {text:?}",
    );
    // Every LF must be preceded by CR.
    for (i, &b) in buf.iter().enumerate() {
        if b == b'\n' {
            assert!(
                i > 0 && buf[i - 1] == b'\r',
                "byte {i} is a bare LF; refusal bytes were {text:?}",
            );
        }
    }
}
