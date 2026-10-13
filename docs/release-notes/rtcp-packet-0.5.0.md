# rtcp-packet 0.5.0

_Released 2026-10-13._

**BREAKING.** `SenderReport` and `ReceiverReport` gain `profile_extensions` (RFC 3550 §6.4.1/§6.4.2) and become `#[non_exhaustive]`; build them with `SenderReport::new(..)` / `ReceiverReport::new(..)` + `.with_profile_extensions(..)`. Parsing keeps bytes after the report blocks instead of dropping them; a length that is not a multiple of 4 is rejected. Dependents `rist-runtime`, `transmux` and `webrtc-runtime` migrated; every crate depending on them moves to the new caret epoch (see v11.1.0 note).


### BREAKING
- `SenderReport` / `ReceiverReport` gain `profile_extensions: Vec<u8>`
  (RFC 3550 §6.4.1/§6.4.2): bytes after the report blocks were previously
  silently dropped on parse; they are now retained, written by serialize and
  counted in the length word. A length that is not a multiple of 4 is rejected
  with `Error::InvalidValue { field: "profile_extensions_len" }` on both parse
  and serialize.
- Both structs are now `#[non_exhaustive]`; construct with the new
  `SenderReport::new(..)` / `ReceiverReport::new(..)` plus
  `.with_profile_extensions(Vec<u8>)`. Struct literals outside the crate no
  longer compile.
- In-workspace dependents migrated to the constructors: rist-runtime
  (tests), webrtc-runtime (`media/transport.rs`, SRTP vector test), transmux
  (tests `rtcp.rs`, `rtcp_av_sync.rs`); crate docs/README/examples updated.

---

Published from tag `rtcp-packet-v0.5.0`.
