fn free_udp_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .expect("bind free port")
        .local_addr()
        .expect("local addr")
        .port()
}

const LIBSRT_SPAWN_ATTEMPTS: usize = 3;
const LIBSRT_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// With `-loglevel:debug -lfa:chn-mgmt`, `srt-live-transmit` 1.5.x prints
/// `D:SRT.km: CHANNEL: Bound to local address: 0.0.0.0:<port>` once its SRT
/// socket owns the port, and exits with `srt_bind` failure if it could not.
const LIBSRT_BOUND_LINE: &str = "Bound to local address";

/// Spawns `srt-live-transmit` with the arguments `args(port)` for a port picked
/// here, and returns once it reports the port bound. The external process must
/// choose the bind itself, so a taken port is retried with a new one, at most
/// `LIBSRT_SPAWN_ATTEMPTS` times; only this setup step is ever retried.
fn spawn_libsrt_ready(args: impl Fn(u16) -> Vec<String>) -> (std::process::Child, u16) {
    let mut captured = Vec::new();
    for _ in 0..LIBSRT_SPAWN_ATTEMPTS {
        let port = free_udp_port();
        let mut child = std::process::Command::new("srt-live-transmit")
            .args(["-loglevel:debug", "-lfa:chn-mgmt"])
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
        let deadline = std::time::Instant::now() + LIBSRT_READY_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match rx.recv_timeout(left) {
                Ok(line) if line.contains(LIBSRT_BOUND_LINE) => {
                    return (child, port);
                }
                Ok(line) => captured.push(line),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = child.wait();
                    break;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "srt-live-transmit never bound a port; stderr:\n{}",
                        captured.join("\n")
                    );
                }
            }
        }
    }
    panic!(
        "srt-live-transmit exited before binding on {LIBSRT_SPAWN_ATTEMPTS} ports; stderr:\n{}",
        captured.join("\n")
    );
}
