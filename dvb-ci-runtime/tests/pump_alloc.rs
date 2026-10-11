//! r10-O-11: `Driver::pump` must not copy each received frame before handing
//! it to the stack. A counting global allocator records the largest single
//! allocation made during one `pump` of a full-size junk frame, so the check
//! is deterministic (no timing).

use std::time::Duration;

use dvb_ci_runtime::{Driver, MockCaDevice};

#[global_allocator]
static A: test_alloc::ProcessCounting = test_alloc::ProcessCounting::new();

#[test]
fn pump_does_not_copy_the_received_frame() {
    const FRAME: usize = 4096; // the driver's whole receive buffer
    let mock = MockCaDevice::new([vec![0u8; FRAME]]);
    let mut driver = Driver::new(mock);
    A.reset_largest();
    let _ = driver.pump(Duration::from_millis(1));
    let largest = A.largest();
    eprintln!("largest single allocation during pump: {largest} bytes");
    assert!(
        largest < FRAME / 2,
        "largest single allocation was {largest} bytes: the frame was copied"
    );
}
