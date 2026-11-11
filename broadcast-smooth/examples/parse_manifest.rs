//! Parse a real Smooth Streaming client Manifest — ffmpeg's own
//! `smoothstreaming` muxer output, a committed fixture — into the typed model,
//! and render it back.
//!
//! Run with `cargo run -p broadcast-smooth --example parse_manifest`.

use broadcast_smooth::SmoothManifest;

fn main() {
    let path = test_fixtures::path("smooth/av/Manifest");
    let xml = std::fs::read_to_string(path).expect("read the committed Smooth manifest fixture");

    let manifest = SmoothManifest::parse(&xml).expect("valid client manifest");
    println!(
        "version {}.{} timescale {} duration {:?} live {}",
        manifest.major_version,
        manifest.minor_version,
        manifest.timescale,
        manifest.duration,
        manifest.is_live
    );
    for stream in &manifest.streams {
        println!(
            "  {} stream ({} qualities, url {:?})",
            stream.stream_type.name(),
            stream.qualities.len(),
            stream.url
        );
        for quality in &stream.qualities {
            println!(
                "    {} bps {} {} bytes of CodecPrivateData",
                quality.bitrate,
                quality.four_cc,
                quality.codec_private_data.len()
            );
        }
        let chunks = stream.enumerate_chunks().expect("bounded chunk timeline");
        println!("    {} fragments", chunks.len());
    }

    assert_eq!(
        SmoothManifest::parse(&manifest.to_xml()).expect("re-parse"),
        manifest,
        "the model round-trips through the serializer"
    );
}
