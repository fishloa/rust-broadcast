## Table 1 — MPE-IFEC generic parameters list
_§4.4, PDF pp. 14-15_

| Parameter | Unit | Category | Description | Signalling | Scoping |
|---|---|---|---|---|---|
| EP | Datagram burst | Taxonomy | IFEC Encoding Period | Direct via Time_slice_fec_identifier | Time_slice_fec_identifier |
| D | Datagram burst | Taxonomy | Datagram burst sending delay | Direct via Time_slice_fec_identifier | Time_slice_fec_identifier |
| T | rows | Table sizing | Number of ADST, ADT, iFDT rows; T=MPE-FEC Frame rows /G | Indirect via Time_slice_fec_identifier | Time_slice_fec_identifier |
| C | columns | Table sizing | Number of ADST columns | Direct via Time_slice_fec_identifier | Time_slice_fec_identifier |
| R | sections | Table sizing | Maximum number of MPE IFEC sections per Time-Slice Burst | Direct via Time_slice_fec_identifier | Time_slice_fec_identifier |
| K | columns | Table sizing | Number of ADT columns = EP*C | Indirect via Time_slice_fec_identifier | Time_slice_fec_identifier |
| N | columns | Table sizing | Number of iFDT columns = EP*R*G | Indirect via Time_slice_fec_identifier | Time_slice_fec_identifier |
| G | columns | Table sizing | Maximum number of iFDT columns per IFEC section | Direct | Time_slice_fec_identifier |
| M | ADT | Protocol sizing | Number of concurrent encoding matrices M | Indirect (formula dependent on T_code and given in the parameter definition of clause 6) | Time_slice_fec_identifier |
| kmax | N/A | Protocol sizing | Modulo operator for IFEC burst counter | Indirect (formula dependent on T_code and given in the parameter definition of clause 6) | Time_slice_fec_identifier |
| lmax | N/A | Protocol sizing | Maximum backward pointing for datagram burst size used in PREV_BURST_SIZE parameter in clause 3.5 | Indirect (formula dependent on T_code and given in the parameter definition of clause 6) | Time_slice_fec_identifier |
| k | datagram burst | Index | continuous burst counter internal to sender | N/A | Loop |
| k' | IFEC burst | field | Burst number | N/A | IFEC section |

