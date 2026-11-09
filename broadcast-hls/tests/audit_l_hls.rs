use broadcast_hls::MediaPlaylist;

const LL_OPEN_DISCONTINUITY: &str = "#EXTM3U\n\
#EXT-X-VERSION:9\n\
#EXT-X-TARGETDURATION:4\n\
#EXT-X-MEDIA-SEQUENCE:100\n\
#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.5\n\
#EXT-X-PART-INF:PART-TARGET=0.5\n\
#EXT-X-MAP:URI=\"init.mp4\"\n\
#EXT-X-PART:DURATION=0.5,URI=\"s100.0.mp4\",INDEPENDENT=YES\n\
#EXTINF:4.000,\n\
s100.mp4\n\
#EXT-X-DISCONTINUITY\n\
#EXT-X-PART:DURATION=0.5,URI=\"s101.0.mp4\",INDEPENDENT=YES\n";

#[test]
fn discontinuity_before_the_open_segment_survives_parse_and_render() {
    let pl = MediaPlaylist::parse(LL_OPEN_DISCONTINUITY).unwrap();
    assert!(pl.open_segment.is_some());
    let out = pl.to_m3u8().unwrap();
    let disc = out.matches("#EXT-X-DISCONTINUITY\n").count();
    assert_eq!(
        disc, 1,
        "EXT-X-DISCONTINUITY preceding the live-edge parts was dropped; rendered:\n{out}"
    );
}
