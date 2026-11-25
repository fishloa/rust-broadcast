//! CENC/CBCS encrypt walkthrough — issue #564.
//!
//! Drives the whole encrypt path end to end through public APIs only:
//!
//! ```text
//! cleartext Media (TsDemux over fixtures/ts/h264/main.ts, AVC track)
//!   -> CencEncryptStage         (sets sample + track protection, ciphers samples)
//!   -> CmafMuxStage             (CMAF muxer stage)
//!   -> protect_init_segment     (splice in tenc/sinf; the fragment writer
//!                                emits senc/saiz/saio itself)
//!   -> a standalone, encrypted CMAF file written to a temp path
//! ```
//!
//! and then shows the two signalling surfaces added in issue #564's final
//! task, both driven straight off the encrypted `Media`'s
//! [`transmux::TrackProtection`]:
//!
//! - [`transmux::DashPackager`] auto-derives a `<ContentProtection
//!   schemeIdUri="urn:mpeg:dash:mp4protection:2011">` element from
//!   `TrackSpec::protection` / `Sample::protection` — no extra wiring needed.
//! - [`broadcast_hls::cenc_ext_x_key`] renders the HLS `#EXT-X-KEY` tag for
//!   both CENC schemes (`cbcs` -> `SAMPLE-AES`, `cenc` ->
//!   `SAMPLE-AES-CTR`, RFC 8216bis §4.4.4.4). HLS playlist syntax lives in
//!   the `broadcast-hls` crate
//!   (issue #878), but both crates name the scheme with the *same*
//!   [`broadcast_common::CencScheme`], so no conversion is needed.
//!
//! Run it with:
//!
//! ```text
//! cargo run -p transmux --example cenc_encrypt --features cenc
//! ```

/// One CMAF init segment followed by one media segment.
fn cmaf_bytes(media: &transmux::Media) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut stage =
        transmux::CmafMuxStage::new(media.movie_timescale, transmux::FragmentCut::OnFlush, 1);
    let outputs =
        transmux::mux_stage::drive_media(&mut stage, media, broadcast_common::Timestamp::ZERO)?;
    let mut bytes = Vec::new();
    for output in outputs {
        match output {
            transmux::mux_stage::MuxOutput::Init(part)
            | transmux::mux_stage::MuxOutput::Bytes(part) => bytes.extend_from_slice(&part),
            transmux::mux_stage::MuxOutput::Segment(segment) => {
                bytes.extend_from_slice(&segment.bytes);
            }
            _ => {}
        }
    }
    Ok(bytes)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "cenc")]
    {
        run()?;
    }
    #[cfg(not(feature = "cenc"))]
    {
        println!("transmux built without the `cenc` feature; nothing to encrypt.");
    }
    Ok(())
}

