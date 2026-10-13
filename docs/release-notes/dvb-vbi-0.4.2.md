# dvb-vbi 0.4.2

_Released 2026-10-13._

### Fixed
- `DataUnit::serialize_into` now validates that `id` is a valid Table 3
  `data_unit_id` for the payload kind and returns the new
  `Error::DataUnitIdPayloadMismatch` instead of writing a mislabelled unit
  (e.g. `{ id: Vps, payload: Teletext }`). Added
  `DataUnitPayload::is_consistent_with`. Fields stay public; `parse` always
  yields consistent pairs. Non-breaking (`Error` is `#[non_exhaustive]`).

---

Published from tag `dvb-vbi-v0.4.2`.
