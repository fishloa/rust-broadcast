## Table 11h — Satellite position v3 info
_§5.2.11.6, PDF pp. 49-50_

| Syntax | Number of Bits | Mnemonic |
|---|---|---|
| satellite_position_v3_info () { |
| oem_version_major | 4 | uimsbf |
| oem_version_minor | 4 | uimsbf |
| creation_date_year | 8 | uimsbf |
| reserved_zero_future_use | 7 | bslbf |
| creation_date_day | 9 | uimsbf |
| creation_date_day_fraction | 32 | spfmsbf |
| for (i=1; i<=N; i++) { |
| satellite_id | 24 | bslbf |
| reserved_zero_future_use | 3 | bslbf |
| metadata_flag | 1 | bslbf |
| usable_start_time_flag | 1 | bslbf |
| usable_stop_time_flag | 1 | bslbf |
| ephemeris_accel_flag | 1 | bslbf |
| covariance_flag | 1 | bslbf |
| if (metadata_flag == 1) { |
| total_start_time_year | 8 | uimsbf |
| reserved_zero_future_use | 7 | bslbf |
| total_start_time_day | 9 | uimsbf |
| total_start_time_day_fraction | 32 | spfmsbf |
| total_stop_time_year | 8 | uimsbf |
| reserved_zero_future_use | 7 | bslbf |
| total_stop_time_day | 9 | uimsbf |
| total_stop_time_day_fraction | 32 | spfmsbf |
| reserved_zero_future_use | 1 | bslbf |
| interpolation_flag | 1 | bslbf |
| interpolation_type | 3 | uimsbf |
| interpolation_degree | 3 | uimsbf |
| if (usable_start_time_flag == 1) { |
| usable_start_time_year | 8 | uimsbf |
| reserved_zero_future_use | 7 | bslbf |
| usable_start_time_day | 9 | uimsbf |
| usable_start_time_day_fraction | 32 | spfmsbf |
| } |
| if (usable_stop_time_flag == 1) { |
| usable_stop_time_year | 8 | uimsbf |
| reserved_zero_future_use | 7 | bslbf |
| usable_stop_time_day | 9 | uimsbf |
| usable_stop_time_day_fraction | 32 | spfmsbf |
| } |
| } |
| ephemeris_data_count | 16 | uimsbf |
| for (j=0; j< ephemeris_data_count; j++) { |
| epoch_year | 8 | uimsbf |
| reserved_zero_future_use | 7 | bslbf |
| epoch_day | 9 | uimsbf |
| epoch_day_fraction | 32 | spfmsbf |
| ephemeris_x | 32 | spfmsbf |
| ephemeris_y | 32 | spfmsbf |
| ephemeris_z | 32 | spfmsbf |
| ephemeris_x_dot | 32 | spfmsbf |
| ephemeris_y_dot | 32 | spfmsbf |
| ephemeris_z_dot | 32 | spfmsbf |
| if (ephemeris_accel_flag) { |
| ephemeris_x_ddot | 32 | spfmsbf |
| ephemeris_y_ddot | 32 | spfmsbf |
| ephemeris_z_ddot | 32 | spfmsbf |
| } |
| } |
| if (covariance_flag == 1) { |
| covariance_epoch_year | 8 | uimsbf |
| reserved_zero_future_use | 7 | bslbf |
| covariance_epoch_day | 9 | uimsbf |
| covariance_epoch_day_fraction | 32 | spfmsbf |
| for (j=0; j<21; j++) { |
| covariance_element | 32 | spfmsbf |
| } |
| } |
| } |
| } |

