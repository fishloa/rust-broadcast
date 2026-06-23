//! Drive a CAM through the `Driver` against an in-memory `MockCaDevice`.
//!
//! Shows the host-facing loop: `init` → `pump` (poll cadence) →
//! `take_notifications`. The same `Driver` API runs against the real Linux
//! `/dev/dvb/adapterN/caM` device (the `linux` feature) — only the `CaDevice`
//! changes.
//!
//! Run: `cargo run -p dvb-ci-runtime --example mock_cam_session`

use std::time::Duration;

use dvb_ci_runtime::dvb_ci::tpdu::tags;
use dvb_ci_runtime::{Driver, MockCaDevice, Notification};

fn main() -> std::io::Result<()> {
    // Script a module that accepts the transport connection (C_T_C_Reply).
    let dev = MockCaDevice::new([vec![tags::C_T_C_REPLY, 0x01, 0x01]]);
    let mut driver = Driver::new(dev);

    // Bring the interface up: reset + open the transport connection.
    driver.init()?;
    println!("init: sent {} device op(s)", driver.device().ops.len());

    // Pump the device. When readable it reads a frame and feeds the stack;
    // otherwise it advances the EN 50221 poll cadence by the timeout.
    for step in 0..5 {
        let read = driver.pump(Duration::from_millis(100))?;
        println!("pump {step}: processed_frame={read}");
    }

    // Anything the host application needs to act on surfaces as a Notification.
    for note in driver.take_notifications() {
        match note {
            Notification::CamReady => println!("note: CAM ready — safe to send ca_pmt"),
            Notification::ApplicationInfo { menu, .. } => {
                println!("note: application_information menu={menu:?}")
            }
            Notification::CaInfo { ca_system_ids } => {
                println!("note: ca_info system_ids={ca_system_ids:?}")
            }
            other => println!("note: {other:?}"),
        }
    }

    // The mock records every device op (writes/ioctls) — handy for assertions.
    println!("total recorded device ops: {}", driver.device().ops.len());
    Ok(())
}
