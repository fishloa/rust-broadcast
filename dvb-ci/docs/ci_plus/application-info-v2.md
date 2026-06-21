# Application Information — version 2

_Source: ETSI TS 101 699 V1.1.1 §5, Table 11 (PDF p. 21), render-verified_

Version 2 of the Application Information resource (`resource_identifier` = `0x00020042`) extends the set of `application_type` values that can be coded in the Application Info object. The base object layouts (Application Info Enquiry, Application Info, Enter Menu) are **unchanged from EN 50221 §8.4** — version 2 adds only new `application_type` enum values (§5.1.1) and the unrecognized-type handling rule (§5.1.2). No new syntax tables are introduced for the APDU objects themselves.

## APDU objects (base layout per EN 50221 §8.4 — unchanged)

| Object | apdu_tag | Direction |
|--------|----------|-----------|
| Application Info Enquiry | `Tapplication_info_enq` = `0x9F8020` | host ---> app |
| Application Info | `Tapplication_info` = `0x9F8021` | app ---> host |
| Enter Menu | `Tenter_menu` = `0x9F8022` | host ---> app |

The `application_info()` object carries `application_type` (8 bits), `application_manufacturer` (16 bits), `manufacturer_code` (16 bits), and a `menu_string` (per EN 50221 §8.4 Table 4 — base object syntax unchanged). TS 101 699 §5 changes only the permitted value set of the `application_type` field, given below.

## Table 11 — Application type coding (p. 21)

| Application type | application_type |
|------------------|------------------|
| Conditional_Access | `01` |
| Electronic_Programme_Guide | `02` |
| Software_upgrade | `03` |
| Network_interface | `04` |
| Accessibility_aids | `05` |
| Unclassified | `06` |
| reserved | other values |

Field notes (§5.1.1):
- `Software_upgrade` (`0x03`) — modules that upload software to the host to upgrade the host's software. No specific upload protocol is implied by this application type.
- `Network_interface` (`0x04`) — any type of input module (including types 'A' and 'B' described in this document) can present an application of Network_interface type.
- `Accessibility_aids` (`0x05`) — modules that provide a facility for those with some form of disability or impairment. Audio description modules are in this type.
- `Unclassified` (`0x06`) — modules that don't fall into any other category. A new module application type is not usually allocated unless a host is likely to have more than one of a type installed. Audience metering modules are in this type.

Note: `Conditional_Access` (`0x01`) and `Electronic_Programme_Guide` (`0x02`) are inherited from EN 50221 §8.4.

### Unrecognized application type semantics (§5.1.2)

A host with a version-2 Application Information resource understands the full set of application types in Table 11. When presented with an unrecognized application type it shall treat it as **Unclassified (type `0x06`)**.
