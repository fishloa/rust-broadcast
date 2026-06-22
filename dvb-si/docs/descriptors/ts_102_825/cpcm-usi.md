# CPCM Usage State Information (USI)

_ETSI TS 102 825-4 V1.2.2 §5.4 (Table 8), with copy/propagation code tables 9–11._
_Carried as the `selector_byte`s of the `cpcm_delivery_signalling_descriptor`
(TS 102 825-9 §4.1.5 Table 2, extension tag 0x01) when `cpcm_version == 1`._

## Table 8 — `CPCM_usage_state_information()`

| Syntax | Bits | Type | Notes |
|---|---|---|---|
| `length` | 8 | uimsbf | bytes following the length field |
| `copy_control` | 3 | uimsbf | Table 9 |
| `do_not_cpcm_scramble` | 1 | bslbf | 1 = do not scramble |
| `viewable` | 1 | bslbf | |
| `view_window_activated` | 1 | bslbf | gates the view_window_* fields |
| `view_period_activated` | 1 | bslbf | gates view_period_from_first_playback |
| `simultaneous_view_count_activated` | 1 | bslbf | gates simultaneous_view_count |
| `move_local` | 1 | bslbf | |
| `view_local` | 1 | bslbf | |
| `move_and_copy_propagation_information` | 2 | uimsbf | Table 10 |
| `view_propagation_information` | 2 | uimsbf | Table 11 |
| `remote_access_date_moving_window_flag` | 1 | bslbf | gates remote_access_date |
| `remote_access_date_immediate_flag` | 1 | bslbf | gates remote_access_date |
| `remote_access_record_flag` | 1 | bslbf | |
| `export_controlled_cps` | 1 | bslbf | gates the CPS vector |
| `export_beyond_trust` | 1 | bslbf | |
| `disable_analogue_sd_export` | 1 | bslbf | |
| `disable_analogue_sd_consumption` | 1 | bslbf | |
| `disable_analogue_hd_export` | 1 | bslbf | |
| `disable_analogue_hd_consumption` | 1 | bslbf | |
| `image_constraint` | 1 | bslbf | |
| `if (view_window_activated==1) { view_window_start` | 40 | CPCM_date_time | |
| `  view_window_end` | 40 | CPCM_date_time | `}` |
| `if (view_period_activated==1) { view_period_from_first_playback` | 16 | CPCM_playback_period | `}` |
| `if (simultaneous_view_count_activated==1) { simultaneous_view_count` | 8 | uimsbf | `}` |
| `if (rad_immediate_flag\|\|rad_moving_window_flag) { remote_access_date` | 40 | CPCM_date_time | `}` |
| `if (export_controlled_cps==1) { cps_vector_count` | 8 | uimsbf | |
| `  for(i<cps_vector_count){ C_and_R_regime_mask` | 8 | bslbf | per-entry |
| `  cps_vector_length` | 16 | uimsbf | |
| `  for(j<cps_vector_length){ cps_vector_byte` | 8 | bslbf | `}}}` |

### Byte layout (the fixed prefix is byte-aligned)

- **byte 0**: `length`
- **byte 1**: `copy_control`[7:5] · `do_not_cpcm_scramble`[4] · `viewable`[3] · `view_window_activated`[2] · `view_period_activated`[1] · `simultaneous_view_count_activated`[0]
- **byte 2**: `move_local`[7] · `view_local`[6] · `move_and_copy_propagation_information`[5:4] · `view_propagation_information`[3:2] · `remote_access_date_moving_window_flag`[1] · `remote_access_date_immediate_flag`[0]
- **byte 3**: `remote_access_record_flag`[7] · `export_controlled_cps`[6] · `export_beyond_trust`[5] · `disable_analogue_sd_export`[4] · `disable_analogue_sd_consumption`[3] · `disable_analogue_hd_export`[2] · `disable_analogue_hd_consumption`[1] · `image_constraint`[0]
- then the conditional fields in declaration order. `CPCM_date_time` = 40 bits (5 bytes); `CPCM_playback_period` = 16 bits (2 bytes).

## Table 9 — `copy_control` (cci_and_zero_retention)

| Value | Meaning |
|---|---|
| 0 | Copy Control Not Asserted |
| 1 | Copy Once |
| 2 | Copy No More |
| 3 | Copy Never — Zero Retention Not Asserted |
| 4 | Copy Never — Zero Retention Asserted |
| 5–7 | Reserved for future use |

## Table 10 — `move_and_copy_propagation_information`

| Value | Meaning |
|---|---|
| 0 | MLAD — within the same Localized AD |
| 1 | MGAD — within the same Geographically-constrained AD |
| 2 | MAD — within the same Authorized Domain |
| 3 | MCPCM — to any CPCM-compliant Storage Entity |

## Table 11 — `view_propagation_information`

| Value | Meaning |
|---|---|
| 0 | VLAD — Consumption within the same Localized AD |
| 1 | VGAD — within the same Geographically-constrained AD |
| 2 | VAD — within the same Authorized Domain |
| 3 | VCPCM — using any CPCM-compliant Consumption Point |

Semantics (§5.4): the 1-bit flags are booleans as named. `length` counts the bytes
after itself. `CPCM_date_time` (Table 3) and `CPCM_playback_period` are treated at
the descriptor layer as fixed-width opaque values (5 / 2 bytes) unless decoded by a
caller. Note: a non-version-1 `cpcm_version`, or a selector whose `length` does not
match the available bytes, is NOT a valid USI — keep it as raw `selector_bytes`.
