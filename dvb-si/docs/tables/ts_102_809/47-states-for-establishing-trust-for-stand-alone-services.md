## Table 47 — States for establishing trust for stand-alone services
_§9.5.2.2, PDF pp. 77-77_

| # | State Name | Description |
|---|---|---|
| A | Service detected but not yet visited | The receiver has detected the service (for example after a factory reset, a user initiated or automated channel scan) but has not yet visited the service. After a factory reset, all services shall be treated as being found following a user initiated channel scan. |
| B | No trust established | There is no established trust signalling or trusted verification key for the service and so application authentication is not possible. See clause 9.5.2.4. |
| C | Probation | Coherent trust signalling is present but trust in this signalling has not yet been established. See clause 9.5.2.4. |
| D | Trust established | The receiver shall apply the process described in clause 9.4.2 of the present document to MPEG-2 private sections received from protectable streams. |
| E | Loss of authenticated trust signalling | There is no authenticated trust signalling in the network. Either the receiver has received new candidate trust signalling that is not a valid successor to the currently established trust signalling or the receiver is currently not receiving any coherent trust signalling. The receiver checks the stability of this situation for a period "Loss2" as defined in table 49 before determining that the currently established trust signalling will not be coming back. The receiver shall apply the process described in clause 9.4.2 of the present document to MPEG-2 private sections received from protectable streams using the trusted verification key for that service. |

