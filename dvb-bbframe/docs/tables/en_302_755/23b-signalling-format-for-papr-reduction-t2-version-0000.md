## Table 23b — Signalling format for PAPR reduction (T2_VERSION > '0000')
_§7.2.2, PDF p. 63_

| Value | PAPR reduction |
|---|---|
| 0000 | L1-ACE is used and TR is used on P2 symbols only |
| 0001 | L1-ACE and ACE only are used |
| 0010 | L1-ACE and TR only are used |
| 0011 | L1-ACE, ACE and TR are used |
| 0100 to 1111 | Reserved for future use |

NOTE: The term ACE refers to the algorithm as defined in clause 9.6.1 and the term L1-ACE refers to the algorithm defined in clause 7.3.3.3. The effect of L1-ACE may be turned off by setting the parameter CL1_ACE_MAX to a value of 0.

