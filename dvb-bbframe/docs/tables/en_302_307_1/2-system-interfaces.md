## Table 2 — System interfaces
_§5.1.1, PDF pp. 17-17_

| Location | Interface | Interface type | Connection | Multiplicity |
|---|---|---|---|---|
| Transmit station | Input | MPEG [1, 4] Transport Stream (see note 1) | from MPEG multiplexer | Single or multiple |
| Transmit station | Input (see note 2) | Generic Stream | From data sources | Single or multiple |
| Transmit station | Input (see note 3) | ACM command | From rate control unit | Single |
| Transmit station | Output | 70 MHz/140 MHz IF, L-band IF, RF (see note 4) | to RF devices | Single or multiple |
| Transmit station | Input | Mode Adaptation | from Mode Adaptation block | Single |

NOTE 1: For interoperability reasons, the Asynchronous Serial Interface (ASI) with 188 bytes format, data burst mode (bytes regularly spread over time) is recommended.

NOTE 2: For data services.

NOTE 3: For ACM only. Allows external setting of the ACM transmission mode.

NOTE 4: IF shall be higher than twice the symbol rate.

---

