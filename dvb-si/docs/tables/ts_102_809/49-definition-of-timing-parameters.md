## Table 49 — Definition of timing parameters
_§9.5.2.3, PDF pp. 79-79_

| Name | Definition | Value |
|---|---|---|
| Probation1 | Probation if this is the first time that the service has been selected since a user initiated channel scan or factory reset. | 300 seconds (see note) |
| Probation2 | Probation if the service has been selected previously or this is the first visit to the service since an automatic channel scan. | 1 800 seconds (see note) |
| Loss1 | Coherent trust signalling has not been received for a period | The repetition period defined in clause 9.3.5.2 multiplied by 4 |
| Loss2 | Coherent trust signalling has not been received for a period or new coherent trust signalling received that is not a valid successor to the established trust signalling | As defined by the trustTimeToLive in the established manager certificate for this service as defined in clause 9.5.4.10.3 |
| NOTE: A platform specification may specify values greater than these. | | |

