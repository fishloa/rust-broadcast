# dvb-simulcrypt 0.6.0

_Released 2026-10-05._

### Changed (breaking)
- Removed the generic `Parse` impl for `SimulcryptMessage` (#1098). It always
  decoded against the ECMG⇔SCS interface, so a message from any other
  interface reached through generic `T: Parse` tooling came back silently
  mislabelled. Use `SimulcryptMessage::parse_on(interface, bytes)`, which was
  already required for correct decoding and is now the only entry point.

### Added
- `SimulcryptMessage::frame_len(&[u8]) -> Option<usize>` (#1098): lets a
  TCP-stream caller tell "read more" from "malformed" before `parse_on`,
  whose `InvalidMessageLength` could not distinguish the two.

---

Published from tag `dvb-simulcrypt-v0.6.0`.
