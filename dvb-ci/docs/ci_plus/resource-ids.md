# DVB CI Extensions — resource identifiers & application object tags

_Source: ETSI TS 101 699 V1.1.1 §8, Table 87 "Common interface resources" (PDF pp. 79-80) + §8.1 (PDF p. 81), render-verified_

This is the master registry of the DVB CI-extension resources, their 32-bit
`resource_identifier` values (per the EN 50221 §8.2.2 / Table 15 coding —
`resource_id_type` (2) + `resource_class` (14) + `resource_type` (10) +
`resource_version` (6)), and the APDU `apdu_tag` (3-octet "Tag Value") of every
application object they carry, with transfer direction.

Note: ETSI TS 103 605 (CI Plus over USB) does NOT re-publish these APDU layouts;
its §6.3 only references the proprietary CI Plus specification [3] for the
content-control / SAC / operator-profile / CAM-upgrade resources (see
`fragment-header.md` and the per-resource notes below). The resources tabulated
here are the ones whose syntax TS 101 699 itself prints.

## Resource ID coding recap (EN 50221 §8.2.2, Table 15)

For a public resource (`resource_id_type` = 0):

| Field | Bits |
|-------|------|
| resource_id_type | 2 |
| resource_class | 14 |
| resource_type | 10 |
| resource_version | 6 |

The 4 columns `class` / `type` / `version` in Table 87 below pack with
`resource_id_type = 0` into the 32-bit identifier. E.g. ResourceManager
class=1, type=1, version=2 → `0x00010042`.

## §8.1 — Resource type = 1* (PDF p. 81)

Where the `type` field of the resource ID is shown as `1*` in Table 87, the
10-bit type field is `0001 iiiiii`: the most-significant nibble indicates
"type = 1" and the lower 6 bits specify a **Module ID** (`ii` in the binary
column). See §4.1 "Extending use of the resource ID type field". So the printed
identifier values for `1*` resources carry the Module ID in those 6 bits
(shown as `ii` in the hex/binary breakdown, e.g. `0x00801ii1`).

## Table 87 — Common interface resources (PDF pp. 79-80)

### ResourceManager — class 1, type 1, version 2 — `0x00010042`

binary: `0000 0000 0000 0001 0000 0000 0100 0010`

| APDU name | Tag Value | To Resource | From Resource |
|-----------|-----------|:-----------:|:-------------:|
| Profile Enquiry   | `0x9F8010` | ✓ | ✓ |
| Profile Reply     | `0x9F8011` | ✓ | ✓ |
| Profile Changed   | `0x9F8012` | ✓ | ✓ |
| Module ID Send    | `0x9F8013` | ✓ |   |
| Module ID Command | `0x9F8014` |   | ✓ |

### ApplicationInformation — class 2, type 1, version 2 — `0x00020042`

binary: `0000 0000 0000 0010 0000 0000 0100 0010`

| APDU name | Tag Value | To Resource | From Resource |
|-----------|-----------|:-----------:|:-------------:|
| Application Info Enquiry | `0x9F8020` | ✓ |   |
| Application Info Info    | `0x9F8021` |   | ✓ |
| Enter Menu               | `0x9F8022` | ✓ |   |

### StreamInput — class 128, type 1*, version 1 — `0x00801ii1`

binary: `0000 0000 1000 0000 0001 iiii ii00 0001` (`ii` = Module ID)

| APDU name | Tag Value | To Resource | From Resource |
|-----------|-----------|:-----------:|:-------------:|
| DeliverySystemInfoReq | `0x9F8000` | ✓ |   |
| DeliverySystemInfoAck | `0x9F8001` |   | ✓ |
| ScanStartReq          | `0x9F8002` | ✓ |   |
| ScanNextReq           | `0x9F8003` | ✓ |   |
| ScanAck               | `0x9F8004` |   | ✓ |
| TuneTSReq             | `0x9F8005` | ✓ |   |
| TuneTSAck             | `0x9F8006` |   | ✓ |

### ServiceGateway (Generic Service Gateway) — see NOTE

The generic service gateway is the basis for other service-gateway resources;
it never exists on its own. (No class/type/version printed in Table 87 — it is
the base for BroadcastServiceGateway.)

| APDU name | Tag Value | To Resource | From Resource |
|-----------|-----------|:-----------:|:-------------:|
| ServiceListReq        | `0x9F8000` | ✓ |   |
| ServiceListAck        | `0x9F8001` |   | ✓ |
| ServiceListVersionReq | `0x9F8002` | ✓ |   |
| ServiceListVersionAck | `0x9F8003` |   | ✓ |
| ServiceListChanged    | `0x9F8004` |   | ✓ |
| ServiceDescReq        | `0x9F8005` | ✓ |   |
| ServiceDescAck        | `0x9F8006` |   | ✓ |
| GetServiceReq         | `0x9F8007` | ✓ |   |
| GetServiceAck         | `0x9F8008` |   | ✓ |

### BroadcastServiceGateway — class 129, type 1*, version 1 — `0x00811ii1`

