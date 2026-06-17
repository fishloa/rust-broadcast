## Table 50 — Identifying certificate types from properties
_§9.5.4.4, PDF pp. 85-85_

| Successor ID present | Successor ID matches SubjectKeyID of another certificate within collection | KeyUsage bits Asserted — digitalSig | KeyUsage bits Asserted — KeyCertSign | Self-signed | Certificate type |
|---|---|---|---|---|---|
| Y | N | 1 | 0 | Y | Manager (note 1) |
| Y | N | 0 | 1 | Y | Manager |
| Y | Y | 1 | 0 | Y | Previous manager |
| Y | Y | 0 | 1 | Y | Previous manager (note 2) |
| N | Not applicable | 0 | 1 | N | Intermediate |
| N | Not applicable | 1 | 0 | N | Verification |
| Y | Matches own subjectKeyID | 0 | 0 | Y | Termination of trust |
| NOTE 1: In this case the certificate chain has only one certificate and the manager certificate also provides the verification key. NOTE 2: In this case the certificate was previously a manager that was used in a chain with one certificate and the manager certificate also provided the verification key. | | | | | |

