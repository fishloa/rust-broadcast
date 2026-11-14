use std::time::Duration;

pub const LL_TEST_PUBLISH_TIME: &str = "2026-01-01T00:00:00Z";

pub fn ll_packager(
    segment_secs: f64,
    chunk_secs: f64,
    latency: Duration,
) -> transmux::ll_dash::LlDashPackager {
    let mut pkg = transmux::ll_dash::LlDashPackager::new(
        segment_secs,
        chunk_secs,
        latency,
        LL_TEST_PUBLISH_TIME,
    )
    .expect("packager");
    pkg.base.publish_time = Some(LL_TEST_PUBLISH_TIME.to_string());
    pkg
}
