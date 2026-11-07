use super::common::live_fuzz_driver::run;

fn seeded(len: usize, seed: u64) -> Vec<u8> {
    let mut rng = super::common::Rng(seed);
    (0..len).map(|_| rng.next().to_le_bytes()[3]).collect()
}

#[test]
fn driver_accepts_in_range_inputs_without_panicking() {
    for seed in 1..=64u64 {
        let data = seeded(64 + usize::try_from(seed).unwrap() * 37, seed);
        assert!(run(&data, &test_fixtures::read), "seed {seed}");
    }
}

#[test]
fn driver_rejects_inputs_outside_the_size_range() {
    assert!(!run(&[0; 5], &test_fixtures::read));
    assert!(!run(
        &vec![0; test_fixtures::fuzz::MAX_INPUT + 1],
        &test_fixtures::read
    ));
}
