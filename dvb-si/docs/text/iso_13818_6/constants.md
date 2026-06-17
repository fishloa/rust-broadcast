## Constants

### Tagged-profile / component tags (32-bit, `profileId_tag` / `componentId_tag`)
_TR 101 202 §4.7.3, Tables 4.5/4.7_

| Constant | Value | Meaning |
|---|---|---|
| `TAG_BIOP` | `0x49534F06` | BIOP Profile Body |
| `TAG_LITE_OPTIONS` | `0x49534F05` | Lite Options Profile Body |
| `TAG_ObjectLocation` | `0x49534F50` | BIOP::ObjectLocation component |
| `TAG_ConnBinder` | `0x49534F40` | DSM::ConnBinder component |
| `TAG_ServiceLocation` | `0x49534F46` | DSM::ServiceLocation component |

### U-U object type_id aliases (`objectKind` / IOR `type_id`)
_TR 101 202 §4.7.3.1, Table 4.4 — DVB uses ONLY the 3-char alias (+ NUL = 4 bytes)_

| Full type_id | alias | `objectKind_data` (4 bytes) |
|---|---|---|
| `DSM::Directory` | `"dir"` | `0x64697200` |
| `DSM::File` | `"fil"` | `0x66696C00` |
| `DSM::Stream` | `"str"` | `0x73747200` |
| `DSM::ServiceGateway` | `"srg"` | `0x73726700` |
| `BIOP::StreamEvent` | `"ste"` | `0x73746500` |

### Tap `use` values
_TR 101 202 §4.7.3.2, Table 4.6_

| Constant | Value | Broadcast on PID |
|---|---|---|
| `BIOP_DELIVERY_PARA_USE` | `0x0016` | Module delivery parameters |
| `BIOP_OBJECT_USE` | `0x0017` | BIOP objects in Modules |

### Binding type (`bindingType`, 8-bit)
_TR 101 202 §4.7.4.1, Table 4.9_

| Value | Meaning |
|---|---|
| `0x01` | `nobject` — name bound to a non-Directory/ServiceGateway object |
| `0x02` | `ncontext` — name bound to a Directory or ServiceGateway |

(`composite` is not supported for U-U object carousels.)

### Message header constants (all four message types)

| Field | Value |
|---|---|
| `magic` (4 bytes) | `0x42494F50` (`"BIOP"`) |
| `biop_version.major` | `0x01` |
| `biop_version.minor` | `0x00` |
| `byte_order` | `0x00` (big-endian — DVB mandatory) |
| `message_type` | `0x00` |

---

