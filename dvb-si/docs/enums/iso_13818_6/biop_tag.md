# biop tag

_BIOP tagged-profile and component tags (32-bit)._

> Values rendered from the co-located drift-guard [`biop_tag.toml`](./biop_tag.toml) — the spec defines these inline in a larger syntax table, so there is no standalone table to transcribe. The drift test keeps this list in lockstep with the Rust enum.

| value | variant | spec meaning |
|---|---|---|
| 0x49534F06 | `TAG_BIOP` | BIOP Profile Body — TR 101 202 §4.7.3.2, Table 4.5 |
| 0x49534F05 | `TAG_LITE_OPTIONS` | Lite Options Profile Body — TR 101 202 §4.7.3.3, Table 4.7 |
| 0x49534F50 | `TAG_OBJECT_LOCATION` | BIOP::ObjectLocation component — TR 101 202 §4.7.3.2, Table 4.5 |
| 0x49534F40 | `TAG_CONN_BINDER` | DSM::ConnBinder component — TR 101 202 §4.7.3.2, Table 4.5 |
| 0x49534F46 | `TAG_SERVICE_LOCATION` | DSM::ServiceLocation component — TR 101 202 §4.7.3.3, Table 4.7 |
