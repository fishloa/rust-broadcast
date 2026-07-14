//! CENC/CBCS encrypt walkthrough — issue #564.
//!
//! Drives the whole encrypt path end to end through public APIs only:
//!
//! ```text
//! cleartext Media (TsDemux over fixtures/ts/h264/main.ts, AVC track)
//!   -> CencEncryptor::encrypt        (sets Track::encryption, ciphers samples)
//!   -> CmafMux::package              (unchanged CMAF muxer)
//!   -> protect_init_segment + protect_media_segment
//!                                    (splice in tenc/sinf + senc/saiz/saio)
//!   -> a standalone, encrypted CMAF file written to a temp path
//! ```
//!
//! and then shows the two signalling surfaces added in issue #564's final
//! task, both driven straight off the encrypted `Media`'s
//! [`transmux::TrackEncryption`]:
//!
//! - [`transmux::DashPackager`] auto-derives a `<ContentProtection
//!   schemeIdUri="urn:mpeg:dash:mp4protection:2011">` element from
//!   `Track::encryption` — no extra wiring needed.
//! - [`transmux::cenc_ext_x_key`] renders the HLS `#EXT-X-KEY` tag for the
//!   `cbcs` scheme (`cenc`/CTR has no valid HLS `METHOD` and is DASH-only).
//!
//! Run it with:
//!
//! ```text
//! cargo run -p transmux --example cenc_encrypt --features cenc
//! ```

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
    use std::path::PathBuf;

    use broadcast_common::{Encrypt, Package, Unpackage};
    use transmux::init_segment::protect_init_segment;
    use transmux::movie_fragment::{FragmentProtection, protect_media_segment};
    use transmux::pipeline::CodecConfig;
    use transmux::{
        CencEncryptor, CencScheme, CmafMux, DashPackager, EncryptConfig, IvGen, Media,
        SubsamplePolicy, TrackEncryption, TsDemux, cenc_ext_x_key,
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
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264/main.ts");
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
    let mut demux = TsDemux::new();
    let media: Media = demux.unpackage(&ts[..])?;
    let mut media = media.select_tracks_by(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))?;
    println!(
        "demuxed IR: {} track(s), {} sample(s)",
        media.tracks.len(),
        media.tracks[0].samples.len()
    );

    // 3. Encrypt in place: `cbcs` (AES-128-CBC, 1:9 pattern) — the CMAF-HLS
    //    case, so the HLS signalling step below has something to render.
    let cfg = EncryptConfig {
        scheme: CencScheme::Cbcs,
        kid: KID,
        key: KEY,
        iv: IvGen::Constant(CONSTANT_IV),
        pattern: Some((1, 9)),
        subsample: SubsamplePolicy::Video,
    };
    CencEncryptor.encrypt(&mut media, &cfg)?;
    let track_id = media.tracks[0].spec.track_id;
    let encryption: TrackEncryption = media.tracks[0]
        .encryption
        .as_ref()
        .expect("CencEncryptor populates Track::encryption")
        .clone();
    println!(
        "encrypted: scheme={} kid={}",
        encryption.scheme.name(),
        hex(&encryption.tenc.default_kid)
    );

    // 4. Mux the encrypted IR (unchanged CmafMux — encryption is orthogonal
    //    to container muxing) into one combined ftyp+moov+styp+moof+mdat
    //    buffer, then splice in the CENC/CBCS boxes.
    let raw = CmafMux::new(1).package(&media)?;
    let with_protected_init = protect_init_segment(&raw, track_id, &encryption)?;
    let fragment_protection = FragmentProtection {
        track_id,
        entries: &encryption.samples,
        per_sample_iv_size: encryption.tenc.default_per_sample_iv_size,
    };
    let protected = protect_media_segment(&with_protected_init, &[fragment_protection])?;

    let out_path = std::env::temp_dir().join("transmux_cenc_encrypt_example.mp4");
    std::fs::write(&out_path, &protected)?;
    println!(
        "wrote {} encrypted CMAF bytes to {}",
        protected.len(),
        out_path.display()
    );

    // 5. DASH signalling: `DashPackager` auto-derives the "common"
    //    ContentProtection from `Track::encryption` — no extra wiring.
    let mpd = DashPackager::default().package(&media)?;
    let cp_line = mpd
        .lines()
        .find(|l| l.contains("ContentProtection"))
        .unwrap_or("(none)");
    println!("\n--- DASH signalling ---\n{}", cp_line.trim());

    // 6. HLS signalling: `cenc_ext_x_key` renders `#EXT-X-KEY` for `cbcs`;
    //    it returns `None` for `cenc` (CTR has no valid HLS METHOD).
    let key_uri = "https://keyserver.example.com/key";
    match cenc_ext_x_key(encryption.scheme, &encryption.tenc.default_kid, key_uri) {
        Some(tag) => println!("--- HLS signalling ---\n{tag}"),
        None => println!(
            "--- HLS signalling ---\n{} has no HLS key tag (DASH-only)",
            encryption.scheme.name()
        ),
    }

    Ok(())
}

#[cfg(feature = "cenc")]
fn hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
