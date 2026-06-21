# dvb-simulcrypt

[![Crates.io](https://img.shields.io/crates/v/dvb-simulcrypt.svg)](https://crates.io/crates/dvb-simulcrypt)
[![docs.rs](https://img.shields.io/docsrs/dvb-simulcrypt)](https://docs.rs/dvb-simulcrypt)

DVB SimulCrypt head-end CA message framing — ETSI TS 103 197. A **codec** for
the messages the head-end carries over TCP between its conditional-access
components; this crate does not open sockets.

Implements:

- **`SimulcryptMessage`** — the generic `generic_message` (TS 103 197 §4.4.1,
  Table 1b): a 5-byte header (`protocol_version` + `message_type` +
  `message_length`, big-endian) followed by an ordered list of TLV
  **`Parameter`**s (`parameter_type` + `parameter_length` + value).
  `message_length` and every `parameter_length` are **recomputed on serialize**
  from the typed fields — there is no raw passthrough.
- **ECMG ⇔ SCS** (clause 5): `EcmgScsMessageType` (channel/stream
  setup/test/status/close/error, `CW_provision` `0x0201`, `ECM_response`
  `0x0202`) + the Table 5 `EcmgScsParameterType` registry (`Super_CAS_id`
  `0x0001` … `ECM_id` `0x0019`, `error_status` `0x7000`, `error_information`
  `0x7001`) + `EcmgErrorStatus` (Table 6).
- **EMMG/PDG ⇔ MUX** (clause 6): `EmmgMuxMessageType` (channel/stream messages,
  `stream_BW_request`/`allocation`, `data_provision` `0x0211`) + the Table 7
  `EmmgMuxParameterType` registry + `EmmgErrorStatus` (Table 8), plus the
  `DataType` (§6.2.3) and `SectionTspktFlag` value tables.

## Interface scoping

The 16-bit `message_type` / `parameter_type` spaces are **interface-scoped**:
the same value means different things on different interfaces, and the interface
is not on the wire — it is fixed by which TCP connection a message arrived on.
So `SimulcryptMessage::parse_on` takes an `Interface` hint and decodes the raw
values into the matching interface-tagged `MessageType` / `ParameterType` enums.

## Signalling only — no crypto

The control words in `CP_CW_combination` / `CW_encryption`, the ECMs in
`ECM_datagram`, and the EMM / private data in `datagram` are carried as **opaque
borrowed bytes**. This crate frames and parses them; it never decrypts or
interprets them. The non-implemented interfaces (C(P)SIG⇔(P)SIG, EIS⇔SCS,
(P)SIG⇔MUX, ACG⇔EIS, SIMCOMP⇔MUXCONFIG) share the same framing but are not
modelled.

`#![no_std]` + `alloc`; depends only on `dvb-common`.

## Quick start

```rust
use dvb_simulcrypt::{
    EcmgScsMessageType, EcmgScsParameterType, Interface, MessageType, Parameter,
    ParameterType, SimulcryptMessage,
};
use dvb_common::traits::{Parse, Serialize};

let ecm_channel_id = [0x00, 0x2A];
let super_cas_id = [0x00, 0x01, 0x00, 0x02];
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

## Examples

```sh
cargo run -p dvb-simulcrypt --example build_channel_setup
cargo run -p dvb-simulcrypt --example parse_cw_provision
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | no      | `serde::Serialize` derives on public types. |

## Minimum Supported Rust Version

1.81

## License

MIT OR Apache-2.0
