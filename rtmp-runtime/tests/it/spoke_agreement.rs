#[test]
fn transmux_rtmp_spoke_uses_the_same_default_chunk_size() {
    assert_eq!(
        usize::try_from(rtmp_runtime::chunk::DEFAULT_CHUNK_SIZE).expect("chunk size fits usize"),
        transmux::rtmp::DEFAULT_CHUNK_SIZE
    );
}
