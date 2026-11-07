use rtmp_runtime::target::{RtmpTarget, RtmpUrlError};

#[test]
fn a_hostless_base_url_is_no_host() {
    assert_eq!(
        RtmpTarget::from_parts("rtmp:live", "live", "key").unwrap_err(),
        RtmpUrlError::NoHost
    );
}
