# biop object kind

_BIOP U-U object type_id aliases (objectKind / IOR type_id, 4 bytes)._

> Values rendered from the co-located drift-guard [`biop_object_kind.toml`](./biop_object_kind.toml) — the spec defines these inline in a larger syntax table, so there is no standalone table to transcribe. The drift test keeps this list in lockstep with the Rust enum.

| value | variant | spec meaning |
|---|---|---|
| dir | `Directory` | DSM::Directory — TR 101 202 Table 4.4 (alias \"dir\", 0x64697200) |
| fil | `File` | DSM::File — TR 101 202 Table 4.4 (alias \"fil\", 0x66696C00) |
| str | `Stream` | DSM::Stream — TR 101 202 Table 4.4 (alias \"str\", 0x73747200) |
| srg | `ServiceGateway` | DSM::ServiceGateway — TR 101 202 Table 4.4 (alias \"srg\", 0x73726700) |
| ste | `StreamEvent` | BIOP::StreamEvent — TR 101 202 Table 4.4 (alias \"ste\", 0x73746500) |
