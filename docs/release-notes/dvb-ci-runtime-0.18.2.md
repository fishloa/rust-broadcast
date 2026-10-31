# dvb-ci-runtime 0.18.2

_Released 2026-10-31._

### Added
- `Driver::update_service` (and the `CaDescrambler` forwarder) re-sends the `ca_pmt` for a programme already in the managed set with `ca_pmt_list_management = update` (EN 50221 §8.4.3.4 Table 25), replacing the stored service record so later `remove_service`/re-query use the new PMT. Returns `CaError::NotManaged { program_number }` for an unknown programme. Re-sending `add` for an existing programme was already equivalent per the spec; this adds the explicit entry point.

---

Published from tag `dvb-ci-runtime-v0.18.2`.
