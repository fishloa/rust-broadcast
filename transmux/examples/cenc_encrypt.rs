//! CENC/CBCS encrypt walkthrough — issue #564.
//!
//! Drives the encrypt path end to end through public APIs only, in bounded
//! memory:
//!
//! ```text
//! TS fixture read in 64 KiB pieces
//!   -> StreamingTsDemux       (events; only the AVC track is kept)
//!   -> CencEncryptStage       (sets sample + track protection, ciphers samples)
//!   -> CmafMuxStage           (CMAF muxer stage)
//!   -> protect_init_segment   (splice in tenc/sinf; the fragment writer
//!                             emits senc/saiz/saio itself)
//!   -> a standalone, encrypted CMAF file written to a temp path
//! ```
//!
//! and then shows the two signalling surfaces added in issue #564's final
//! task, both driven straight off the encrypted track's
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
//!   [`broadcast_core::CencScheme`], so no conversion is needed.
//!
//! Run it with:
//!
//! ```text
//! cargo run -p transmux --example cenc_encrypt --features cenc
//! ```

/// Size of one bounded read from the input file.
const READ_BUF: usize = 64 * 1024;

#[cfg(feature = "cenc")]
use broadcast_common::stage::Stage;

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
    use broadcast_hls::cenc_ext_x_key;
    use transmux::{
        CencEncryptStage, CencScheme, CmafMuxStage, DashPackager, EncryptConfig, FragmentCut,
        IvGen, StreamingTsDemux, SubsamplePolicy, VIDEO_CLOCK_TIMESCALE,
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

    let path = test_fixtures::path("ts/h264/main.ts");
    let mut input = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(err) => {
            println!(
                "fixture {} unavailable ({err}); nothing to do.",
                path.display()
            );
            return Ok(());
        }
    };

    let out_path = std::env::temp_dir().join("transmux_cenc_encrypt_example.mp4");
    let cfg = EncryptConfig::new(CencScheme::Cbcs, KID, SubsamplePolicy::Video)
        .with_iv(IvGen::Constant(CONSTANT_IV))
        .with_pattern((1, 9));
    let mut pipeline = Pipeline {
        encrypt: CencEncryptStage::new(KEY, cfg)?,
        mux: CmafMuxStage::new(VIDEO_CLOCK_TIMESCALE, FragmentCut::OnFlush, 1),
        output: std::fs::File::create(&out_path)?,
        video: None,
        specs: Vec::new(),
        protection: None,
        protection_track: None,
        sample_count: 0,
        written: 0,
    };
    let mut demux = StreamingTsDemux::new();
    let mut buf = [0u8; READ_BUF];

    loop {
        let read = std::io::Read::read(&mut input, &mut buf)?;
        if read == 0 {
            break;
        }
        demux.feed(&buf[..read])?;
        pipeline.pump(&mut demux)?;
    }
    demux.finish()?;
    pipeline.pump(&mut demux)?;
    pipeline.finish()?;

    let encryption = pipeline
        .protection
        .as_ref()
        .expect("CencEncryptStage populates TrackSpec::protection");
    println!(
        "encrypted {} sample(s): scheme={} kid={}",
        pipeline.sample_count,
        encryption.scheme.name(),
        broadcast_common::hex::hex_encode(&encryption.tenc.default_kid)
    );
    println!(
        "wrote {} encrypted CMAF bytes to {}",
        pipeline.written,
        out_path.display()
    );

    let log = transmux::segment_log::SegmentSummaryLog::new();
    let mpd = DashPackager::default().render(&pipeline.specs, &log)?;
    let cp_line = mpd
        .lines()
        .find(|l| l.contains("ContentProtection"))
        .unwrap_or("(none)");
    println!("\n--- DASH signalling ---\n{}", cp_line.trim());

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

/// The streaming demux → encrypt → CMAF chain plus the state the final
/// signalling needs (track specs and the track's protection).
#[cfg(feature = "cenc")]
struct Pipeline {
    encrypt: transmux::CencEncryptStage,
    mux: transmux::CmafMuxStage,
    output: std::fs::File,
    video: Option<transmux::TrackId>,
    specs: Vec<transmux::TrackSpec>,
    protection: Option<transmux::TrackProtection>,
    protection_track: Option<transmux::TrackId>,
    sample_count: u64,
    written: usize,
}

#[cfg(feature = "cenc")]
impl Pipeline {
    fn pump(
        &mut self,
        demux: &mut transmux::StreamingTsDemux,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use broadcast_common::Timestamp;
        use transmux::DemuxEvent;
        use transmux::mux_stage::MuxInput;
        use transmux::pipeline::CodecConfig;

        while let Some(event) = demux.poll_event() {
            match event {
                DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                    if matches!(spec.config, CodecConfig::Avc { .. }) {
                        self.video = Some(spec.track_id);
                        self.encrypt
                            .feed(MuxInput::Track(Box::new(spec)), Timestamp::ZERO)?;
                    }
                }
                DemuxEvent::Sample {
                    track_id, sample, ..
                } if self.video == Some(track_id) => {
                    self.sample_count += 1;
                    self.encrypt.feed(
                        MuxInput::Sample {
                            track: track_id,
                            sample,
                        },
                        Timestamp::ZERO,
                    )?;
                }
                _ => {}
            }
            self.drain_encrypt()?;
            self.drain_mux()?;
        }
        Ok(())
    }

    fn drain_encrypt(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        use broadcast_common::Timestamp;
        use transmux::mux_stage::MuxInput;

        while let Some(item) = self.encrypt.poll() {
            if let MuxInput::Track(spec) = &item
                && let Some(protection) = &spec.protection
            {
                self.protection = Some(protection.clone());
                self.protection_track = Some(spec.track_id);
                self.specs.push((**spec).clone());
            }
            self.mux.feed(item, Timestamp::ZERO)?;
            self.drain_mux()?;
        }
        Ok(())
    }

    fn drain_mux(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        use std::io::Write;
        use transmux::cenc_segment::protect_init_segment;
        use transmux::mux_stage::MuxOutput;

        while let Some(item) = self.mux.poll() {
            match item {
                MuxOutput::Init(bytes) => {
                    let (Some(protection), Some(track)) =
                        (self.protection.as_ref(), self.protection_track)
                    else {
                        continue;
                    };
                    let raw = protect_init_segment(&bytes, track, protection)?;
                    self.output.write_all(&raw)?;
                    self.written += raw.len();
                }
                MuxOutput::Bytes(bytes) => {
                    self.output.write_all(&bytes)?;
                    self.written += bytes.len();
                }
                MuxOutput::Segment(segment) => {
                    self.output.write_all(&segment.bytes)?;
                    self.written += segment.bytes.len();
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        use broadcast_common::stage::Stage;
        use std::io::Write;

        self.encrypt.finish()?;
        self.drain_encrypt()?;
        self.mux.finish()?;
        self.drain_mux()?;
        self.output.flush()?;
        Ok(())
    }
}
