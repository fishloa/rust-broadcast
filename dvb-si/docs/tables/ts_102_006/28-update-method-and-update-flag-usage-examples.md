## Table 28 — update_method and update_flag usage examples
_§9.5.2, PDF pp. 28-28_

| update_method | update_flag = 0 (manual) | update_flag = 1 (automatic) |
|---|---|---|
| 0 (immediate update) | A message asking for the update of the IRD shall be displayed and the IRD shall wait for the user agreement. | The update shall be performed whatever the state of the IRD (force update). |
| 1 (IRD available) | A message shall inform the user about the availability of an update only if it does not disturb the user (front panel etc.) but current display should not be disturbed by a message. | The update shall be performed only if the IRD is available and the update will not disturb the user. |
| 2 (next restart) | At the next restart a message shall ask the user for his agreement to perform the IRD update. | The update will automatically be performed at the next restart. |

