//! Build an RTT Echo Request, serialize, parse back, verify.

use broadcast_common::{Parse, Serialize};
use rist_runtime::{RttEcho, RttEchoKind};

fn main() {
    let rtt_echo = RttEcho {
        kind: RttEchoKind::Request,
        ssrc_media: 0xAABBCCDD,
        timestamp: 0x0102030405060708,
        processing_delay_us: 0,
        padding: vec![],
    };

    println!("Built RTT Echo:");
    println!("  Kind: {}", rtt_echo.kind.name());
    println!("  SSRC media: 0x{:08x}", rtt_echo.ssrc_media);
    println!("  Timestamp: 0x{:016x}", rtt_echo.timestamp);
    println!("  Processing delay: {} us", rtt_echo.processing_delay_us);
    println!("  Padding: {} bytes", rtt_echo.padding.len());

    let bytes = rtt_echo.to_bytes();
    println!("Serialized to {} bytes", bytes.len());

    match RttEcho::parse(&bytes) {
        Ok(reparsed) => {
            if reparsed == rtt_echo {
                println!("✓ Round-trip successful: RTT Echo packets match");
            } else {
                println!("✗ Round-trip failed: RTT Echo packets differ");
            }
        }
        Err(e) => {
            println!("✗ Parse error: {}", e);
        }
    }
}
