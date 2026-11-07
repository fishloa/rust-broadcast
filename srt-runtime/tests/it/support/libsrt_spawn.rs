use test_fixtures::process::free_udp_port;

const LIBSRT_SPAWN_ATTEMPTS: usize = 3;
const LIBSRT_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// With `-loglevel:debug -lfa:chn-mgmt`, `srt-live-transmit` 1.5.x prints
/// `D:SRT.km: CHANNEL: Bound to local address: 0.0.0.0:<port>` once its SRT
/// socket owns the port, and exits with `srt_bind` failure if it could not.
const LIBSRT_BOUND_LINE: &str = "Bound to local address";

/// With `-verbose`, `srt-live-transmit` 1.5.x opens its `udp://` source before
/// the SRT target and prints `Opening SRT target ...` only once that source is
/// bound; a taken port prints `Binding address for UDP: Address already in use`
/// and exits instead. libsrt does not set `SO_REUSEADDR` on that source.
const LIBSRT_UDP_SOURCE_BOUND_LINE: &str = "Opening SRT target";

/// Spawns `srt-live-transmit` with the arguments `args(port)` for a port picked
/// here, and returns once its stderr has shown every line of `ready`. The port
/// is released before the external process binds it, so another process can
/// take it in between and libsrt then exits; such a setup is retried with a new
/// port, at most `LIBSRT_SPAWN_ATTEMPTS` times, and only this setup step is ever
/// retried.
fn spawn_libsrt_ready(
    ready: &[&str],
    args: impl Fn(u16) -> Vec<String>,
) -> (std::process::Child, u16) {
    let mut captured = Vec::new();
    for _ in 0..LIBSRT_SPAWN_ATTEMPTS {
        let port = free_udp_port();
        let mut child = std::process::Command::new("srt-live-transmit")
            .args(["-verbose", "-loglevel:debug", "-lfa:chn-mgmt"])
            .args(args(port))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn srt-live-transmit");
        let stderr = child.stderr.take().expect("piped stderr");
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(stderr)
                .lines()
                .map_while(Result::ok)
            {
                let _ = tx.send(line);
            }
        });
        let mut pending: Vec<&str> = ready.to_vec();
        let deadline = std::time::Instant::now() + LIBSRT_READY_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match rx.recv_timeout(left) {
                Ok(line) => {
                    pending.retain(|marker| !line.contains(marker));
                    captured.push(line);
                    if pending.is_empty() {
                        return (child, port);
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = child.wait();
                    break;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "srt-live-transmit never reported {pending:?}; stderr:\n{}",
                        captured.join("\n")
                    );
                }
            }
        }
    }
    panic!(
        "srt-live-transmit exited before reporting {ready:?} on {LIBSRT_SPAWN_ATTEMPTS} ports; stderr:\n{}",
        captured.join("\n")
    );
}
