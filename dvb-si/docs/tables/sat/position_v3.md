# Satellite Position v3 info (table_id 0x4D, satellite_table_id 0x04)

**Spec:** ETSI EN 300 468 v1.19.1 §5.2.11.6
**Parser file:** `dvb-si/src/tables/sat.rs`
**Rust struct:** `Sat` (body variant `SatBody::PositionV3`)

> **Hand-corrected from the canonical PDF** (`specs/etsi_en_300_468_v01.19.01_dvb_si.pdf`,
> pp. 49-52). The automated `pdfplumber` extraction misaligned the bit-width
> column of this multi-page table (brace-only rows carry no width and shifted
> the column by one row); the widths below are read directly from the page image.

`satellite_position_v3_info` is an alternative way of specifying satellite
ephemeris using state vectors rather than orbit averages (cf. position_v2). When
a satellite's data set is split across sections, the "Metadata" group
(`metadata_flag`) is present only in the first section, and covariance data only
once per data set.

## Tables

### Table 11h — Satellite position v3 info (§5.2.11.6)

| Syntax | No. of Bits | Mnemonic |
|---|---|---|
| `satellite_position_v3_info () {` |  |  |
| &nbsp;&nbsp;oem_version_major | 4 | uimsbf |
| &nbsp;&nbsp;oem_version_minor | 4 | uimsbf |
| &nbsp;&nbsp;creation_date_year | 8 | uimsbf |
| &nbsp;&nbsp;reserved_zero_future_use | 7 | bslbf |
| &nbsp;&nbsp;creation_date_day | 9 | uimsbf |
| &nbsp;&nbsp;creation_date_day_fraction | 32 | spfmsbf |
| &nbsp;&nbsp;`for (i=1; i<=N; i++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;satellite_id | 24 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved_zero_future_use | 3 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;metadata_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;usable_start_time_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;usable_stop_time_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;ephemeris_accel_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;covariance_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`if (metadata_flag == 1) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;total_start_time_year | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved_zero_future_use | 7 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;total_start_time_day | 9 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;total_start_time_day_fraction | 32 | spfmsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;total_stop_time_year | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved_zero_future_use | 7 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;total_stop_time_day | 9 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;total_stop_time_day_fraction | 32 | spfmsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved_zero_future_use | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;interpolation_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;interpolation_type | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;interpolation_degree | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`if (usable_start_time_flag == 1) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;usable_start_time_year | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved_zero_future_use | 7 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;usable_start_time_day | 9 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;usable_start_time_day_fraction | 32 | spfmsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`if (usable_stop_time_flag == 1) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;usable_stop_time_year | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved_zero_future_use | 7 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;usable_stop_time_day | 9 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;usable_stop_time_day_fraction | 32 | spfmsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;ephemeris_data_count | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (j=0; j< ephemeris_data_count; j++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;epoch_year | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved_zero_future_use | 7 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;epoch_day | 9 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;epoch_day_fraction | 32 | spfmsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ephemeris_x | 32 | spfmsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ephemeris_y | 32 | spfmsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ephemeris_z | 32 | spfmsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ephemeris_x_dot | 32 | spfmsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ephemeris_y_dot | 32 | spfmsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ephemeris_z_dot | 32 | spfmsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`if (ephemeris_accel_flag) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ephemeris_x_ddot | 32 | spfmsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ephemeris_y_ddot | 32 | spfmsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ephemeris_z_ddot | 32 | spfmsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;`if (covariance_flag == 1) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;covariance_epoch_year | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved_zero_future_use | 7 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;covariance_epoch_day | 9 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;covariance_epoch_day_fraction | 32 | spfmsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for (j=0; j<21; j++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;covariance_element | 32 | spfmsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;`}` |  |  |
| `}` |  |  |

### Table 11i — Interpolation method for ephemeris data (§5.2.11.6)

| Value | Method |
|---|---|
| 0 | Reserved |
| 1 | Linear |
| 2 | Lagrange |
| 3 | Reserved |
| 4 | Hermite |
| 5 to 7 | Reserved |

## Field semantics (key)

- **oem_version_major/minor** — major/minor version of the OEM standard underlying the data record.
- **creation_date_year / _day / _day_fraction** — 8-bit last-two-digits year (0–99),
  9-bit day of year (1–366), 32-bit fraction of day (0.0–1.0).
- **satellite_id** — 24-bit label identifying the satellite in this loop iteration.
- **metadata_flag** — '1' in the first segment for a satellite (carries the metadata group), '0' in later segments.
- **ephemeris_accel_flag** — when '1', every ephemeris record includes the acceleration triplet.
- **covariance_flag** — when '1', the covariance matrix is present (21 elements, lower-triangular, row-major).
- **ephemeris_x/y/z** — cartesian coordinates (m); `_dot` velocities (m/s); `_ddot` accelerations (m/s²).

---
_Hand-corrected from ETSI EN 300 468 v1.19.1 §5.2.11.6 (PDF pp. 49-52)._
