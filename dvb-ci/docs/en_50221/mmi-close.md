# MMI — Close MMI object (close_mmi)

_Source: EN 50221 §8.6.2.1, Table 33 + close_mmi_cmd_id value table (PDF p. 36), render-verified_

The Man-Machine Interface resource (resource_identifier `00400041`) supports display
and keypad interaction with the user. Two interaction levels exist: Low-Level MMI
(detailed bitmap/keycode control) and High-Level MMI (menus and lists, host controls
look and feel). MMI modes cannot be mixed in the same session. The Close MMI object is
used in both modes.

apdu_tag `Tclose_mmi` = `9F 88 00`, Direction `--->` (module to host).

## Table 33 — Close MMI object coding

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `close_mmi () {` | | |
| &nbsp;&nbsp;close_mmi_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;close_mmi_cmd_id | 8 | uimsbf |
| &nbsp;&nbsp;`if (close_mmi_cmd_id == delay) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;delay | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

## close_mmi_cmd_id values (Table, p. 36)

| close_mmi_cmd_id | value |
|------------------|-------|
| immediate | `00` |
| delay     | `01` |
| reserved  | other values |

Semantics:
- Indicates whether the display should return immediately to the previous service or
  be delayed to allow another MMI dialogue to replace this one.
- `delay` — the `delay` byte gives the delay, in seconds, before the display should be
  returned to the current service, if another MMI session has not started in the
  meantime.
- When sent by the application, the host immediately closes the current MMI session.
  If `immediate`, the host also returns to its previous display state immediately. If
  `delay`, the host maintains the last screen state until the delay expires or another
  MMI session starts. If the application closes the session to the MMI resource
  instead, the host interprets it as a close_mmi with the `immediate` command.
