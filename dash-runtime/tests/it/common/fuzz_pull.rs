use dash_runtime::client::{DashClient, Error, FailureClass, PullLimits, ResourceId};
use test_fixtures::fuzz::CAPS;

pub const PENDING_CAP: usize = 4_096;
pub const BASE_URL: &str = "http://example/dash/";
pub const MPD_NAME: &str = "stream.mpd";
const WINDOW_MASK: u8 = 0b11;
const CAP_SHIFT: u32 = 2;
const CAP_MASK: u8 = 0b11;

pub fn pull_limits(flags: u8) -> PullLimits {
    let mut limits = PullLimits::default();
    limits.max_inflight_segments = usize::from(flags & WINDOW_MASK) + 1;
    let cap = CAPS[usize::from((flags >> CAP_SHIFT) & CAP_MASK)];
    limits.max_segment_failures = cap;
    limits.max_init_failures = cap;
    limits
}

pub fn deliver_resource(client: &mut DashClient, now_us: u64, id: ResourceId, body: &[u8]) {
    let now = broadcast_common::stage::Timestamp::from_micros(now_us);
    let result = match id {
        ResourceId::Init { .. } => client.on_init(now, id, body),
        ResourceId::Index { .. } => client.on_index(now, id, body),
        _ => client.on_segment(now, id, body),
    };
    if let Err(Error::Rejected { id, class, .. }) = result
        && let FailureClass::Retry { .. } = class
    {
        client.retry(now, id);
    }
}
