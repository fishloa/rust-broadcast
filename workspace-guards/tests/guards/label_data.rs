//! Per-crate data for the `label` guard (#204): each crate's SKIP list verbatim
//! with its reasons, the matcher/scope variant it was already using, and its
//! pinned set of spec/field enums where it had one. Add a line for a new crate.

use crate::label::{Crate, Matcher, Scope};

/// Members with no `src/lib.rs` (pure binaries: `dvb-tools`, `multimux-cli`) or no
/// public enums in `src/` (`dvb-stream`: async stream adapters + a stats struct;
/// `test-bounded`, `test-alloc`, `test-fixtures`: shared test helpers).
pub const EXEMPT: &[&str] = &[
    "dvb-tools",
    "multimux-cli",
    "dvb-stream",
    "test-bounded",
    "test-alloc",
    "test-fixtures",
];

pub const TABLE: &[Crate] = &[
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // `atsc3-route` defines three public spec/field label enums — [`Codepoint`],
    // [`FormatId`], [`FragMode`] (all in `codepoint.rs`) — each carrying
    // `name()` + `impl_spec_display!`.
    //
    // `Error` is skipped (structured error, no spec label). `RouteFecPayloadId`
    // (`fec.rs`) is also skipped: it is a **dispatch enum** selecting between
    // two disjoint wire layouts by an out-of-band bit (the packet's LCT SPI),
    // not itself a spec/field token — the same category as the workspace's
    // `Any*` tag-dispatch enums. See its module doc comment for the full
    // rationale.
    //
    // Checked **per file**, not over the whole crate concatenated, so a future
    // module reusing a variant/enum name cannot let one module's
    // `impl_spec_display!` silently satisfy the check for a different enum of
    // the same name declared elsewhere.
    //
    // [`Codepoint`]: atsc3_route::Codepoint
    // [`FormatId`]: atsc3_route::FormatId
    // [`FragMode`]: atsc3_route::FragMode
    Crate {
        name: "atsc3-route",
        scope: Scope::PerFile,
        matcher: Matcher::Loose,
        skip: &["Error", "RouteFecPayloadId"],
        expected: Some(&["Codepoint", "FormatId", "FragMode"]),
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written
    // `Display` impl.
    Crate {
        name: "atsc3",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        skip: &["Error"],
        expected: Some(&["LlsTableId", "ServiceCategory", "SlsProtocol"]),
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
    // impl. A present `Display` delegates to an inherent `name()`, so this one
    // check transitively enforces the whole convention.
    //
    // Skip list:
    // - `Error` — error enum, not a spec/field label per the convention.
    // - `Credentials` — a data-carrying ADT (each variant holds distinct
    // caller-supplied secrets: username/password/token), not a spec-defined
    // field enum with a fixed label set. It has no wire "value" to render as a
    // short label — `Display`-ing it risks reads-like-it-shows-a-secret
    // confusion, so it deliberately has no `Display` impl at all.
    // - `AuthResult` — a binary pass/fail verification outcome (`Ok`/
    // `Unauthorized`), not a spec-defined wire field with its own label set.
    Crate {
        name: "broadcast-auth",
        scope: Scope::Crate,
        matcher: Matcher::Strict,
        skip: &["Error", "Credentials", "AuthResult"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
    // impl. A present `Display` delegates to an inherent `name()`, so this one
    // check transitively enforces the whole convention.
    //
    // Skip list:
    // - `Error` (`client::error`) — structured `thiserror` error, not a spec/
    // field label.
    // - `TokioError` (`client::tokio_client`) — same: a structured IO-adapter
    // error.
    // - `Output` (`client::output`) — a data-carrying ADT (`Init(Vec<u8>)`,
    // `Samples { track_id, samples }`, …); callers match the typed variant,
    // a static label would be lossy.
    // - `Action` (`client::action`) — data-carrying dispatch ADT (`FetchPlaylist
    // { url, blocking, skip }`, …), same reasoning as `Output`.
    // - `ResourceId` (`client::action`) — data-carrying identity ADT (`Part {
    // msn, part }`, …), not a fixed spec label set.
    // - `HlsRequest` (`server::engine`) — data-carrying dispatch ADT
    // (`Playlist { track_id, query }`, `Resource { name }`), same reasoning
    // as `Action`/`ResourceId` (plan step 4).
    // - `HlsBody` (`server::engine`) — data-carrying outcome ADT
    // (`Playlist(String)`, `Resource(Bytes)`), same reasoning as `Output`
    // (plan step 4).
    // - `HlsOriginBuildError` (`server::engine`) — structured `thiserror` error
    // (issue #873's `HlsOriginBuilder::build`), not a spec/field label.
    // - `HlsMasterError` (`server::engine`) — structured `thiserror` error
    // (`HlsOrigin::master_playlist`), not a spec/field label.
    Crate {
        name: "broadcast-client",
        scope: Scope::Crate,
        matcher: Matcher::Strict,
        skip: &["Action", "Event", "Error", "AuthDecision"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
    // impl. A present `Display` delegates to an inherent `name()`, so this one
    // check transitively enforces the whole convention (issue #806).
    //
    // Skip list:
    // SKIP is empty: this crate's one public enum (`BitError`) already carries a
    // hand-written `Display` impl (a structured error message), which this
    // guard's `has_impl` check recognizes directly.
    Crate {
        name: "broadcast-common",
        scope: Scope::Crate,
        matcher: Matcher::Strict,
        skip: &[],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (errors only — all other public enums in broadcast-hls are
    // spec/field label enums), and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    // A present `Display` delegates to an inherent `name()`, so it transitively
    // guarantees `name()` exists — this one check enforces the whole convention
    // and catches a new `pub enum` that nobody labelled.
    Crate {
        name: "broadcast-hls",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        skip: &[
            "Error",
            "Define",
            "SessionDataContent",
            "InstreamId",
            "ClosedCaptionsRef",
        ],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (error / dispatch-wrapper enums that carry no spec label), and
    // fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    Crate {
        name: "broadcast-loudness",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels.
        skip: &["Error"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (error / dispatch-wrapper enums that carry no spec label), and
    // fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    Crate {
        name: "buffering-runtime",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels.
        skip: &["Error"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (error enums that carry no spec label), and fails if any
    // remaining enum has neither `broadcast_common::impl_spec_display!(Name)`
    // nor a hand-written `Display` impl.
    //
    // Because the project-wide `Display` impl delegates to an inherent
    // `name() -> &'static str`, a present `Display` transitively guarantees
    // `name()` exists (it would not compile otherwise) — so this single coverage
    // check enforces the whole convention and catches the one thing the compiler
    // cannot: a brand-new `pub enum` that nobody labelled.
    //
    // `caption-convert` defines three label enums, all in `matrix.rs`:
    // `SourceFormat`, `TargetFormat`, and `Support` (this crate's own
    // conversion-matrix vocabulary, not a wire spec, but labelled the same
    // way per project convention). `Error` is the documented skip (a
    // structured error type, no spec label). `MatrixEntry` is a struct, not an
    // enum, and is not scanned here.
    Crate {
        name: "caption-convert",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels: the structured
        // error type has no spec label.
        skip: &["Error"],
        expected: Some(&["SourceFormat", "Support", "TargetFormat"]),
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    //
    // Because the project-wide `Display` impl delegates to an inherent
    // `name() -> &'static str`, a present `Display` transitively guarantees
    // `name()` exists (it would not compile otherwise) — so this single coverage
    // check enforces the whole convention and catches the one thing the compiler
    // cannot: a brand-new `pub enum` that nobody labelled.
    //
    // Public enums in cc-data and their status:
    // - `CcType` — spec/field label → has `name()` + `impl_spec_display!`
    // - `Cea608Mode` — spec/field label → has `name()` + `impl_spec_display!`
    // - `Cea608Channel` — spec/field label → has `name()` + `impl_spec_display!`
    // - `Cea608Color` — spec/field label (CTA-608-E Tables 51/53) → has `name()` + `impl_spec_display!`
    // - `WindowState` — spec/field label → has `name()` + `impl_spec_display!`
    // - `AnchorPoint` — spec/field label (CTA-708-E §8.4.6) → has `name()` + `impl_spec_display!`
    // - `Opacity` — spec/field label → has `name()` + `impl_spec_display!`
    // - `EdgeType` — spec/field label → has `name()` + `impl_spec_display!`
    // - `PenSize` — spec/field label → has `name()` + `impl_spec_display!`
    // - `PenOffset` — spec/field label → has `name()` + `impl_spec_display!`
    // - `FontStyle` — spec/field label → has `name()` + `impl_spec_display!`
    // - `Justify` — spec/field label → has `name()` + `impl_spec_display!`
    // - `PrintDirection` — spec/field label → has `name()` + `impl_spec_display!`
    // - `ScrollDirection` — spec/field label → has `name()` + `impl_spec_display!`
    //
    // SKIP list:
    // - `Error` — structured error ADT, not a spec/field label.
    // - `WindowMapOp` — internal operation discriminant, not a public wire field.
    Crate {
        name: "cc-data",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels. Each is one of:
        // a structured error, an internal operation discriminant, or a data-carrying
        // ADT whose variants hold payloads.
        skip: &[
            // structured error
            "Error",
            // internal window-map operation discriminant — not pub, not a wire field
            "WindowMapOp",
        ],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // This crate has exactly one public enum today, [`compliance_probe::Scte35Sanity`]
    // (`src/scte35.rs`) — this crate's own future-vs-past cue classification,
    // not a wire-format token, but carrying `name()` + `impl_spec_display!` like
    // every other labelled enum in the workspace per convention. Copied
    // per-file (not per-crate-concatenated) from `webrtc-runtime`'s variant —
    // see that crate's `tests/label_coverage.rs` for why per-file scope is the
    // semantically correct one (`impl_spec_display!` resolves in the module it
    // is invoked from).
    Crate {
        name: "compliance-probe",
        scope: Scope::PerFile,
        matcher: Matcher::Loose,
        skip: &[],
        expected: Some(&["Scte35Sanity"]),
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
    // impl. A present `Display` delegates to an inherent `name()`, so this one
    // check transitively enforces the whole convention (issue #806).
    //
    // The SKIP list is empty: every `pub enum` here carries `name()` + a `Display`.
    // `Probe`, `Detail`, `IsobmffLayout`, `DocType` and `Format` all implement
    // `impl_spec_display!` (or, for the data-bearing `Detail`, a lossless
    // hand-written `Display` that delegates to `name()`); `Candidate` is a struct,
    // not an enum, so it never reaches this scan.
    Crate {
        name: "container-probe",
        scope: Scope::Crate,
        matcher: Matcher::Strict,
        skip: &[],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204): every
    // public enum has `impl_spec_display!` (delegating to a hand-written
    // `name()`) or a hand-written `Display`, unless it is in the skip list.
    //
    // Skip list (structured errors and data-carrying ADTs a static label would
    // make lossy):
    // - `MpdError`, `AddressError`, `PlanError`, `TimingError`, `DateTimeError`,
    // `InheritError`: `thiserror` errors, not spec labels.
    // - `OfferOutcome`: wraps an `AdoptReason` or a `RejectReason`, both labelled.
    // - `SegmentInfo`: data-carrying (`Template`/`List`/`Base`) resolution result.
    // - `client::Error`: `thiserror` error. `Action`, `ResourceId`, `Output`,
    // `Failure`, `FailureClass`: data-carrying protocol messages of the client,
    // whose payload a static label would drop (their label-bearing parts,
    // `FailureKind`, `Rejection`, `FatalKind`, `DropCause`, `Notice`, are covered).
    Crate {
        name: "dash-runtime",
        scope: Scope::Crate,
        matcher: Matcher::Dash,
        skip: &[
            "MpdError",
            "AddressError",
            "PlanError",
            "TimingError",
            "DateTimeError",
            "InheritError",
            "OfferOutcome",
            "SegmentInfo",
            "Error",
            "Action",
            "ResourceId",
            "Output",
            "Failure",
            "FailureClass",
        ],
        expected: None,
        min_enums: Some(15),
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (error / iterator / dispatch-wrapper enums that carry no spec
    // label), and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    //
    // Because the project-wide `Display` impl delegates to an inherent
    // `name() -> &'static str`, a present `Display` transitively guarantees
    // `name()` exists (it would not compile otherwise) — so this single coverage
    // check enforces the whole convention and catches the one thing the compiler
    // cannot: a brand-new `pub enum` that nobody labelled.
    Crate {
        name: "dvb-bbframe",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels: structured errors,
        // iterators, and dispatch/wrapper enums.
        skip: &["Error", "UpIter"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
    // impl. A present `Display` delegates to an inherent `name()`, so this one
    // check transitively enforces the whole convention (issue #806).
    //
    // Skip list:
    // - `CaError` (`managed`), `TransportError` (`transport`) — structured
    // `thiserror` errors, not spec/field labels.
    // - `DeviceOp`, `LinkEvent` (`device`), `Event`, `HostRequest`, `Action`,
    // `Notification`, `MmiEvent`, `HostControlEvent` (`event`) — data-carrying
    // dispatch/event ADTs (`Write(Vec<u8>)`, `Readable(&'a [u8])`,
    // `Tune { network_id, .. }`, …); callers match the typed variant, a
    // static label would be lossy.
    // - `TcState` (`transport`) — an internal transport-connection
    // state-machine discriminant (`Idle`/`Creating`/`Active`/…), not itself a
    // wire-format field.
    Crate {
        name: "dvb-ci-runtime",
        scope: Scope::Crate,
        matcher: Matcher::Strict,
        skip: &[
            "CaError",
            "TransportError",
            "DeviceOp",
            "LinkEvent",
            "Event",
            "HostRequest",
            "Action",
            "Notification",
            "MmiEvent",
            "HostControlEvent",
            "TcState",
        ],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
    // impl. A present `Display` delegates to an inherent `name()`, so this one
    // check transitively enforces the whole convention (issue #806).
    //
    // This guard checks **per file**, not over the whole crate concatenated.
    // `AckCode` is declared independently by both `ci_plus::file_retrieval` and
    // `ci_ext::application_mmi` (different variant sets), and `ActivationState`
    // is declared independently by both `ci_ext::power_manager` and
    // `ci_ext::status_query` — a whole-crate scan would let one module's
    // `impl_spec_display!` satisfy the check for both same-named enums, so
    // dropping either impl would go unnoticed. `impl_spec_display!(Name)`
    // resolves `Name` in the module it is invoked from, so per-file is also the
    // semantically correct scope.
    //
    // Skip list:
    // - `Error` (`error`) — structured `thiserror` error, not a spec/field label.
    // - `AnyApdu` (`any`) — the `declare_apdus!`-generated top dispatch enum
    // (this crate's `AnyDescriptor` analogue), same reasoning as dvb-si's own
    // `AnyDescriptor`/`AnyTableSection` SKIP entries.
    // - Every per-resource `*Apdu` enum (`ApplicationInfoV2Apdu`,
    // `ApplicationMmiApdu`, `BroadcastServiceGatewayApdu`, `CaPipelineApdu`,
    // `CaSupportApdu`, `CiExtApdu`, `CiPlusApdu`, `CicamPlayerApdu`,
    // `ContentControlApdu`, `CopyProtectionApdu`, `DownloadApdu`,
    // `EventManagerApdu`, `FileRetrievalApdu`, `LscV4Apdu`,
    // `MultistreamApdu`, `MultistreamHostControlApdu`, `PowerManagerApdu`,
    // `ResourceManagerV2Apdu`, `SampleDecryptionApdu`, `ServiceGatewayApdu`,
    // `StatusQueryApdu`, `StreamInputApdu`) — a per-resource dispatch/wrapper
    // ADT, each variant wrapping a distinct typed APDU struct (e.g.
    // `Request(CaPipelineRequest<'a>)`); same reasoning as `AnyApdu`.
    // - `ControlCommand`, `EmiData`, `CommsCmdParams`, `DisplayReplyBody`,
    // `SamplePayload` — data-carrying ADTs whose variants hold a differently
    // shaped payload (bitfields, nested structs, or byte vectors); a static
    // label would be lossy and add nothing over the typed variant.
    Crate {
        name: "dvb-ci",
        scope: Scope::PerFile,
        matcher: Matcher::Loose,
        skip: &[
            "Error",
            "AnyApdu",
            "ApplicationInfoV2Apdu",
            "ApplicationMmiApdu",
            "BroadcastServiceGatewayApdu",
            "CaPipelineApdu",
            "CaSupportApdu",
            "CiExtApdu",
            "CiPlusApdu",
            "CicamPlayerApdu",
            "ContentControlApdu",
            "CopyProtectionApdu",
            "DownloadApdu",
            "EventManagerApdu",
            "FileRetrievalApdu",
            "LscV4Apdu",
            "MultistreamApdu",
            "MultistreamHostControlApdu",
            "PowerManagerApdu",
            "ResourceManagerV2Apdu",
            "SampleDecryptionApdu",
            "ServiceGatewayApdu",
            "StatusQueryApdu",
            "StreamInputApdu",
            "ControlCommand",
            "EmiData",
            "CommsCmdParams",
            "DisplayReplyBody",
            "SamplePayload",
        ],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (errors + `Any*` dispatch wrappers), and fails if any remaining
    // enum has neither `broadcast_common::impl_spec_display!(Name)` nor a hand-written
    // `Display` impl. A present `Display` delegates to an inherent `name()`, so it
    // transitively guarantees `name()` exists — this one check enforces the whole
    // convention and catches a new `pub enum` that nobody labelled.
    Crate {
        name: "dvb-conformance",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        skip: &[],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // dvb-csa's one spec/field enum is `ts::KeyParity` (which of control word
    // (de)scrambles TS packet, ETSI TS 100 289 V1.1.1 §5.1 Table 1) — it has
    // `name()` + a hand-written `Display` (this crate does not depend on
    // `broadcast-common`, so `impl_spec_display!` isn't used). `Error` is
    // always skipped. This test ensures any future pub enum gets the same
    // treatment or is recorded in SKIP.
    Crate {
        name: "dvb-csa",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        skip: &["Error"],
        expected: Some(&["KeyParity"]),
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
    // impl. A present `Display` delegates to an inherent `name()`, so this one
    // check transitively enforces the whole convention (issue #806).
    //
    // Skip list:
    // - `Error` — structured `thiserror` error, not a spec/field label.
    Crate {
        name: "dvb-mabr",
        scope: Scope::Crate,
        matcher: Matcher::Strict,
        skip: &["Error"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    //
    // Because the project-wide `Display` impl delegates to an inherent
    // `name() -> &'static str`, a present `Display` transitively guarantees
    // `name()` exists (it would not compile otherwise) — so this single coverage
    // check enforces the whole convention and catches the one thing the compiler
    // cannot: a brand-new `pub enum` that nobody labelled.
    //
    // This guard checks **per file**, not over the whole crate concatenated.
    // `ReferenceType` is declared independently by both `tables::protection_message`
    // and `descriptors::announcement_support` (different variant sets), and
    // `LinkType` is declared independently by both `tables::rct` and
    // `descriptors::linkage` — a whole-crate scan would let one module's
    // `impl_spec_display!` satisfy the check for both same-named enums, so
    // dropping either impl would go unnoticed. `impl_spec_display!(Name)`
    // resolves `Name` in the module it is invoked from, so per-file is also the
    // semantically correct scope.
    Crate {
        name: "dvb-si",
        scope: Scope::PerFile,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels. Each is one of:
        // a structured error; a dispatch/registry wrapper or tag enum (the `declare_*`
        // system, `Any*`); a section-kind discriminant; or a data-carrying ADT whose
        // variants hold payloads (a static label would be lossy and add nothing — use
        // the typed variant instead).
        skip: &[
            // errors + dispatch wrappers / tag enums
            "Error",
            "AnyDescriptor",
            "AnyAitDescriptor",
            "AnyIntDescriptor",
            "AnyUntDescriptor",
            "AnyRntDescriptor",
            "GenericStreamSelector",
            "PhyStreamId",
            "AnyCarouselDescriptor",
            "AnyTableSection",
            "DescriptorTag",
            "TableId",
            "ExtensionTag",
            "SatTableId",
            // data-carrying / wrapper / body ADTs
            "BiopMessage",
            "CarouselObject",
            "TaggedProfile",
            "ObjectKind",
            "UnMessage",
            "CollectError",
            "CompletedEit",
            "SatBody",
            "ProtectionMessageBody",
            "FontInfo",
            "StereoscopicVideoView",
            "HevcTileSubstreamDetail",
            "DvbLocatorService",
            "DvbLocatorIdentifier",
            "ExtIterItem",
            "RegisteredExtension",
            "ExtensionBody",
            "TargetId",
            "LinkageData",
            "FormatSpecifier",
            "IdSelector",
            "CridLocation",
            "DepthRangeBody",
            "ImageIconBody",
            "IconLocation",
            "RegionCodes",
            "CellLinkage",
            "VbiService",
            "ShModulationMode",
            "ShInterleaver",
            "PositionSystem",
            "BeamhoppingMode",
            "DecoderConfig",
            // internal section-kind discriminants + config (not wire-field labels)
            "EitKind",
            "NitKind",
            "SdtKind",
            "PacketStride",
        ],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (error / dispatch-wrapper enums that carry no spec label), and
    // fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    //
    // Because the project-wide `Display` impl delegates to an inherent
    // `name() -> &'static str`, a present `Display` transitively guarantees
    // `name()` exists (it would not compile otherwise) — so this single coverage
    // check enforces the whole convention and catches the one thing the compiler
    // cannot: a brand-new `pub enum` that nobody labelled.
    Crate {
        name: "dvb-simulcrypt",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels: structured errors.
        skip: &["Error"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
    // impl. A present `Display` delegates to an inherent `name()`, so this one
    // check transitively enforces the whole convention (issue #806).
    //
    // Skip list:
    // - `Error` — structured `thiserror` error, not a spec/field label.
    // - `AnySegment` — the top dispatch enum over segment types (this crate's
    // `AnyDescriptor` analogue), same reasoning as dvb-si's own
    // `AnyDescriptor`/`AnyTableSection` SKIP entries.
    // - `ObjectDataPayload` (`segments::object_data`) — a data-carrying ADT
    // (`InterlacedPixels(..)`, `Characters { number_of_codes, .. }`); callers
    // match the typed variant, a static label would be lossy.
    Crate {
        name: "dvb-subtitle",
        scope: Scope::Crate,
        matcher: Matcher::Strict,
        skip: &["Error", "AnySegment", "ObjectDataPayload"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (error / dispatch-wrapper enums that carry no spec label), and
    // fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    //
    // Because the project-wide `Display` impl delegates to an inherent
    // `name() -> &'static str`, a present `Display` transitively guarantees
    // `name()` exists (it would not compile otherwise) — so this single coverage
    // check enforces the whole convention and catches the one thing the compiler
    // cannot: a brand-new `pub enum` that nobody labelled.
    Crate {
        name: "dvb-t2mi",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels: structured errors
        // and the `AnyPayload` dispatch wrapper.
        skip: &["Error", "AnyPayload"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (error / dispatch-wrapper enums that carry no spec label), and
    // fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    //
    // Because the project-wide `Display` impl delegates to an inherent
    // `name() -> &'static str`, a present `Display` transitively guarantees
    // `name()` exists (it would not compile otherwise) — so this single coverage
    // check enforces the whole convention and catches the one thing the compiler
    // cannot: a brand-new `pub enum` that nobody labelled.
    Crate {
        name: "dvb-vbi",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels:
        // - `Error` — structured error type, not a spec value label.
        // - `DataUnitPayload` — data-carrying dispatch ADT (carries typed payload
        // structs, not a flat spec value); not a spec/field label enum.
        skip: &["Error", "DataUnitPayload"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
    // impl. A present `Display` delegates to an inherent `name()`, so this one
    // check transitively enforces the whole convention.
    //
    // Skip list:
    // - `Error` (`client::error`) — structured `thiserror` error, not a spec/
    // field label.
    // - `TokioError` (`client::tokio_client`) — same: a structured IO-adapter
    // error.
    // - `Output` (`client::output`) — a data-carrying ADT (`Init(Vec<u8>)`,
    // `Samples { track_id, samples }`, …); callers match the typed variant,
    // a static label would be lossy.
    // - `Action` (`client::action`) — data-carrying dispatch ADT (`FetchPlaylist
    // { url, blocking, skip }`, …), same reasoning as `Output`.
    // - `ResourceId` (`client::action`) — data-carrying identity ADT (`Part {
    // msn, part }`, …), not a fixed spec label set.
    // - `HlsRequest` (`server::engine`) — data-carrying dispatch ADT
    // (`Playlist { track_id, query }`, `Resource { name }`), same reasoning
    // as `Action`/`ResourceId` (plan step 4).
    // - `HlsBody` (`server::engine`) — data-carrying outcome ADT
    // (`Playlist(String)`, `Resource(Bytes)`), same reasoning as `Output`
    // (plan step 4).
    // - `HlsOriginBuildError` (`server::engine`) — structured `thiserror` error
    // (issue #873's `HlsOriginBuilder::build`), not a spec/field label.
    // - `HlsMasterError` (`server::engine`) — structured `thiserror` error
    // (`HlsOrigin::master_playlist`), not a spec/field label.
    Crate {
        name: "hls-runtime",
        scope: Scope::Crate,
        matcher: Matcher::Strict,
        skip: &[
            "Error",
            "TokioError",
            "Output",
            "Action",
            "ResourceId",
            "HlsRequest",
            "HlsBody",
            "HlsOriginBuildError",
            "HlsMasterError",
        ],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    Crate {
        name: "media-doctor",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are not spec/field labels (errors, dispatch wrappers, CLI args).
        skip: &[
            "CheckArgs",          // clap argument struct
            "Cli",                // clap CLI dispatch enum
            "MulticastInterface", // config selector (which interface a group is joined on), not a spec/field label
        ],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
    // impl.
    //
    // # Why this crate's skip-list is almost everything
    //
    // The #204 convention exists for enums that decode an *externally-cited
    // spec's* coded field (a `table_id`, a `tag`, a `stream_type`) into a
    // stable, spec-grounded token. `media-plane` is not a wire-decode crate —
    // it is the ingress/byte-layer/Trunk/egress integration layer sitting
    // *above* the demuxers (`transmux`, `dvb-si`, …) that already do that
    // decoding. Its public enums are this crate's own control-flow, lifecycle,
    // and outcome types: `thiserror` errors, data-carrying ADTs (a poll result,
    // a driven-session event, a health state — each variant carries its own
    // payload, so a static label would be lossy and callers already match the
    // typed variant instead), and a handful of bare status/policy enums that
    // are this crate's *own* invented vocabulary, not a token from a cited
    // spec. [`crate::egress::CachePolicy`] is the one enum in this crate that
    // *does* carry the label pair (it doubles as an HTTP `Cache-Control` /
    // metrics token) and is deliberately not in this list.
    //
    // Because the project-wide `Display` impl delegates to an inherent
    // `name() -> &'static str`, a present `Display` transitively guarantees
    // `name()` exists (it would not compile otherwise) — so this single
    // coverage check enforces the whole convention and catches the one thing
    // the compiler cannot: a brand-new `pub enum` that nobody labelled or
    // triaged into this skip-list.
    Crate {
        name: "media-plane",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels — see the module
        // doc for the two categories ("error" / "data-carrying ADT") and the third,
        // crate-specific one ("bare internal status/policy enum with no cited-spec
        // token").
        skip: &[
            // --- byte_merge.rs ---
            // Behaviour policy ADT: `Failover` carries `primary`/`secondary`/
            // `silence_timeout` fields, so a static label would drop them. Not a
            // spec-cited token — this crate's own merge-policy vocabulary.
            "MergePolicy",
            // Structured `thiserror` error enum — no spec label.
            "MergeError",
            // --- byte_tap.rs ---
            // Bare enum, but purely descriptive metadata about *this crate's own*
            // pipeline position (wire vs. post-transform), not a token from any
            // cited spec.
            "TapPoint",
            // Data-carrying ADT: `Data(Bytes, Timestamp)` / `Lagged { skipped }`
            // carry a payload each.
            "TapItem",
            // --- egress.rs ---
            // (`CachePolicy` already has `name()` + `impl_spec_display!` — not
            // listed here.)
            // Data-carrying ADT, generic over the response body type `B`
            // (`Ready { body, cache }` / `Await { .. }` / `BadRequest { reason }`).
            "EgressResponse",
            // Data-carrying ADT, generic over the caller's error type `E`
            // (`Accepted(TrackSelection)` / `Refused { reason }` / `Error(E)`).
            "NegotiationOutcome",
            // --- ingress.rs ---
            // Data-carrying ADT (`NewProgram { program, tracks }` / `Sample { .. }`)
            // — see the type's own doc for why it deliberately carries no label.
            "SessionEvent",
            // Data-carrying ADT, generic over the session's error type `E`
            // (`Failed(E)` / `HandshakeTimedOut { deadline }`).
            "HealthState",
            // Data-carrying ADT, generic over both the session type `S` and error
            // type `E` (a dial attempt's outcome, carrying the session or error).
            "DialAttempt",
            // Data-carrying ADT, generic over the listener's error type `E`
            // (`Admitted(SessionId)` / `Error(E)`).
            "AcceptOutcome",
            // --- retention.rs ---
            // Bare enum (`Taken`/`Busy`), but a sink hand-off status this crate
            // invented, not a token from any cited spec.
            "SinkOutcome",
            // Data-carrying ADT: `Tiered` carries a `cold_window`/sink
            // configuration, not a bare label.
            "Retention",
            // Bare enum (`Hot`/`Cold`/`Evicted`), but this crate's own
            // catch-up-locate status, not a cited-spec token.
            "SegmentLocation",
            // --- trunk.rs ---
            // Bare enum (`Timed`/`Sparse`), but this crate's own retention-ring
            // classification, not a cited-spec token.
            "RetentionClass",
            // Bare enum (`Gap`/`StallIngest`/`Terminate`), but a caller-chosen
            // trade this crate's own DVR design invented, not a cited-spec token.
            "ArchiveOverrun",
            // Data-carrying ADT (the event log's absolute/segment-relative anchor
            // choice, each variant carrying its own timestamp/segment data).
            "EventAnchor",
            // Data-carrying ADT (`Timed { track_id, sample }` /
            // `Sparse { .. }` / `Lagged { skipped }`).
            "SampleCursorItem",
            // Data-carrying ADT (a finished segment's fields, or `Gap`/`Terminated`
            // with their own data).
            "SegmentCursorItem",
            // Data-carrying ADT (a timed-metadata event plus its anchor, or a
            // `Lagged`/gap variant with its own data).
            "EventCursorItem",
            // Structured `thiserror` error enum — no spec label.
            "TryPublishSegmentError",
        ],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (errors only — all other public enums in mp4-emsg are spec/field
    // label enums), and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    // A present `Display` delegates to an inherent `name()`, so it transitively
    // guarantees `name()` exists — this one check enforces the whole convention
    // and catches a new `pub enum` that nobody labelled.
    Crate {
        name: "mp4-emsg",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        skip: &["Error"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
    // impl. A present `Display` delegates to an inherent `name()`, so this one
    // check transitively enforces the whole convention (issue #806).
    //
    // Skip list:
    // - `Error` — structured `thiserror` error, not a spec/field label.
    // - `TrickMode` (`packet`) — each `trick_mode_control` value (ISO/IEC
    // 13818-1 §2.4.3.6) carries its own bitfield payload (`FastForward
    // { field_id, .. }`, `SlowMotion { .. }`, …); a static label would drop
    // those fields, matching dvb-si's `DecoderConfig`/`BeamhoppingMode`
    // precedent for bitfield-carrying spec enums.
    Crate {
        name: "mpeg-pes",
        scope: Scope::Crate,
        matcher: Matcher::Strict,
        skip: &["Error", "TrickMode"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
    // impl. A present `Display` delegates to an inherent `name()`, so this one
    // check transitively enforces the whole convention (issue #806).
    //
    // Skip list:
    // - `Error` — structured `thiserror` error, not a spec/field label.
    Crate {
        name: "mpeg-ps",
        scope: Scope::Crate,
        matcher: Matcher::Strict,
        skip: &["Error"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (error / dispatch-wrapper enums that carry no spec label), and
    // fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    //
    // Because the project-wide `Display` impl delegates to an inherent
    // `name() -> &'static str`, a present `Display` transitively guarantees
    // `name()` exists (it would not compile otherwise) — so this single coverage
    // check enforces the whole convention and catches the one thing the compiler
    // cannot: a brand-new `pub enum` that nobody labelled.
    Crate {
        name: "mpeg-ts",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels.
        //
        // - `Error` — structured error enum, not a spec/field label.
        //
        // # Coded `u8` fields vs. public enums
        //
        // This guard checks that every `pub enum` in `src/` has a `Display` impl
        // (via `broadcast_common::impl_spec_display!`). It does **not** check raw `u8`
        // fields that encode spec values without a typed enum accessor — those
        // require manual review.
        //
        // Audited coded-u8 fields in `mpeg-ts`:
        // - `TsHeader::scrambling` (2-bit) → typed accessor `scrambling_control()` →
        // `ScramblingControl` (covered by guard above)
        // - `TsHeader::continuity_counter` (4-bit) → counter, not a spec label, no enum needed
        // - `TsHeader::pid` (13-bit) → see `pid::Pid`; raw u16 on `TsHeader` by design
        skip: &["Error"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
    // impl. A present `Display` delegates to an inherent `name()`, so this one
    // check transitively enforces the whole convention (issue #806).
    //
    // This guard checks **per file**, not over the whole crate concatenated.
    // `StreamStatus` is declared independently by both `source::srt` and
    // `source::ts_http` (it is in SKIP below, so it's a latent risk rather than a
    // live gap, but the same hazard applies to any future same-named pair) — a
    // whole-crate scan would let one module's `impl_spec_display!` satisfy the
    // check for both same-named enums, so dropping either impl would go
    // unnoticed. `impl_spec_display!(Name)` resolves `Name` in the module it is
    // invoked from, so per-file is also the semantically correct scope.
    //
    // Skip list:
    // - `MultimuxError` (`error`) — structured `thiserror` error, not a spec/
    // field label.
    // - `FileReaderError` (`source::file_reader`) — structured `thiserror` error
    // for the file-reader task's read/probe/demux failures; not a spec/field
    // label.
    // - `RtmpPushError` (`push::rtmp`) — thiserror error for the RTMP push
    // transport's connect/protocol/IO failures; not a spec/field label.
    // - `RtspPushError` (`push::rtsp`) — thiserror error for the RTSP push
    // transport's connect/protocol/IO failures; not a spec/field label.
    // - `SendMediaError` (`push`) — thiserror error for
    // `PushTransport::send_media` (issue #934): mux failure vs. transport
    // send failure; not a spec/field label.
    // - `InputSpec`, `AuthSpec`, `OutputAuthSpec` (`config`) — data-carrying
    // config ADTs (`Rtsp { url, .. }`, `Password { .. }`, `Basic { .. }`, …);
    // callers match the typed variant, a static label would be lossy.
    // - `DashInput`, `FetchTarget` (`source::dash_pull`) — data-carrying
    // dispatch/identity ADTs (`Resource { id, body }`, `Utc(UtcId)`), same
    // reasoning as `InputSpec`.
    // - `HlsFetchId` (`source::hls_pull`) — data-carrying dispatch ADT
    // (`Resource(ResourceId)`), same reasoning.
    // - `SmoothResourceId`, `SmoothAction` (`source::smooth_pull`) — data-carrying
    // dispatch/identity ADTs (`Fragment(StreamIdx, u64)`, `FetchManifest { .. }`),
    // same reasoning.
    // - `StreamStatus` (`source::srt`, `source::ts_http`) — an internal
    // read-loop outcome discriminant (`Fed`/`Ended`), not itself a wire-format
    // field.
    // - `ReconnectState` (`push`) — the push reconnect FSM's state discriminant
    // (`Ready`/`Backoff`/`Failed`), an internal driver enum whose three states
    // carry no wire token; a static label would add nothing.
    // - `Budget` (`origin::limit`) — the request-concurrency budget
    // classification (`Exempt`/`BlockingReload`/`Ordinary`); its `label()` is a
    // Prometheus `kind` label, not a spec/field wire token, and it is
    // `#[doc(hidden)]` (an implementation detail of the origin's router).
    Crate {
        name: "multimux",
        scope: Scope::PerFile,
        matcher: Matcher::Loose,
        skip: &[
            "MultimuxError",
            "IngestError",
            "SessionError",
            "AddSegmentError",
            "FileReaderError",
            "RtmpPushError",
            "RtspPushError",
            "SendMediaError",
            "InputSpec",
            "AuthSpec",
            "OutputAuthSpec",
            "DashInput",
            "FetchTarget",
            "HlsFetchId",
            "SmoothResourceId",
            "SmoothAction",
            "StreamStatus",
            "ReconnectState",
            "Budget",
        ],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // `Error` is the crate's error enum (data-carrying variants, not a spec
    // label) — SKIPped, same convention `ssai-runtime`'s own
    // `tests/label_coverage.rs` documents for its `Error`.
    //
    // `EntryKind` (`schedule.rs`) and `BreakEdge` (`scte35.rs`) ARE this
    // crate's own field/token enums (no external wire spec backs them — issue
    // #748's design decision is that the schedule format is ours to define —
    // but the #204 convention applies uniformly to every public token enum
    // regardless of whether an external spec assigns the token) and carry
    // `name()` + `impl_spec_display!` like any other.
    Crate {
        name: "playout-runtime",
        scope: Scope::PerFile,
        matcher: Matcher::Loose,
        skip: &["Error"],
        expected: Some(&["BreakEdge", "EntryKind"]),
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written
    // `Display` impl.
    //
    // `rdd29` has eight spec/field enums, each carrying `name()` +
    // `impl_spec_display!(...)`: [`rdd29::ElementId`] (Table 1),
    // [`rdd29::SampleRate`] (Table 2), [`rdd29::BitDepth`] (Table 3),
    // [`rdd29::FrameRate`] (Table 4), [`rdd29::ChannelId`] (Table 6),
    // [`rdd29::ZoneGain`] (Table 9), [`rdd29::ZoneId`] (Table 8 — an index
    // label, not itself a wire field, but still spec-cited and labelled),
    // [`rdd29::ObjectSpreadMode`] (Table 10), and [`rdd29::DecorCoefPrefix`]
    // (Table 11). [`rdd29::AnyElement`] is a tag-dispatch enum (like
    // `dvb_si::AnyDescriptor`), not a spec/field label, and is skipped.
    Crate {
        name: "rdd29",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels: the structured
        // error type has no spec label, and `AnyElement` is a tag-dispatch enum
        // over the three concrete element types (see this crate's module docs).
        skip: &["Error", "AnyElement"],
        expected: Some(&[
            "BitDepth",
            "ChannelId",
            "DecorCoefPrefix",
            "ElementId",
            "FrameRate",
            "ObjectSpreadMode",
            "SampleRate",
            "ZoneGain",
            "ZoneId",
        ]),
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written
    // `Display` impl.
    //
    // `rist-runtime` has two spec/field enums: [`rist_runtime::RttEchoKind`]
    // (TR-06-1 §5.2.6) and [`rist_runtime::ReportPart`] (the SR/RR that opens a
    // compound, RFC 3550 §6.1). [`rist_runtime::Error`] is a structured error
    // type and is skipped.
    Crate {
        name: "rist-runtime",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        skip: &["Error"],
        expected: Some(&["ReportPart", "RttEchoKind"]),
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    //
    // Because the project-wide `Display` impl delegates to an inherent
    // `name() -> &'static str`, a present `Display` transitively guarantees
    // `name()` exists (it would not compile otherwise) — so this single coverage
    // check enforces the whole convention and catches the one thing the compiler
    // cannot: a brand-new `pub enum` that nobody labelled.
    Crate {
        name: "rmt-flute",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels.
        skip: &[
            // structured error type
            "Error",
            // data-carrying ADT (per-sub-type NORM_CMD fixed body), not a spec/field
            // label — `NormCmdType` (its discriminant) already carries the label.
            "NormCmdBody",
        ],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (error / dispatch-wrapper enums that carry no spec label), and
    // fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    //
    // Because the project-wide `Display` impl delegates to an inherent
    // `name() -> &'static str`, a present `Display` transitively guarantees
    // `name()` exists (it would not compile otherwise) — so this single coverage
    // check enforces the whole convention and catches the one thing the compiler
    // cannot: a brand-new `pub enum` that nobody labelled.
    //
    // `rtcp-packet` defines three public spec/field enums today —
    // `RtcpPacketType`, `SdesItemType`, and the `RtcpPacket` dispatch enum —
    // all three already carry `impl_spec_display!` (carried over unchanged from
    // the pre-extraction `transmux::rtcp` implementation).
    Crate {
        name: "rtcp-packet",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels: the structured
        // error type has no spec label, and any future dispatch wrappers belong here.
        skip: &["Error"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (error / data-carrying ADT / wire-variant enums that carry no
    // spec label), and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
    // impl.
    //
    // Because the project-wide `Display` impl delegates to an inherent
    // `name() -> &'static str`, a present `Display` transitively guarantees
    // `name()` exists (it would not compile otherwise) — so this single coverage
    // check enforces the whole convention and catches the one thing the compiler
    // cannot: a brand-new `pub enum` that nobody labelled.
    Crate {
        name: "rtmp-runtime",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels.
        skip: &[
            // Structured `thiserror` error enum — no spec label.
            "RtmpError",
            // Structured `thiserror` URL-rejection enum for `target::RtmpTarget` — no spec label.
            "RtmpUrlError",
            // Data-carrying ADT (§8.2 AMF0 value types): each variant holds its own
            // payload (Number(f64), Utf8String<'a>, Object(..), ...), so a static
            // label would be lossy — callers match the typed variant instead. See
            // the doc comment on `Amf0Value` in amf0.rs for the full rationale.
            "Amf0Value",
            // Wire-variant ADT (§5.3.1.2 Chunk Message Header Type 0/1/2/3): each
            // variant holds a different set of resolved header fields, not a bare
            // spec label — `Fmt` (the 2-bit format selector) is the labelled
            // counterpart already carrying `name()`/`impl_spec_display!`.
            "MessageHeader",
            // Data-carrying event ADT `ServerSession::handle_data` surfaces to the
            // caller (Connected{app}/Publish{..}/Media{flv}/Eof) — not a spec label.
            "ServerEvent",
            // Data-carrying event ADT `ClientSession::handle_data` surfaces to the
            // caller (Connected/StreamCreated/Publishing/Error/Closed) — not a spec label.
            "ClientEvent",
            // Internal client state machine discriminant, not a spec label.
            "ClientState",
        ],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (error / dispatch-wrapper enums that carry no spec label), and
    // fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    //
    // Because the project-wide `Display` impl delegates to an inherent
    // `name() -> &'static str`, a present `Display` transitively guarantees
    // `name()` exists (it would not compile otherwise) — so this single coverage
    // check enforces the whole convention and catches the one thing the compiler
    // cannot: a brand-new `pub enum` that nobody labelled.
    //
    // `rtp-packet`'s RFC 3550 core defines no spec/field enums (the wire format
    // is all struct fields — see `RtpPacket`/`HeaderExtension`). The optional
    // `rfc8285` feature adds one `pub enum`, `ExtensionElements` — a
    // data-carrying `profile_id` dispatch wrapper (one-byte vs. two-byte form),
    // in the same spirit as this workspace's `AnyTableSection`/`AnyDescriptor`
    // dispatch enums, not a spec/field label — so it is on the SKIP list below.
    // This test scans `src/` as plain text, so it covers `rfc8285.rs` even when
    // the feature is off.
    Crate {
        name: "rtp-packet",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels: the structured
        // error type has no spec label, and dispatch wrappers (like the `rfc8285`
        // feature's `ExtensionElements`, which dispatches on `profile_id` between
        // the RFC 8285 one-byte/two-byte forms) belong here.
        skip: &["Error", "ExtensionElements"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204), local
    // variant.
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has no `Display` impl. A present
    // `Display` here delegates to an inherent `name() -> &'static str` (it would
    // not compile otherwise), so this single check enforces the whole convention.
    //
    // `rtsp-runtime` is a standalone RTSP crate with no DVB dependency, so it does
    // **not** pull `broadcast_common::impl_spec_display!`; the spec/field enums
    // (`SessionState`, `LowerTransport`, `Delivery`) carry hand-written `name()` +
    // `Display` instead. This guard accepts either form.
    Crate {
        name: "rtsp-runtime",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels: the structured error
        // and the two data-carrying event ADTs whose variants hold payloads (a static
        // label would be lossy — callers match the typed variant instead).
        skip: &[
            "Error",       // structured thiserror error
            "ClientEvent", // data-carrying ADT (Response/AuthRetry/MediaData)
            "ServerEvent", // data-carrying ADT (RequestAccepted/MethodNotValid/SessionSetup)
        ],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
    // impl. A present `Display` delegates to an inherent `name()`, so this one
    // check transitively enforces the whole convention (issue #806).
    //
    // Skip list:
    // - `Error` — structured `thiserror` error, not a spec/field label.
    // - `AnyOperation`, `AnySingleOperation` (`operations`) — the
    // `declare_operations!`-generated dispatch enums (this crate's
    // `AnyDescriptor` analogue), same reasoning as dvb-si's own
    // `AnyDescriptor`/`AnyTableSection` SKIP entries.
    // - `Timestamp` (`time`) — a data-carrying ADT (`Utc(UtcTimestamp)`,
    // `SmpteVitc(SmpteVitcTimestamp)`, …); callers match the typed variant, a
    // static label would be lossy.
    Crate {
        name: "scte104",
        scope: Scope::Crate,
        matcher: Matcher::Strict,
        skip: &["Error", "AnyOperation", "AnySingleOperation", "Timestamp"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (errors + `Any*` dispatch wrappers), and fails if any remaining
    // enum has neither `broadcast_common::impl_spec_display!(Name)` nor a hand-written
    // `Display` impl. A present `Display` delegates to an inherent `name()`, so it
    // transitively guarantees `name()` exists — this one check enforces the whole
    // convention and catches a new `pub enum` that nobody labelled.
    Crate {
        name: "scte35-splice",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        skip: &[
            "Error",
            "AnySpliceDescriptor",
            "AnyCommand",
            // DVB-TA (TS 103 752-1) carriage/dispatch ADTs, not spec/field-label enums:
            // they select between sub-structures rather than labelling a coded byte.
            "CompactScte35",
            "Scte35Carriage",
        ],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has no `Display` impl. Every
    // non-skipped enum here gets its `Display` from
    // `broadcast_common::impl_spec_display!`, which requires an inherent
    // `name()` — so a present `Display` implies the whole convention is
    // satisfied.
    Crate {
        name: "srt-runtime",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels:
        skip: &[
            "Error",           // structured thiserror error
            "SrtPacket",       // data-carrying ADT (Data/Control payload)
            "ControlPacket",   // data-carrying ADT (one variant per control packet struct)
            "AckCif",          // data-carrying ADT (Full/Small/Light field sets)
            "LossListEntry",   // data-carrying ADT (Single/Range sequence numbers)
            "HandshakeOutput", // data-carrying ADT (handshake SM event, not a wire label)
            "KmRefreshEvent",  // data-carrying ADT (KM refresh driver event, not a wire label)
        ],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // `AssetSource` is a data-carrying dispatch enum (its variants hold a
    // `String` URI, not an enumerated wire token) — the same shape as
    // `timed-metadata::event::SourcePayload`, which the project also treats as
    // not a label. It is SKIPped for that reason, documented here rather than
    // rediscovered.
    //
    // `SnapMode`, `RestrictMode` (both `decision.rs`) and `SnapDirection`
    // (`splice.rs`) ARE spec/field-style token enums (`X-SNAP`/`X-RESTRICT`
    // wire tokens, and this crate's own splice-conditioning result
    // classification) and carry `name()` + `impl_spec_display!` like any other.
    Crate {
        name: "ssai-runtime",
        scope: Scope::PerFile,
        matcher: Matcher::Loose,
        skip: &["Error", "AssetSource"],
        expected: Some(&["SnapMode", "RestrictMode", "SnapDirection"]),
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (error / dispatch-wrapper enums that carry no spec label), and
    // fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    //
    // `st12-1` defines two public spec/field enums today: [`st12_1::FrameRate`]
    // (ST 12-1 Table 3's three flag-bit-position columns) and
    // [`st12_1::BinaryGroupUsage`] (Table 1's binary-group-usage
    // classification) — both carry `name()` + `impl_spec_display!`.
    Crate {
        name: "st12-1",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels: the structured
        // error type has no spec label.
        skip: &["Error"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written
    // `Display` impl.
    Crate {
        name: "st2022",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        skip: &["Error", "ExtensionItem"],
        expected: Some(&[
            "ClockFrequency",
            "FecUsage",
            "FrameRate",
            "FrameStructure",
            "MapStructure",
            "SampleStructure",
            "Scrambling",
            "TimestampRef",
            "VideoSourceId",
        ]),
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (error / dispatch-wrapper enums that carry no spec label), and
    // fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    //
    // Because the project-wide `Display` impl delegates to an inherent
    // `name() -> &'static str`, a present `Display` transitively guarantees
    // `name()` exists (it would not compile otherwise) — so this single coverage
    // check enforces the whole convention and catches the one thing the compiler
    // cannot: a brand-new `pub enum` that nobody labelled.
    Crate {
        name: "st291",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels: the structured
        // error type has no spec label, and any future dispatch wrappers belong here.
        skip: &["Error"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (error enums that carry no spec label), and fails if any
    // remaining enum has neither `broadcast_common::impl_spec_display!(Name)`
    // nor a hand-written `Display` impl.
    //
    // Because the project-wide `Display` impl delegates to an inherent
    // `name() -> &'static str`, a present `Display` transitively guarantees
    // `name()` exists (it would not compile otherwise) — so this single coverage
    // check enforces the whole convention and catches the one thing the compiler
    // cannot: a brand-new `pub enum` that nobody labelled.
    //
    // `st337` defines exactly one spec/field `pub enum`, [`st337::DataMode`]
    // (SMPTE ST 337 §7.2.4.3 Table 8), which carries `name()` +
    // `impl_spec_display!(DataMode)`. `data_type` is deliberately **not** an
    // enum (see `docs/st337.md` scope decision 3 — the type -> codec mapping is
    // registered in SMPTE ST 338, not independently verified here), so this
    // test also guards against a future contributor introducing an unlabelled
    // `data_type` enum without reading that decision first.
    Crate {
        name: "st337",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels: the structured
        // error type has no spec label.
        skip: &["Error"],
        expected: Some(&["DataMode"]),
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (error enums that carry no spec label), and fails if any
    // remaining enum has neither `broadcast_common::impl_spec_display!(Name)`
    // nor a hand-written `Display` impl.
    //
    // `st377-1` defines five public spec/field enums today:
    // [`st377_1::PartitionKind`], [`st377_1::PartitionStatus`] (Table 4),
    // [`st377_1::ItemLengthMode`] (§9.3 Note 1), [`st377_1::ReleaseType`]
    // (§4.3's `ProductVersion.release` enumeration), and
    // [`st377_1::StructuralSetKind`] (Table 17 — hand-written `Display` since
    // its `Unknown` catch-all carries a 2-byte payload the
    // `impl_spec_display!` macro's single-byte-payload form doesn't fit).
    // [`st377_1::BerLength`] (issue #1047) is a sixth public enum but is on
    // `SKIP` below — see its comment.
    Crate {
        name: "st377-1",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels: the structured
        // error type has no spec label. `BerLength` (issue #1047) is a
        // serialization-*form* control (a data-carrying ADT — one variant carries
        // the on-wire width), not a spec-defined named vocabulary.
        skip: &["Error", "BerLength"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    // A present `Display` delegates to an inherent `name()`, so it transitively
    // guarantees `name()` exists — this one check enforces the whole convention
    // and catches a new `pub enum` that nobody labelled.
    //
    // SKIP list rationale:
    // - `Error` — thiserror error enum, not a spec/field label enum.
    // - `SourcePayload` — data-carrying ADT (holds typed payload values, not a
    // spec field code), analogous to `Any*` enums in other crates.
    Crate {
        name: "timed-metadata",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        skip: &[
            "Error",         // thiserror error enum — not a spec/field label
            "SourcePayload", // data-carrying ADT — not a spec/field label
        ],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204),
    // introduced in `transmux` by issue #580.
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    //
    // Because the project-wide `Display` impl delegates to an inherent
    // `name() -> &'static str`, a present `Display` transitively guarantees
    // `name()` exists (it would not compile otherwise) — so this single coverage
    // check enforces the whole convention and catches the one thing the compiler
    // cannot: a brand-new `pub enum` that nobody labelled.
    Crate {
        name: "transmux",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels. Each is one of:
        // a structured error; a dispatch/tag enum whose variants wrap full
        // structured payloads (not a flat spec code); a `clap` CLI argument enum; or
        // a data-carrying ADT whose variants hold arbitrary payloads (a static label
        // would be lossy and add nothing — use the typed variant instead).
        skip: &[
            // `slice_header::AvcPicture` (issue #1161): a data-carrying classification
            // ADT (`Field(AvcFieldInfo)`), not a spec-token label.
            "AvcPicture",
            // `display_track::ActivationOutcome` (display-geometry Phase 2): what one
            // access unit did to the tracker (a data-carrying ADT, `Changed { .. }`);
            // not a spec code and never rendered to a user.
            "ActivationOutcome",
            // `ts_hls::UpdateTrackOutcome` (issue #1425): a data-carrying ADT for
            // the result of `update_track` (Changed/Unchanged), not a spec code.
            "UpdateTrackOutcome",
            // `mpeg2_display::Mpeg2Outcome` (display-geometry Phase 3b): the same
            // shape for MPEG-2 (a data-carrying ADT), not a spec code.
            "Mpeg2Outcome",
            // `display_track::TrackCodec` (display-geometry Phase 2): which
            // parameter-set syntax a demuxer's tracker follows (AVC / HEVC), a
            // routing choice, not a wire label.
            "TrackCodec",
            // `live_join::JoinDecision`/`JoinState` (issue #1162 R4): data-carrying
            // state ADTs (`Drop(DropReason)`, `JoinPoint(RapKind)`, `Joined { .. }`),
            // not spec-token labels; their leaf enums `DropReason`/`RapKind` carry
            // `name()` + `impl_spec_display!`.
            "JoinDecision",
            "JoinState",
            // `ir::SubtitleSampleEntry` (issue #1186): a data-carrying ADT holding the
            // whole parsed `stpp`/`wvtt` sample entry; its format label is
            // `SubtitleFormat` (which carries `name()` + `impl_spec_display!`).
            "SubtitleSampleEntry",
            // `subtitle_entries::WvttSampleBox`: a data-carrying ADT of the boxes of a
            // `wvtt` media sample (`vttc` / `vtte` / `vtta` / other), not a spec label.
            "WvttSampleBox",
            // errors (Display comes from `thiserror`'s derive or a hand-written
            // human-readable message, not a spec-token label)
            "Error",
            "FlvError",
            "RtmpError",
            "CliError",
            // `dash_parse::DashParseError` (issue #758 T1): a structured parse error,
            // labelled via a hand-written `Display` impl (matching `FlvError`), not
            // the spec-token convention — already satisfied by that `Display` impl,
            // listed here for the same documentation reason `FlvError`/`RtmpError` are.
            "DashParseError",
            // `smooth_parse::SmoothParseError` (issue #759 T1): a structured parse
            // error, labelled via a hand-written `Display` impl mirroring
            // `DashParseError`, not the spec-token convention — already satisfied by
            // that `Display` impl, listed here for the same documentation reason.
            "SmoothParseError",
            // clap CLI argument enum (labels are the `--format` strings, owned by
            // `clap::ValueEnum`, not this convention)
            "FormatArg",
            // data-carrying ADTs: every variant wraps a distinct structured payload
            // (a full sample-entry/box/config/value type), so a flat label would be
            // lossy — the typed variant *is* the label.
            "CodecConfig",
            "DemuxEvent",
            // `rtp_stream::RtpLossEvent` (issue #779): a data-carrying ADT, same
            // shape as `DemuxEvent` above — each variant wraps distinct structured
            // fields (track/ssrc/sequence numbers), not a flat spec-defined code.
            "RtpLossEvent",
            // `rtp::RtpTimingWarning` (audit r04-W29): the same shape — a variant
            // carries the stream index and the two timestamps that disagreed, which a
            // flat label could not express.
            "RtpTimingWarning",
            // `LlHlsSegmenter`'s `Stage::Out` (media plane step 2e-2): a dispatch
            // enum wrapping two full structured payloads (`PartInfo`/`SegmentInfo`),
            // same shape as `DemuxEvent` above — not a spec-defined flat code.
            "LlHlsStageOutput",
            "Output",
            "SampleEntryVariant",
            "StblChild",
            "ProtocolControl",
            "AmfValue",
            "SgpdEntry",
            // `cenc_encrypt` (#564) config enums: `IvGen` carries either a scalar
            // counter base or a full caller-supplied IV list (data-carrying ADT, like
            // `CodecConfig` above); `SubsamplePolicy` is a plain behaviour switch, not
            // a spec-defined wire code, so a `name()`/`Display` label would be
            // fabricated rather than transcribed from a spec table.
            "IvGen",
            "SubsamplePolicy",
            // `movie_fragment::TrafChild`/`MoofChild` (audit r05-W11): dispatch
            // enums the parser records wire order into — each variant either names a
            // typed child that lives on the owning box or wraps an `OpaqueChild`, so
            // a flat spec-token label would be lossy and add nothing (the same reason
            // `StblChild` above is skipped).
            "TrafChild",
            "MoofChild",
            // `sample_entries::SampleEntryChild` (audit item 3, round 3): the same
            // shape again — either an opaque child's full bytes, or a marker for the
            // codec config box at its wire position.
            "SampleEntryChild",
        ],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
    // impl. A present `Display` delegates to an inherent `name()`, so this one
    // check transitively enforces the whole convention (issue #806).
    //
    // Skip list:
    // - `Error` — structured `thiserror` error, not a spec/field label.
    // - `PcrRestamp` (`ops::pcr_restamp`), `PidFilter` (`ops::pid_filter`),
    // `Stuffing` (`ops::stuffing`) — data-carrying operation-config ADTs
    // (`FromBitrate { bitrate }`, `Keep { pids }`, `PadTo { rate }`, …)
    // selecting which conditioning op runs and its parameters; not a
    // spec/field value decoded from the wire, so a static label would add
    // nothing over the typed variant.
    Crate {
        name: "ts-fix",
        scope: Scope::Crate,
        matcher: Matcher::Strict,
        skip: &["Error", "PcrRestamp", "PidFilter", "Stuffing"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
    // impl. A present `Display` delegates to an inherent `name()`, so this one
    // check transitively enforces the whole convention.
    //
    // Skip list:
    // - `Error` (`client::error`) — structured `thiserror` error, not a spec/
    // field label.
    // - `TokioError` (`client::tokio_client`) — same: a structured IO-adapter
    // error.
    // - `Output` (`client::output`) — a data-carrying ADT (`Init(Vec<u8>)`,
    // `Samples { track_id, samples }`, …); callers match the typed variant,
    // a static label would be lossy.
    // - `Action` (`client::action`) — data-carrying dispatch ADT (`FetchPlaylist
    // { url, blocking, skip }`, …), same reasoning as `Output`.
    // - `ResourceId` (`client::action`) — data-carrying identity ADT (`Part {
    // msn, part }`, …), not a fixed spec label set.
    // - `HlsRequest` (`server::engine`) — data-carrying dispatch ADT
    // (`Playlist { track_id, query }`, `Resource { name }`), same reasoning
    // as `Action`/`ResourceId` (plan step 4).
    // - `HlsBody` (`server::engine`) — data-carrying outcome ADT
    // (`Playlist(String)`, `Resource(Bytes)`), same reasoning as `Output`
    // (plan step 4).
    // - `HlsOriginBuildError` (`server::engine`) — structured `thiserror` error
    // (issue #873's `HlsOriginBuilder::build`), not a spec/field label.
    // - `HlsMasterError` (`server::engine`) — structured `thiserror` error
    // (`HlsOrigin::master_playlist`), not a spec/field label.
    Crate {
        name: "tshttp-runtime",
        scope: Scope::Crate,
        matcher: Matcher::Strict,
        skip: &["Action", "Event"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list, and fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
    // impl. A present `Display` delegates to an inherent `name()`, so this one
    // check transitively enforces the whole convention (issue #806).
    //
    // Skip list:
    // - `Error` — structured `thiserror` error, not a spec/field label.
    // - `InlineContent` — a data-carrying ADT with typed variants (Text, Span, Br);
    // callers match the typed variant; a static label would be lossy.
    // - `AnimationChild` — a data-carrying ADT with typed variants.
    // - `MetadataChild` — a data-carrying ADT with typed variants.
    // - `TimeExpression` — a data-carrying ADT with different time forms;
    // callers match the variant; a static label would be lossy.
    // - `WallclockForm` — a data-carrying ADT with typed variants.
    // - `UnknownNode` — a data-carrying ADT (element child vs text node) inside
    // an [`UnknownElement`] subtree; callers match the variant; a static label
    // would be lossy (#1110/TT-W1).
    Crate {
        name: "ttml-subtitle",
        scope: Scope::Crate,
        matcher: Matcher::Strict,
        skip: &[
            "Error",
            "InlineContent",
            "AnimationChild",
            "MetadataChild",
            "TimeExpression",
            "WallclockForm",
            "UnknownNode",
        ],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // Scans this crate's `src/` for every `pub enum`, subtracts a documented
    // skip-list (error / dispatch-wrapper enums that carry no spec label), and
    // fails if any remaining enum has neither
    // `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
    //
    // Because the project-wide `Display` impl delegates to an inherent
    // `name() -> &'static str`, a present `Display` transitively guarantees
    // `name()` exists (it would not compile otherwise) — so this single coverage
    // check enforces the whole convention and catches the one thing the compiler
    // cannot: a brand-new `pub enum` that nobody labelled.
    Crate {
        name: "ule",
        scope: Scope::Crate,
        matcher: Matcher::Loose,
        // Enums that are intentionally **not** spec/field labels: structured errors.
        skip: &["Error"],
        expected: None,
        min_enums: None,
    },
    // Drift-guard for the spec/field-enum label convention (issue #204).
    //
    // webrtc-runtime's public enums are state-machine types (State, Event) and a
    // structured error — those are skipped from the label convention. The HTTP
    // `Method` the WHIP/WHEP state machines used to declare themselves is now
    // `http::Method` (re-exported), so no wire-token enum of that kind is left in
    // this crate.
    //
    // This guard checks **per file**, not over the whole crate concatenated:
    // `impl_spec_display!(Name)` resolves `Name` in the module it is invoked from,
    // so per-file is the semantically correct scope.
    //
    // `media::transport::MediaEvent` (feature `media`) is likewise skipped: it is
    // a data-carrying dispatch enum (some variants hold a whole `DecryptedRtp` or
    // `rtcp_packet::CompoundPacket`), the same shape as WHIP/WHEP's own `Event` —
    // not a spec/field label. `media::transport::SetupRole` is NOT skipped: its
    // three variants are the literal SDP `a=setup` tokens (RFC 8842 §4.1), so it
    // carries `name()` + `impl_spec_display!`.
    Crate {
        name: "webrtc-runtime",
        scope: Scope::PerFile,
        matcher: Matcher::Loose,
        skip: &["Error", "Event", "State", "MediaEvent"],
        expected: Some(&["SetupRole"]),
        min_enums: None,
    },
];
