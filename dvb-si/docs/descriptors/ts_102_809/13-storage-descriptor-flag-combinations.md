## Table 13 — Storage descriptor flag combinations
_§5.2.11.2, PDF pp. 28-28_

| not_launchable_from_broadcast | launchable_completely_from_cache | is_launchable_with_older_version | Description |
|---|---|---|---|
| 0 | 0 | 0 | Normal case. |
| 0 | 0 | 1 | Shall not be signalled. |
| 0 | 1 | 0 | Shall not be signalled. |
| 0 | 1 | 1 | Shall not be signalled. |
| 1 | 0 | 0 | Runs if signalled version is stored. |
| 1 | 0 | 1 | Runs if signalled or older version is stored. |
| 1 | 1 | 0 | Runs completely from cache if signalled version is stored. The application cannot be stored due to unavailability of the object carousel for the current service. |
| 1 | 1 | 1 | Runs if signalled or older version is stored. The application cannot be stored due to unavailability of the object carousel for the current service. |

