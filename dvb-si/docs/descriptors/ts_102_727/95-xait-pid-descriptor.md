## Table 95 — xait_pid_descriptor
_§10.17.3, PDF p.184. An extension descriptor; signalled e.g. in the NIT network
descriptor loop to give the PID of the XAIT table._

| Syntax | No. of bits | Identifier | Value |
|---|---|---|---|
| xait_pid_descriptor() { |  |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf | 0x7F |
| &nbsp;&nbsp;descriptor_tag_extension | 8 | uimsbf | 0x0C |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |  |
| &nbsp;&nbsp;xait_PID | 16 | uimsbf |  |
| } |  |  |  |

- It is an **extension descriptor**: `descriptor_tag` `0x7F`, `descriptor_tag_extension` `0x0C`.
  In dvb-si terms the *selector* bytes (after the `0x7F`/length/`0x0C` header) are the
  2-byte `xait_PID`.
- **xait_PID**: 16-bit field; the PID value (max `0x1FFF`, i.e. the top 3 bits are
  reserved/zero). Default when absent: `0x1FFC`.
