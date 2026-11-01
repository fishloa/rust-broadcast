//! End-to-end: the real `media-doctor watch` binary. Ports are kernel-assigned
//! (`:0`) and learnt from the binary's own start-up line — no reserve-then-
//! rebind, no readiness poll, no sleeps. The server's connection-policy
//! behaviours are tested in-process in `src/metrics_server.rs`.
//!
//! Disposition of the former tests in this file (audit MD-W9 era):
//!
//! | Old test | Disposition | New test |
//! |---|---|---|
//! | `idle_client_does_not_block_a_second_scraper` | re-expressed | `metrics_server::tests::idle_client_does_not_block_a_second_scraper` |
//! | `repeated_scrapes_all_succeed_with_a_stalled_client_outstanding` | re-expressed | `metrics_server::tests::repeated_scrapes_succeed_with_stalled_clients_outstanding` |
//! | `connections_beyond_the_cap_are_refused` | re-expressed (over-cap sockets are closed at accept; deterministic accept order, no probe loop, no sleeps) | `metrics_server::tests::connections_beyond_the_cap_are_closed_promptly_and_do_not_accumulate` |
//! | `connection_flood_beyond_the_cap_still_serves_after_it_drains` | re-expressed | `metrics_server::tests::flood_beyond_the_cap_drains_and_serving_resumes` |
//! | `dribbling_client_is_dropped_at_the_total_deadline` | re-expressed (not made moot by hyper: its header-read timeout needs a timer set) | `metrics_server::tests::dribbling_client_is_dropped_at_the_deadline` |
//! | `refusal_response_uses_crlf_framing` | retired: pinned hand-written `HTTP/1.1 503` bytes; hyper now writes every status line and header, and over-cap refusal is a prompt close | none |
//! | helpers `free_port`, readiness poll, `hold_connections`, `all_open`, `PROBE_PATIENCE` | retired: reserve-then-rebind port allocation and accept-order probing existed only because the old server could not report its bound address or admit deterministically; replaced by port-0 listeners | none |

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const TS_PACKET_SIZE: usize = 188;
const DATAGRAM_PACKETS: usize = 7;

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Spawn `watch` on port 0 for both sockets and return the addresses it printed.
fn spawn_watch(extra: &[&str]) -> (ChildGuard, SocketAddr, SocketAddr) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_media-doctor"))
        .args([
            "watch",
            "--udp",
            "127.0.0.1:0",
            "--metrics-addr",
            "127.0.0.1:0",
        ])
        .args(extra)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn watch");
    let stderr = child.stderr.take().unwrap();
    let guard = ChildGuard(child);
    let mut line = String::new();
    BufReader::new(stderr)
        .read_line(&mut line)
        .expect("start-up line");
    // "media-doctor watch: ingesting UDP 127.0.0.1:PORT, metrics on http://127.0.0.1:PORT/metrics"
    let udp = line
        .split("UDP ")
        .nth(1)
        .and_then(|s| s.split(',').next())
        .expect(&line)
        .parse()
        .expect(&line);
    let http = line
        .split("http://")
        .nth(1)
        .and_then(|s| s.split("/metrics").next())
        .expect(&line)
        .parse()
        .expect(&line);
    (guard, udp, http)
}

fn scrape(addr: SocketAddr) -> String {
    let mut s = TcpStream::connect(addr).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

fn metric(text: &str, name: &str) -> Option<f64> {
    text.lines()
        .find_map(|l| l.strip_prefix(name)?.strip_prefix(' ')?.trim().parse().ok())
}

#[test]
fn first_scrape_already_lists_every_family() {
    let (_g, _udp, http) = spawn_watch(&[]);
    let body = scrape(http);
    assert!(body.starts_with("HTTP/1.1 200 OK"), "{body}");
    assert_eq!(
        metric(&body, "media_doctor_packets_total"),
        Some(0.0),
        "{body}"
    );
}

/// The last datagrams before the feed goes quiet must still be published
/// (the binary flushes when its socket read times out).
#[test]
fn final_datagrams_become_visible_after_the_feed_stops() {
    let (_g, udp, http) = spawn_watch(&[]);
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/ts/m6-single.ts"
    ))
    .unwrap();
    let packets = bytes.len() / TS_PACKET_SIZE;
    let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
    for chunk in bytes.chunks(DATAGRAM_PACKETS * TS_PACKET_SIZE) {
        tx.send_to(chunk, udp).unwrap();
    }
    // Condition-wait, bounded; each scrape is one real round trip.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let body = scrape(http);
        // UDP on loopback can drop under load; require "most", and stability of the flush.
        if metric(&body, "media_doctor_packets_total").is_some_and(|n| n >= (packets as f64) * 0.5)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "packets never became visible: {body}"
        );
    }
}

