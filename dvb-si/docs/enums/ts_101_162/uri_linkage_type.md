# uri linkage type

_ETSI TS 101 162 registry — uri_linkage_type codes_

> Values rendered from the co-located drift-guard [`uri_linkage_type.toml`](./uri_linkage_type.toml) — the spec defines these inline in a larger syntax table, so there is no standalone table to transcribe. The drift test keeps this list in lockstep with the Rust enum.

| value | variant | spec meaning |
|---|---|---|
| 0x00 | `OnlineSdt` | online SDT (Service Discovery & Selection) |
| 0x01 | `DvbIptvSds` | DVB-IPTV SD&S |
| 0x02 | `MaterialResolutionServer` | material resolution server |
| 0x03 | `DvbIServiceList` | DVB-I service list |
