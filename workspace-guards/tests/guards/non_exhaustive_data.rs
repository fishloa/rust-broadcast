//! Per-crate data for the `non_exhaustive` guard. Add a line for a new crate.

use crate::non_exhaustive::Crate;

const fn plain(name: &'static str) -> Crate {
    Crate {
        name,
        skip: &[],
        variants: false,
        structs: false,
        struct_skip: &[],
        struct_extra: &[],
    }
}

/// A runtime crate whose `pub struct`s naming a config/policy/event/stats
/// value must be `#[non_exhaustive]` (they will grow fields, so a new field
/// must be an additive change). `skip` holds spec-fixed wire-layout structs
/// whose names coincidentally share a suffix; `extra` holds config/event value
/// structs whose names carry no suffix.
const fn runtime(
    name: &'static str,
    skip: &'static [&'static str],
    extra: &'static [&'static str],
) -> Crate {
    Crate {
        name,
        skip: &[],
        variants: false,
        structs: true,
        struct_skip: skip,
        struct_extra: extra,
    }
}

/// Members with no `src/lib.rs` (pure binaries: `dvb-tools`, `multimux-cli`).
/// (`dvb-stream` is a TABLE entry: its `ResyncStats` is a config/stats struct
/// covered by the struct rule, and it has no public enums.)
pub const EXEMPT: &[&str] = &[
    "dvb-tools",
    "multimux-cli",
    "test-alloc",
    "test-fixtures",
    "test-harness",
];