#[cfg(feature = "cenc")]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use broadcast_common::stage::Stage;
    use broadcast_hls::cenc_ext_x_key;
    use transmux::cenc_segment::protect_init_segment;
    use transmux::pipeline::CodecConfig;
    use transmux::{
        CencEncryptStage, CencScheme, DashPackager, EncryptConfig, IvGen, Media, SubsamplePolicy,
        TrackProtection, select_tracks_by,
    };

    // A test KID/key (never a real production key) — the standard `cbcs`
    // constant-IV convention (docs/superpowers/specs/2026-07-13-cenc-encrypt-design.md
    // §5; confirmed against Bento4's `mp4encrypt`, which always emits a
    // `tenc.default_constant_IV` for `cbcs`).
    const KID: [u8; 16] = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ];
    const KEY: [u8; 16] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f,
    ];
    const CONSTANT_IV: [u8; 16] = [
        0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0xfb, 0xfc, 0xfd, 0xfe,
        0xff,
    ];

    // 1. Read a real cleartext fixture (fixture-less checkouts still build
    //    and run cleanly, per the crate's example convention).
    let path = test_fixtures::path("ts/h264/main.ts");
    let ts = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) => {
            println!(
                "fixture {} unavailable ({err}); nothing to do.",
                path.display()
            );
            return Ok(());
        }
    };
    println!("read {} bytes from {}\n", ts.len(), path.display());

    // 2. Demux to the neutral IR, keeping only the AVC video track.

    let media: Media = test_harness::collect::media_from_ts(&ts[..])?;
    let media = select_tracks_by(&media, |t| matches!(t.spec.config, CodecConfig::Avc { .. }))?;
    println!(
        "demuxed IR: {} track(s), {} sample(s)",
        media.tracks.len(),
        media.tracks[0].samples.len()
    );

    // 3. Encrypt in place: `cbcs` (AES-128-CBC, 1:9 pattern) — the CMAF-HLS
    //    case, so the HLS signalling step below has something to render.
    let cfg = EncryptConfig::new(CencScheme::Cbcs, KID, SubsamplePolicy::Video)
        .with_iv(IvGen::Constant(CONSTANT_IV))
        .with_pattern((1, 9));
    let mut stage = CencEncryptStage::new(KEY, cfg)?;
    let mut outputs = Vec::new();
    for track in &media.tracks {
        stage.feed(
            transmux::mux_stage::MuxInput::Track(Box::new(track.spec.clone())),
            broadcast_common::stage::Timestamp::ZERO,
        )?;
        outputs.extend(std::iter::from_fn(|| stage.poll()));
        for sample in &track.samples {
            stage.feed(
                transmux::mux_stage::MuxInput::Sample {
                    track: track.spec.track_id,
                    sample: sample.clone(),
                },
                broadcast_common::stage::Timestamp::ZERO,
            )?;
            outputs.extend(std::iter::from_fn(|| stage.poll()));
        }
    }
    stage.finish()?;
    outputs.extend(std::iter::from_fn(|| stage.poll()));
    let mut encrypted_tracks = Vec::new();
    let mut index = std::collections::BTreeMap::new();
    for item in outputs {
        match item {
            transmux::mux_stage::MuxInput::Track(spec) => {
                index.insert(spec.track_id, encrypted_tracks.len());
                encrypted_tracks.push(transmux::Track::new(*spec, Vec::new()));
            }
            transmux::mux_stage::MuxInput::Sample { track, sample } => {
                if let Some(&at) = index.get(&track) {
                    encrypted_tracks[at].samples.push(sample);
                }
            }
            _ => {}
        }
    }
    let mut media = Media::new(encrypted_tracks, media.movie_timescale);
    media.skipped = Vec::new();
    let track_id = media.tracks[0].spec.track_id;
    let encryption: TrackProtection = media.tracks[0]
        .spec
        .protection
        .as_ref()
        .expect("CencEncryptStage populates TrackSpec::protection")
        .clone();
    println!(
        "encrypted: scheme={} kid={}",
        encryption.scheme.name(),
        broadcast_common::hex::hex_encode(&encryption.tenc.default_kid)
    );

    // 4. Mux the encrypted IR (CmafMuxStage) into one combined
    //    ftyp+moov+styp+moof+mdat buffer. The fragment writer emits each
    //    sample's `senc` from its own protection; the init segment's
    //    `sinf`/`tenc` is the one post-processing pass left to do.
    let raw = cmaf_bytes(&media)?;
    let protected = protect_init_segment(&raw, track_id, &encryption)?;

    let out_path = std::env::temp_dir().join("transmux_cenc_encrypt_example.mp4");
    std::fs::write(&out_path, &protected)?;
    println!(
        "wrote {} encrypted CMAF bytes to {}",
        protected.len(),
        out_path.display()
    );

    // 5. DASH signalling: `DashPackager` auto-derives the "common"
    //    ContentProtection from `TrackSpec::protection` — no extra wiring.
    let specs: Vec<transmux::TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let log = transmux::segment_log::SegmentSummaryLog::new();
    let mpd = DashPackager::default().render(&specs, &log)?;
    let cp_line = mpd
        .lines()
        .find(|l| l.contains("ContentProtection"))
        .unwrap_or("(none)");
    println!("\n--- DASH signalling ---\n{}", cp_line.trim());

    // 6. HLS signalling: `cenc_ext_x_key` renders `#EXT-X-KEY` for both
    //    CENC schemes (RFC 8216bis §4.4.4.4). No conversion needed —
    //    `transmux::CencScheme` and the scheme
    //    `broadcast_hls::cenc_ext_x_key` takes are the *same*
    //    `broadcast_common::CencScheme`.
    let key_uri = "https://keyserver.example.com/key";
    match cenc_ext_x_key(encryption.scheme, &encryption.tenc.default_kid, key_uri) {
        Ok(Some(tag)) => println!("--- HLS signalling ---\n{tag}"),
        Ok(None) => println!(
            "--- HLS signalling ---\n{} has no HLS key tag (DASH-only)",
            encryption.scheme.name()
        ),
        Err(e) => println!("--- HLS signalling ---\ninvalid key_uri: {e}"),
    }

    Ok(())
}
