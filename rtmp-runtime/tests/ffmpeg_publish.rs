//! End-to-end oracle (#1085): a real `ffmpeg` publishes H.264 + AAC over
//! RTMP to the tokio server adapter and this crate must receive both audio
//! and video media events. Skips (printing why) when `ffmpeg` is not on
//! `PATH`.
#![cfg(feature = "tokio")]

use std::process::Stdio;
use std::time::Duration;

use rtmp_runtime::io::AsyncRtmpServer;
use rtmp_runtime::server::{ServerConfig, ServerEvent};

/// FLV tag types carried by `ServerEvent::Media` (Adobe FLV v10.1 E.4.1).
const TAG_AUDIO: u8 = 8;
const TAG_VIDEO: u8 = 9;
/// FLV file header (9) + PreviousTagSize0 (4).
const FLV_FILE_HEADER_LEN: usize = 13;

#[tokio::test]
async fn ffmpeg_publish_delivers_audio_and_video() {
    if std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_err()
    {
        eprintln!("skipping ffmpeg_publish_delivers_audio_and_video: ffmpeg not on PATH");
        return;
    }

    let server = AsyncRtmpServer::bind("127.0.0.1:0", ServerConfig::default())
        .await
        .expect("bind");
    let port = server.local_addr().expect("local_addr").port();

    let mut ffmpeg = std::process::Command::new("ffmpeg")
        .args([
            "-loglevel",
            "error",
            "-re",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=duration=3:size=320x240:rate=25",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=3",
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
            "-f",
            "flv",
        ])
        .arg(format!("rtmp://127.0.0.1:{port}/app/key"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn ffmpeg");

    // Watchdog: a wedged publish must fail the test, not hang it.
    let pid = ffmpeg.id();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(40));
        let _ = std::process::Command::new("kill")
            .arg(pid.to_string())
            .status();
    });
    let mut conn = server.accept().await.expect("accept");
    let (mut audio, mut video, mut published) = (0usize, 0usize, false);
    let drive = async {
        while let Some(batch) = conn.next_events().await.expect("next_events") {
            for e in batch {
                match e {
                    ServerEvent::Publish { .. } => published = true,
                    ServerEvent::Media { flv } => {
                        // The first event is prefixed with the 13-byte FLV
                        // file header + PreviousTagSize0.
                        let skip = if flv.starts_with(b"FLV") {
                            FLV_FILE_HEADER_LEN
                        } else {
                            0
                        };
                        match flv[skip] {
                            TAG_AUDIO => audio += 1,
                            TAG_VIDEO => video += 1,
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
    };
    drive.await;
    let _ = ffmpeg.wait();
    assert!(published, "no Publish event");
    assert!(audio > 0, "no audio media events");
    assert!(video > 0, "no video media events");
}