binary: `0000 0000 1000 0001 0001 iiii ii00 0001` (`ii` = Module ID)

| APDU name | Tag Value | To Resource | From Resource |
|-----------|-----------|:-----------:|:-------------:|
| EITSectionReq | `0x9F8010` | ✓ |   |
| EITSectionAck | `0x9F8011` |   | ✓ |

### Status Query — class 33, type 1*, version 1 — `0x00211ii1`

binary: `0000 0000 0010 0001 0001 iiii ii00 0001` (`ii` = Module ID)

| APDU name | Tag Value | To Resource | From Resource |
|-----------|-----------|:-----------:|:-------------:|
| StatusQuery     | `0x9F8000` | ✓ |   |
| Trap            | `0x9F8001` | ✓ |   |
| GetNextItemReq  | `0x9F8002` | ✓ |   |
| GetNextItemAck  | `0x9F8003` |   | ✓ |
| StatusAck       | `0x9F8004` |   | ✓ |

### Power manager — class 34, type 1, version 1 — `0x00220041`

binary: `0000 0000 0010 0010 0000 0000 0100 0001`

| APDU name | Tag Value | To Resource | From Resource |
|-----------|-----------|:-----------:|:-------------:|
| Activation state change request     | `0x9F8000` | ✓ |   |
| Activation state change acknowledge | `0x9F8001` |   | ✓ |

### Event Manager — class 35, type 1*, version 1 — `0x00231ii1`

binary: `0000 0000 0010 0011 0001 iiii ii00 0001` (`ii` = Module ID)

| APDU name | Tag Value | To Resource | From Resource |
|-----------|-----------|:-----------:|:-------------:|
| Event request             | `0x9F8000` | ✓ |   |
| Event request acknowledge | `0x9F8001` |   | ✓ |
| Event notification        | `0x9F8002` |   | ✓ |

### Application MMI — class 65, type 1, version 1 — `0x00410041`

binary: `0000 0000 0100 0001 0000 0000 0100 0001`

| APDU name | Tag Value | To Resource | From Resource |
|-----------|-----------|:-----------:|:-------------:|
| RequestStart    | `0x9F8000` | ✓ |   |
| RequestStartAck | `0x9F8001` |   | ✓ |
| FileRequest     | `0x9F8002` |   | ✓ |
| FileAcknowledge | `0x9F8003` | ✓ |   |
| AppAbortRequest | `0x9F8004` | ✓ | ✓ |
| AppAbortAck     | `0x9F8005` | ✓ | ✓ |

### Copy protection — class 4, type 1*, version 1 — `0x00041ii1`

binary: `0000 0000 0000 0100 0001 iiii ii00 0001` (`ii` = Module ID)

| APDU name | Tag Value | To Resource | From Resource |
|-----------|-----------|:-----------:|:-------------:|
| CP_query    | `0x9F8000` | ✓ |   |
| CP_reply    | `0x9F8001` |   | ✓ |
| CP_command  | `0x9F8002` | ✓ |   |
| CP_response | `0x9F8003` |   | ✓ |

### Download resource — class 5, type 1, version 1 — `0x00051041`

binary: `0000 0000 0000 0101 0001 0000 0100 0001`

| APDU name | Tag Value | To Resource | From Resource |
|-----------|-----------|:-----------:|:-------------:|
| Download Enquiry         | `0x9F8000` | ✓ |   |
| Download Reply           | `0x9F8001` |   | ✓ |
| User Authorization Initiate | `0x9F8002` | ✓ |   |
| User Authorization Result   | `0x9F8003` |   | ✓ |

(binary printed in Table 87: `0000 0000 0000 0101 0001 0000 0100 0001`. The
hex literally printed in Table 87 reads `0x000510041` — 9 hex digits, an evident
spec typo. The binary is authoritative and packs to the 32-bit value
`0x00051041` (class 5 / type 1 / version 1). Use `0x00051041`.)

### CA pipeline resource — class 6, type 1, version 1 — `0x00061ii1`

binary: `0000 0000 0000 0110 0001 iiii ii00 0001` (`ii` = Module ID)

| APDU name | Tag Value | To Resource | From Resource |
|-----------|-----------|:-----------:|:-------------:|
| CAPipelineRequest      | `0x9F8000` | ✓ |   |
| CAPipelineResponse     | `0x9F8001` |   | ✓ |
| CAPipelineNotification | `0x9F8002` |   | ✓ |

NOTE (from Table 87): The generic service gateway is the basis for other
service gateway resources. It never exists on its own. In this release the only
resource based on the generic service gateway is the Broadcast service gateway.

Download resource identifier — RESOLVED (render-verified p. 80). Table 87
prints the class/type/version as 5 / 1 / 1 with binary
`0000 0000 0000 0101 0001 0000 0100 0001` and hex `0x000510041`. The hex as
literally printed in the PDF has 9 hex digits (an extra `0`) — an evident spec
typo. The binary is authoritative: it packs to the 32-bit value `0x00051041`
(type nibble `0001`, then `0000 00` = module-less type-1). Encode the constant
as `0x00051041`.
