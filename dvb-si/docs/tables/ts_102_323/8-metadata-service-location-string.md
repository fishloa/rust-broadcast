## Table 8 — Metadata service location string
_§5.3.4.2, PDF pp. 22-22_

| metadata_service_location_string | = mds_explicit_path \| mds_default_path |
|---|---|
| mds_explicit_path | = "/" path_segments |
| mds_default_path | = "/" metadata_service_id_string |
| metadata_service_id_string | = hex_string |
| hex_string | = 2\*hex |
| hex | = digit \| "A" \| "B" \| "C" \| "D" \| "E" \| "F" \| "a" \| "b" \| "c" \| "d" \| "e" \| "f" |
| digit | = "0" \| "1" \| "2" \| "3" \| "4" \| "5" \| "6" \| "7" \| "8" \| "9" |

