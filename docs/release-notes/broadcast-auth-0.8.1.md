# broadcast-auth 0.8.1

_Released 2026-11-17._

Patch release. The only change is packaging metadata: `[package.metadata.cargo-machete]` records `md-5` as a false positive so the unused-dependency lint can run over the crate. There is no API or behaviour change.

## Changed

- `[package.metadata.cargo-machete]` records `md-5` (imported as `md5`) as a false positive, so the unused-dependency lint can run over this crate without a manual exception (audit 20 #2871).

---

Published from tag `broadcast-auth-v0.8.1`.
