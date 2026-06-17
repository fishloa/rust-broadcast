## Table 27 — update_method coding
_§9.5.2, PDF pp. 28-28_

| update_method | Description |
|---|---|
| 0 | immediate update: performed whatever the IRD state. |
| 1 | IRD available: the update is currently available; it will be taken into account when it does not interfere with the normal user operation. |
| 2 | next restart: the update is currently available; it will be taken into account at the next IRD restart. |
| 3 to 7 | reserved for future use |
| 8 to 14 | private use |
| 15 | reserved |

