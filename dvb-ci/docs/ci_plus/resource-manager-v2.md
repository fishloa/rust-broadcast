# Resource Manager — version 2

_Source: ETSI TS 101 699 V1.1.1 §4.2.1, Tables 3–7 (PDF pp. 13–17), render-verified_

The Resource Manager is a host-provided resource that controls the acquisition and provision of resources to all applications. Version 2 (`resource_identifier` = `0x00010042`) adds the **Module ID establishment** part to the protocol; the Resource Profile establishment part (Profile Enquiry / Profile Reply / Profile Changed) is identical to version 1 of the EN 50221 Resource Manager. Module ID establishment lets the host allocate a 6-bit Module ID to a module so that multiple instances of the same class & type of resource can be distinguished by `resource_instance`.

The version-2 APDU set comprises the three version-1 objects (Profile Enquiry, Profile Reply, Profile Changed) plus two new objects (Module ID Send, Module ID Command).

## Protocol flow (Figure 3, p. 14 — Module ID establishment)

On a successful session open the Resource Manager sends `profile_enq` to the module. A version-2 module replies with `module_id_send` (returning a previously allocated Module ID, or `0` if none). The host then sends `module_id_command`:

- `command = 0x01` (Acknowledgement) — host accepts the Module ID; module continues to Resource Profile establishment (`profile_reply`).
- `command = 0x02` (Set_ModuleID) — `module_id` carries a new ID; the module responds with a further `module_id_send` echoing the new ID, the host acknowledges, then the Profile protocol continues.

Unless the resource manager has version ≥ 2 the module shall omit the Module ID establishment part entirely and not declare resources that depend on a Module ID.

## Table 3 — Profile Enquiry object coding (p. 15)

apdu_tag `Tprofile_enq` = `0x9F8010`, Direction host ---> app (and app ---> host; the profile enquiry requests the recipient to reply with its list of resources in a Profile Reply object).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `profile_enq () {` | | |
| &nbsp;&nbsp;profile_enq_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 0 | | |
| `}` | | |

## Table 4 — Profile Reply object coding (p. 16)

apdu_tag `Tprofile_reply` = `0x9F8011`, Direction app ---> host. Sent in response to a Profile Enquiry; lists the resources the sender is able to provide.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `profile_reply () {` | | |
| &nbsp;&nbsp;profile_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = N*4 | | |
| &nbsp;&nbsp;`for (i = 0; i < N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;resource_identifier() | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `resource_identifier()` — a 32-bit resource identifier (hence `length_field() = N*4`). Resource identifiers for the minimum set of resources that shall be provided are listed in EN 50221 §8.8; optional resources are listed in its annexes; additional resources are defined in this document under "Command Interface – Additional Resources".

## Table 5 — Profile Changed object coding (p. 16)

apdu_tag `Tprofile_changed` = `0x9F8012`, Direction app <--- host (and app ---> host; either party may notify a change). Notifies the recipient that a resource has changed; the recipient replies with a Profile Enquiry to obtain the updated list.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `profile_changed () {` | | |
| &nbsp;&nbsp;profile_changed_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 0 | | |
| `}` | | |

## Table 6 — Module ID Send object coding (p. 16)

apdu_tag `Tmodule_id_send` = `0x9F8013`, Direction app ---> host (m → h). Sends the current Module ID in response to either a Profile Enquiry or a Module ID Command updating the Module ID.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `module_id_send () {` | | |
| &nbsp;&nbsp;module_id_send_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 1 | | |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;module_id | 6 | uimsbf |
| `}` | | |

Field notes:
- `module_id` — the Module ID allocated and managed locally by the host. Only the 6 least-significant bits are used; the two most-significant bits shall be set to zero when assigning the value and shall be ignored when reading it. A Module ID of zero shall be used by the module if one has not already been allocated by the host in a previous transaction. A value allocated by the host shall be retained by the module through removal of power.

## Table 7 — Module ID Command object coding (p. 17)

apdu_tag `Tmodule_id_command` = `0x9F8014`, Direction app <--- host (h → m). Sent as an acknowledgement of a Module ID Send object, or to set/update an existing Module ID.

> ⚠ The PDF prints the Table 7 caption as "Module ID Send object coding" — this is a spec typo; §4.2.1.8 titles the subclause "Module ID Command" and the table body defines `module_id_command()`. Transcribed below as Module ID Command.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `module_id_command () {` | | |
| &nbsp;&nbsp;module_id_command_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 2 | | |
| &nbsp;&nbsp;command | 8 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;module_id | 6 | uimsbf |
| `}` | | |

### command values (p. 17)

| command | command value |
|---------|---------------|
| Acknowledgement | `01` |
| Set_ModuleID | `02` |
| reserved | other values |

Field notes:
- `command` — `0x01` Acknowledgement: the host accepts the Module ID as allocated and the module continues to the Resource Profile establishment phase. `0x02` Set_ModuleID: `module_id` carries a new Module ID; the module responds with a further Module ID Send echoing the new ID.
- `module_id` — as defined for Module ID Send (Table 6 above).
