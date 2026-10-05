# dvb-simulcrypt 0.6.0

_Released 2026-10-05._

Breaking release (0.5 -> 0.6) with one removal and one addition, both in `SimulcryptMessage` (ETSI TS 103 197). Anyone who called `SimulcryptMessage::parse(bytes)` or passed a `SimulcryptMessage` to generic `T: Parse` code must switch to `parse_on`. Everyone else is unaffected. Wire parsing and serialization are otherwise unchanged.

## Breaking change: the generic `Parse` impl is removed (#1098)

The `Parse` impl on `SimulcryptMessage` always decoded `message_type` and `parameter_type` against the ECMG-SCS interface. A message belonging to any other interface (`Interface::EmmgPdgMux` or `Interface::CpSigPSig`) that reached it through generic `T: Parse` tooling therefore came back silently mislabelled. `SimulcryptMessage::parse_on(interface, bytes)`, which was already required for correct decoding, is now the only entry point.

```rust
// before (0.5): decoded as ECMG<->SCS whatever the connection really was
let msg = SimulcryptMessage::parse(bytes)?;
// after (0.6): name the interface of the connection the bytes came from
let msg = SimulcryptMessage::parse_on(Interface::EcmgScs, bytes)?;
```

`Serialize` is unchanged. `parse_on` expects a complete frame: it returns `Error::BufferTooShort` for a truncated header or TLV header, `Error::InvalidMessageLength` if `message_length` overruns the buffer, and `Error::TruncatedParameter` if a `parameter_length` runs past the message body.

## New API: `SimulcryptMessage::frame_len` (#1098)

`SimulcryptMessage::frame_len(bytes: &[u8]) -> Option<usize>` returns the total frame length (header plus `message_length`), or `None` when fewer than the generic header's bytes are available. A caller reading a TCP stream (the transport TS 103 197 assumes) uses it to tell "read more" from "malformed" before `parse_on`: `None`, or `Some(n)` with `n > bytes.len()`, means buffer more and retry; `Some(n)` with `n <= bytes.len()` means `bytes[..n]` is a complete frame to hand to `parse_on`. Before, `parse_on`'s `InvalidMessageLength` could not distinguish a partial read from a genuinely bad length.

## Dependencies

`broadcast-common` `9.3` -> `9.4` (see `broadcast-common-9.4.0.md`). No other `Cargo.toml` change.

---

Published from tag `dvb-simulcrypt-v0.6.0`.
