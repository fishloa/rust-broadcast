## PSI section carriage and pointer_field
_§2.4.4–2.4.4.2, PDF pp. 54-55_

Carriage rules for PSI sections in TS packets (§2.4.4 intro — these ground a
section→TS packetiser):

- Sections may be variable in length; the beginning of a section is indicated
  by a **pointer_field** in the TS packet payload. Adaptation fields may
  occur in TS packets carrying PSI sections.
- **Stuffing**: packet stuffing bytes of value **0xFF** may appear in the
  payload of packets carrying PSI and/or private_sections **only after the
  last byte of a section**; in that case **all** bytes until the end of the
  packet shall also be 0xFF (decoders may discard them), and the payload of
  the **next** packet with the same PID shall begin with a pointer_field of
  value **0x00** (next section starts immediately thereafter).
- Maximum section sizes: **1024 bytes** for an ISO/IEC 13818-1-defined PSI
  table section, **4096 bytes** for a private_section.
- PAT: every TS shall contain packets with PID 0x0000 which together carry a
  complete PAT; only table_id 0x00 sections are permitted on PID 0x0000. CAT
  (PID 0x0001, table_id 0x01) is required whenever any elementary stream is
  scrambled. Each program in the PAT shall be described in a unique
  TS_program_map_section (table_id 0x02); a program definition shall not span
  more than one TS_program_map_section, and the program_map_PID shall not
  change during the continuous existence of a program. A new table version
  becomes valid when the last byte of the needed section(s), with new
  version_number and current_next_indicator `'1'`, exits B_sys (T-STD).
- The NIT is optional and private; if present it is listed in the PAT under
  reserved program_number 0x0000 and takes the form of private_sections.
- There are no restrictions on the occurrence of start codes, sync bytes or
  other bit patterns in PSI data.

### Table 2-29 — Program specific information pointer
_§2.4.4.1, PDF p. 55_

| Syntax | Bits | Mnemonic |
|---|---|---|
| pointer_field | 8 | uimsbf |

**pointer_field** (§2.4.4.2) — the number of bytes, immediately following the
pointer_field, until the first byte of the **first section that is present**
in the payload of the TS packet (0x00 = the section starts immediately after
the pointer_field). When at least one section begins in a given packet, the
payload_unit_start_indicator shall be `'1'` and the first byte of the payload
shall contain the pointer. When no section begins in the packet, the
payload_unit_start_indicator shall be `'0'` and no pointer shall be sent.

