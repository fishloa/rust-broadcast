//! Adversarial review (dvb-si tables/core batch): `t2mi --inner` without
//! `--plp` now opens the input TWICE (a PLP-discovery scan, then the real
//! pass, #1034). For a non-seekable input (a pipe via `/dev/stdin`, a FIFO,
//! process substitution) the discovery pass consumes the whole stream, the
//! second open reads nothing, and the tool exits 0 having written zero
//! packets. v11.1.0 read the input once and recovered the inner TS.
#![cfg(unix)]

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn inner_from_a_pipe_recovers_the_same_packets_as_from_the_file() {
    let path = test_fixtures::path("dvb-t2mi/colombia-capital-t2mi.ts");
    let data = std::fs::read(&path).expect("fixture");
    let bin = env!("CARGO_BIN_EXE_dvb-tools");

    let from_file = Command::new(bin)
        .args(["t2mi", path.to_str().unwrap(), "--pid", "0x0040", "--inner"])
        .output()
        .expect("spawn");
    assert!(from_file.status.success());
    assert!(
        !from_file.stdout.is_empty(),
        "the single-PLP capture yields inner packets from a file"
    );

    let mut child = Command::new(bin)
        .args(["t2mi", "/dev/stdin", "--pid", "0x0040", "--inner"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&data);
    });
    let from_pipe = child.wait_with_output().expect("wait");
    writer.join().unwrap();

    assert_eq!(
        from_pipe.stdout.len(),
        from_file.stdout.len(),
        "piped input must recover the same inner TS (exit {:?}, stderr: {})",
        from_pipe.status,
        String::from_utf8_lossy(&from_pipe.stderr)
    );
}
