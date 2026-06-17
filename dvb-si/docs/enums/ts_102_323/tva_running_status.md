## Table 115 — Running status
_§11.2.4, PDF pp. 101-102_

| Value | Meaning | Description |
|---|---|---|
| 0 | Reserved | |
| 1 | Not yet running | Receivers shall treat the item of content as not yet running. This can be used when the item of content is still to be broadcast, but is unlikely to start until sometime after the most recently indicated scheduled start_time. |
| 2 | Starts (or restarts) shortly | Receivers shall prepare for the change of running_status to "running" to occur shortly. This optional mode can be used to assist receivers in preparing their resources for recording. If used this value should be signalled for 30 seconds before changing to "Running". |
| 3 | Paused | Receivers shall treat the item of content as paused. This can be used when broadcast of the item of content has already started, but at this time the content being broadcast is not a part it. It is assumed that the transmission of relevant content will resume at a later time. It is recommended that the paused state is only used for short interruptions not appearing in the schedule. |
| 4 | Running | Receivers shall treat the item of content as running. This can be used to indicate that at this time the content being broadcast is part of the item of content. |
| 5 | Cancelled | Receivers shall treat the item of content as cancelled. This can be used to indicate that the item of content has been pulled either before commencement of, or part way through transmission. It is recommended that this is signalled for 10 seconds. |
| 6 | Completed | Receivers shall treat the transmission of item of content as being completed. It is recommended that this is signalled for 10 seconds. |
| 7 | Reserved | |

