# dvb-simulcrypt

[![Crates.io](https://img.shields.io/crates/v/dvb-simulcrypt.svg)](https://crates.io/crates/dvb-simulcrypt)
[![docs.rs](https://img.shields.io/docsrs/dvb-simulcrypt)](https://docs.rs/dvb-simulcrypt)

DVB SimulCrypt head-end CA message framing (ETSI TS 103 197): a codec for the
messages a head-end's conditional-access components exchange over TCP.

## What it does / what it does not

**Does.** A DVB SimulCrypt head-end lets several conditional-access systems
share one multiplex. Its components (ECM generator, EMM generator, multiplexer,
CA-specific SI generators) talk over TCP using one generic message layout. This
crate parses and builds that layout and decodes the `message_type` /
`parameter_type` values for three interfaces. Users are head-end software,
test tools and protocol analysers.

**Does not.**
- It does not open sockets or run the protocol state machines (channel and
  stream setup, test/status handshakes, bandwidth negotiation); the caller
  drives TCP.
- It does not do cryptography: control words, ECMs, EMMs and private data are
  carried as opaque borrowed bytes, never decrypted or interpreted.
- It does not type the individual parameter *values* (a parameter value is a
  byte slice; only the type codes and a few value tables are modelled).
- Interfaces EIS-SCS, (P)SIG-MUX, ACG-EIS and SIMCOMP-MUXCONFIG share the same
  framing but their registries are not modelled, and the DVB-H `0x05` protocol
  version (annex N) is not modelled.
- It does not scramble anything; see `dvb-csa` for the cipher.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ETSI TS 103 197, DVB; Head-end implementation of DVB SimulCrypt | V1.5.1 (`etsi_ts_103_197_v01.05.01_simulcrypt`, in `private/specs-md/INDEX.md`) | §4.4.1 Table 1b `generic_message`: 5-byte header plus TLV parameters; Table 2 protocol version | Full | `message_length` and `parameter_length` recomputed on serialize; `SimulcryptMessage::frame_len` for stream reassembly |
| | | Clause 5 ECMG-SCS: message types (Table 3 incl. `CW_provision` `0x0201`, `ECM_response` `0x0202`), parameter types (Table 5), error status (Table 6) | Typed-opaque | Type codes typed; CW/ECM payloads opaque |
| | | Clause 6 EMMG/PDG-MUX: message types, parameter types (Table 7), error status (Table 8), `DataType` (§6.2.3), `SectionTspktFlag` | Typed-opaque | `datagram` payload opaque |
| | | Clause 8 C(P)SIG-(P)SIG: message types `0x0301`-`0x0321`, parameter types (Table 36) | Typed-opaque | Registry only |
| | | Other interfaces (EIS-SCS, (P)SIG-MUX, ACG-EIS, SIMCOMP-MUXCONFIG), XML-based protocols (Table 4), annex N DVB-H | Not implemented | Registries not modelled; the generic framing is the same |

## Interface scoping

The 16-bit `message_type` / `parameter_type` spaces are interface-scoped: the
same value means different things on different interfaces, and the interface is
not on the wire (it is fixed by which TCP connection a message arrived on). So
`SimulcryptMessage::parse_on` takes an `Interface` hint and decodes into the
matching interface-tagged `MessageType` / `ParameterType` enums.
`SimulcryptMessage` deliberately has no `Parse` impl (a documented exception:
`Parse::parse(&[u8])` has nowhere to carry the hint). It implements `Serialize`,
so the round-trip invariant is `parse_on` then `serialize_into`.

## Feature flags

| Feature | Default | Effect |
|---|---|---|
| `std` | yes | Links `std`. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | no | `serde::Serialize` derives on public types. |

## Quick start

```rust
use dvb_simulcrypt::{
    EcmgScsMessageType, EcmgScsParameterType, Interface, MessageType, Parameter,
    ParameterType, SimulcryptMessage,
};
use broadcast_common::traits::Serialize;

let ecm_channel_id = [0x00, 0x2A]; // 0x002A
let super_cas_id = [0x00, 0x01, 0x00, 0x02]; // CA_system_id | subsystem_id
let msg = SimulcryptMessage::new(
    Interface::EcmgScs.protocol_version(),
    MessageType::EcmgScs(EcmgScsMessageType::ChannelSetup),
    vec![
        Parameter::new(
            ParameterType::EcmgScs(EcmgScsParameterType::EcmChannelId),
            &ecm_channel_id,
        ),
        Parameter::new(
            ParameterType::EcmgScs(EcmgScsParameterType::SuperCasId),
            &super_cas_id,
        ),
    ],
);

let mut buf = vec![0u8; msg.serialized_len()];
msg.serialize_into(&mut buf).unwrap();
assert_eq!(SimulcryptMessage::parse_on(Interface::EcmgScs, &buf).unwrap(), msg);
```

Runnable examples: `cargo run -p dvb-simulcrypt --example build_channel_setup`
and `--example parse_cw_provision`.

## What is implemented

- **Framing:** `SimulcryptMessage` (`new`, `parse_on`, `frame_len`, `find`),
  `Parameter`, `HEADER_LEN`, `PARAMETER_HEADER_LEN`.
- **Interface dispatch:** `Interface` (`EcmgScs`, `EmmgPdgMux`, `CpSigPSig`),
  `MessageType`, `ParameterType`.
- **ECMG-SCS:** `EcmgScsMessageType`, `EcmgScsParameterType`, `EcmgErrorStatus`.
- **EMMG/PDG-MUX:** `EmmgMuxMessageType`, `EmmgMuxParameterType`,
  `EmmgErrorStatus`, `DataType`, `SectionTspktFlag`.
- **C(P)SIG-(P)SIG:** `CpSigMessageType`, `CpSigParameterType`.
- **Errors:** `Error` / `Result` (`#[non_exhaustive]`).

Field-level transcriptions live in the crate's `docs/` directory
(`message-framing.md`, `ecmg-scs.md`, `emmg-pdg-mux.md`, `cpsig-psig.md`,
`parameter-types.md`).

## Design notes

- Zero-copy: parameter values borrow from the input.
- No raw passthrough: lengths are recomputed from the typed fields on serialize.
- Round-trip tested against a committed capture of a `CW_provision` message
  (`tests/fixture_cw_provision.rs`) and truncation/length-overflow tests.
- Public enums are `#[non_exhaustive]` with `name()` and `Display`.
- `no_std` + `alloc`.

## Related crates

Depends only on `broadcast-common`. `dvb-csa` is the scrambling cipher the
control words feed; `dvb-ci` / `dvb-ci-runtime` are the receiver-side CAM
counterpart; `dvb-si` carries the CA descriptors and CAT/PMT signalling.

## MSRV and licence

Rust 1.95.0. MIT OR Apache-2.0.