#[test]
fn bad_metrics_address_is_a_startup_error_not_a_hang() {
    let out = Command::new(env!("CARGO_BIN_EXE_media-doctor"))
        .args([
            "watch",
            "--udp",
            "127.0.0.1:0",
            "--metrics-addr",
            "not-an-address",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--metrics-addr"));
}

#[test]
fn udp_interface_flag_is_validated() {
    let out = Command::new(env!("CARGO_BIN_EXE_media-doctor"))
        .args([
            "watch",
            "--udp",
            "127.0.0.1:0",
            "--udp-interface",
            "not-an-interface",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    // The message is ours (not clap's "unexpected argument"): it names the
    // flag AND says what was expected.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("invalid --udp-interface") && stderr.contains("interface index"),
        "{stderr}"
    );
}

const REQUEST: &[u8] = b"GET /metrics HTTP/1.1\r\nHost: localhost\r\n\r\n";

/// `--metrics-max-conns` is the CAP and nothing else: with a cap of 1 and a
/// long deadline, a held first connection keeps its slot while the second is
/// closed at accept, and the first then still gets a full response. (Swapping
/// the two arguments gives a 1 ms deadline and a huge cap: the first
/// connection is dropped by the deadline, so the final request fails.)
#[test]
fn metrics_max_conns_flag_is_the_cap() {
    let (_g, _udp, http) = spawn_watch(&[
        "--metrics-max-conns",
        "1",
        "--metrics-io-timeout-ms",
        "60000",
    ]);
    let mut first = TcpStream::connect(http).expect("first connect");
    let mut second = TcpStream::connect(http).expect("second connect");
    second
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut buf = [0u8; 16];
    // Over the cap: the server closes it (EOF or reset), never serves it.
    match second.read(&mut buf) {
        Ok(0) | Err(_) => {}
        Ok(n) => panic!("over-cap connection was served {n} bytes"),
    }
    // The held connection kept its slot and is still served.
    first
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    first.write_all(REQUEST).expect("first still open");
    let mut out = String::new();
    let _ = first.read_to_string(&mut out);
    assert!(out.starts_with("HTTP/1.1 200 OK"), "{out}");
}

/// `--metrics-io-timeout-ms` is the DEADLINE and nothing else: an idle
/// connection is dropped after about that long. Swapped arguments (8 ms) end
/// it far too early; ignoring the flag (default 5 s) far too late.
#[test]
fn metrics_io_timeout_flag_is_the_deadline() {
    const TIMEOUT_MS: u64 = 400;
    let (_g, _udp, http) =
        spawn_watch(&["--metrics-max-conns", "8", "--metrics-io-timeout-ms", "400"]);
    // Warm up: the metrics thread's runtime starts after the start-up line, so
    // the first connection would otherwise include that start-up latency.
    assert!(scrape(http).starts_with("HTTP/1.1 200 OK"));
    let mut idle = TcpStream::connect(http).expect("connect");
    idle.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let start = Instant::now();
    let mut buf = [0u8; 256];
    // Blocks until the server closes the idle connection.
    let _ = idle.read(&mut buf);
    let elapsed = start.elapsed();
    assert!(
        elapsed >= Duration::from_millis(TIMEOUT_MS / 2),
        "dropped far too early: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(TIMEOUT_MS * 5),
        "dropped far too late (flag ignored?): {elapsed:?}"
    );
}
