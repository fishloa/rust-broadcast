#[global_allocator]
static A: test_alloc::ProcessPeak = test_alloc::ProcessPeak::new();

#[test]
fn peak_during_reports_the_high_water_above_entry() {
    const HELD: usize = 8 << 20;
    let (peak, ()) = A.peak_during(|| drop(vec![0u8; HELD]));
    assert!(peak >= HELD, "peak {peak}");
    assert!(peak < HELD + (1 << 20), "peak {peak}");
    let (after, ()) = A.peak_during(|| ());
    assert!(after < 1 << 16, "nothing held: {after}");
}
