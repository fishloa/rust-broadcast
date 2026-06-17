## Table 3 — Service state
_§4.4.1, PDF pp. 36-36_

|  | Service | present | in |  |  |
|---|---|---|---|---|---|
| PAT | PMT | SDT | SDT running | EIT p/f | State of the service |
|  |  |  | status |  |  |
| yes | no | x | x | x | Transition state |
| no | yes | x | x | x | Transition state |
| yes | yes | no | - | x | Transition state |
| yes | yes | yes | x | no | Transition state |
| yes | yes | yes | running or | yes | Service is running and broadcasting |
|  |  |  | undefined |  |  |
| yes | yes | yes | pausing or not | x | Transition state |
|  |  |  | running |  |  |
| no | no | no | - | yes | Transition state |
| no | no | no | - | no | Idle state, corresponds to the start of the |
|  |  |  |  |  | creation of a service or end state of a service |
| no | no | yes | running | x | Transition state |