pub const TABLE: &[Crate] = &[
    plain("atsc3-route"),
    plain("atsc3"),
    // `RequestContext` is the sole non-suffix config/value struct; `Error` and
    // the challenge/method enums are covered by the enum rule.
    runtime("broadcast-auth", &[], &["RequestContext"]),
    // `tshttp::Config` (matches `*Config`), `PlayoutConfig` (no suffix) are
    // config value structs; `TsHttpPolicy` is the pull-policy engine (all
    // private fields, built via `TsHttpPolicy::new`), not a config value.
    // `SrtCallerConfig`/`UdpConfig` etc. already carry the attribute.
    runtime("broadcast-client", &["TsHttpPolicy"], &["PlayoutConfig"]),
    Crate {
        name: "broadcast-common",
        // `xml::Frame`: a private-to-the-module traversal stack that this
        // lexical scan reads as public because its line follows an
        // `impl<N: Namespace> Reader` line in the same file.
        skip: &["Frame"],
        variants: false,
        // `UdpOptions` already carries `#[non_exhaustive]`; no other
        // suffix-matching pub struct in this crate.
        structs: true,
        struct_skip: &[],
        struct_extra: &[],
    },
    plain("broadcast-dash"),
    // `StreamType` is the closed `StreamIndex@Type` token set (video/audio/text):
    // a caller matches all three and the packager converts between it and
    // `transmux`'s `SmoothStreamType` exhaustively, so a fourth token must be a
    // deliberate change, not a silent fall-through.
    Crate {
        name: "broadcast-smooth",
        skip: &["StreamType"],
        variants: false,
        structs: false,
        struct_skip: &[],
        struct_extra: &[],
    },
    // The HLS playlist model structs (`RenditionReport`, `SkipInfo`,
    // `PartSpec`, `LowLatencyConfig`) grow with RFC 8216bis revisions; they
    // are not a fixed wire layout. `MediaSegment`/`MasterPlaylist` etc. carry
    // no suffix and are also `#[non_exhaustive]` already.
    runtime("broadcast-hls", &[], &[]),
    plain("broadcast-loudness"),
    // `EventRecord` grows; the `*Config`/`Stats` structs already carry the
    // attribute. `Policy` is the target-follower engine (private fields,
    // built via `Policy::new`), not a config value struct.
    runtime("buffering-runtime", &["Policy"], &[]),
    plain("caption-convert"),
    plain("cc-data"),
    plain("compliance-probe"),
    Crate {
        name: "container-probe",
        skip: &[],
        variants: true,
        structs: false,
        struct_skip: &[],
        struct_extra: &[],
    },
    Crate {
        name: "dash-runtime",
        // `ResolvedRef` is a closed two-variant sum (absolute or relative);
        // callers match it exhaustively on purpose. `RequestMeta` is the ingest
        // request value struct (no suffix); the `*Config`/`Policy`/`VariantInfo`
        // structs already carry the attribute.
        skip: &["ResolvedRef"],
        variants: false,
        structs: true,
        struct_skip: &[],
        struct_extra: &["RequestMeta"],
    },
    plain("smooth-runtime"),
    plain("dvb-bbframe"),
    // `SlotInfo` matches the `Info` suffix rule and is a runtime state value
    // (not a fixed wire layout), so it gets the attribute like the rest.
    runtime("dvb-ci-runtime", &[], &[]),
    plain("dvb-ci"),
    plain("dvb-conformance"),
    plain("dvb-csa"),
    plain("dvb-mabr"),
    plain("dvb-si"),
    plain("dvb-simulcrypt"),
    plain("dvb-subtitle"),
    // `ResyncStats` is a stats struct (grows); the crate has no public enums.
    runtime("dvb-stream", &[], &[]),
    plain("dvb-t2mi"),
    plain("dvb-vbi"),
    // `VariantPolicy`/`VariantInfo`/`RenditionInfo` grow with master-playlist
    // attributes; `HlsClientConfig` already carries the attribute.
    runtime("hls-runtime", &[], &[]),
    // `Report` has a private `findings` field and is built via `Report::new`,
    // so it cannot be constructed by literal outside the crate; only
    // `MetricsServerConfig` (already attributed) and future all-pub structs
    // match the suffix rule meaningfully.
    runtime("media-doctor", &["Report"], &[]),
    plain("media-plane"),
    plain("mp4-emsg"),
    plain("mpeg-pes"),
    plain("mpeg-ps"),
    plain("mpeg-ts"),
    // Config/policy/spec structs grow with the routing/transport surface.
    // `FileReaderConfig`/`RouteConfig` already carry the attribute.
    runtime("multimux", &[], &["WhepAnswerParams"]),
    plain("playout-runtime"),
    plain("rdd29"),
    plain("rist-runtime"),
    plain("rmt-flute"),
    plain("rtmp-runtime"),
    plain("rtp-packet"),
    plain("rtsp-runtime"),
    plain("scte104"),
    plain("scte35-splice"),
    // `CryptoConfig` (feature `crypto`) and `NegotiatedParams` grow with the
    // handshake; `HandshakeConfig`/`IoConfig`/`SrtSocketConfig` already carry
    // the attribute.
    runtime("srt-runtime", &[], &[]),
    plain("ssai-runtime"),
    plain("st12-1"),
    plain("st2022"),
    plain("st291"),
    plain("st337"),
    plain("st377-1"),
    // `EmsgConfig` and `TimedEvent` are conversion/value structs that grow;
    // the DATERANGE/WebVTT models carry no suffix and are handled elsewhere.
    runtime("timed-metadata", &[], &["TimedEvent"]),
    // The codec configuration records (`*DecoderConfigurationRecord`,
    // `*SpsInfo`, `*SyncframeInfo`, `AudioSpecificConfig`, `VvcPtlRecord`,
    // `DtsCoreFrameInfo`, `AvcFieldInfo`, `NclxColourInfo`,
    // `DecoderSpecificInfo`) are spec-fixed wire layouts that share the
    // `Config`/`Info`/`Record` suffix; `PartInfo`/`SegmentInfo` are LL-HLS
    // output value structs that grow, and `TimelineJumpConfig`/`EncryptConfig`
    // are runtime config.
    runtime(
        "transmux",
        &[
            "AVCDecoderConfigurationRecord",
            "Ac3SyncframeInfo",
            "AudioSpecificConfig",
            "AvcFieldInfo",
            "AvcSpsInfo",
            "DecoderSpecificInfo",
            "DoviDecoderConfigurationRecord",
            "DtsCoreFrameInfo",
            "Ec3SyncframeInfo",
            "HEVCDecoderConfigurationRecord",
            "HevcSpsInfo",
            "MHADecoderConfigurationRecord",
            "NclxColourInfo",
            "VvcDecoderConfigurationRecord",
            "VvcPtlRecord",
            "VvcSpsInfo",
        ],
        &[],
    ),
    plain("ts-fix"),
    plain("ttml-subtitle"),
    plain("ule"),
    // `HttpRequest`/`HttpResponse`/`MediaTransportConfig` are the sans-IO
    // action/value types the caller builds; `WebRtcClientConfig` already
    // carries the attribute.
    runtime("webrtc-runtime", &[], &["HttpRequest", "HttpResponse"]),
];
