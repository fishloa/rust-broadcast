# CA Pipeline Resource

_Source: ETSI TS 101 699 V1.1.1 §6.8, Tables 84–86 (PDF pp. 75–77), render-verified_

The CA-pipeline resource, with resource identifier **`0x00061ii1`** (resource type `1`, where `ii` = Module ID), is a module-provided resource that provides a framework referenced by application domains when implementing API- and CA-system-specific interfaces between receiver-hosted applications and CA systems (§6.8.1).

Each CA module supporting the CA-pipeline presents a CAP resource during the profile enquiry phase, with a resource ID modified by the module ID so that multiple modules can be discriminated (§6.8.2; "Extending use of the resource ID type field", §4.1). For example one module presents `0x00061041`, another `0x00061081`.

A set of three messages is defined (§6.8.3, "Message transfer"). Two messages provide a transfer protocol that allows sets of bytes to be transferred between host and module; a third (module-to-host) lets modules send an asynchronous event to an application:

- **CAPipelineRequest** — apdu_tag `0x9F8000` (host application ---> module)
- **CAPipelineResponse** — apdu_tag `0x9F8001` (module ---> application)
- **CAPipelineNotification** — apdu_tag `0x9F8002` (module ---> application, asynchronous)

The encoding of `CASpecificData` may be evident from the CA_System_ID identifying the CA system, or negotiated privately within the messages — a subject for the application-domain specification that invokes this interface.

---

## §6.8.3 — Message Transfer

### CAPipelineRequest object (Table 84, p. 76)

apdu_tag `CAPRequestTag` = `0x9F 80 00`, Direction host ---> app (host application to module).

The CA pipeline request sends a message from the host application to the module.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CAPRequest() {` | | |
| &nbsp;&nbsp;CAPRequestTag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;`for (i=0; i<length; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;CASpecificData | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `CAPRequestTag` — 24-bit integer with value `0x9F8000` identifying this message.
- `CASpecificData` — bytes carrying a CA-system-specific function invocation encoded in an API-specific way. Opaque variable-length byte blob.

### CAPipelineResponse object (Table 85, p. 76)

apdu_tag `CAP_response_tag` = `0x9F 80 01`, Direction app <--- host (module to application).

The CA pipeline reply sends a message from the module to the application.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CAPResponse() {` | | |
| &nbsp;&nbsp;CAP_response_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;`for (i=0; i<length; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;CA_specific_Data | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `CAP_response_tag` — 24-bit integer with value `0x9F8001` identifying this message. (The spec's field-note heading under Table 85 mislabels this "CAPRequestTag", but the syntax-table field name is `CAP_response_tag` and the stated value is `0x9F8001`.)
- `CA_specific_Data` — bytes carrying the result of the CA-system-specific function, encoded in an API-specific way. Opaque variable-length byte blob.

### CAPipelineNotification object (Table 86, p. 77)

apdu_tag `CAPNotificationTag` = `0x9F 80 02`, Direction app <--- host (module to application, asynchronous).

The CA pipeline notification sends an asynchronous message from the module to the application, typically used to create an event in an API-specific way.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CAPNotification() {` | | |
| &nbsp;&nbsp;CAPNotificationTag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;`for (i=0; i<length; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;CASpecificData | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `CAPNotificationTag` — 24-bit integer with value `0x9F8002` identifying this message.
- `CASpecificData` — bytes carrying optional event data encoded in an API-specific way. Opaque variable-length byte blob.

---

## §6.8.4 — Alternative implementations

This proposal does not preclude similar application-to-module communications using a private resource. The use of a private resource ID as an addressing mechanism allows the interface to operate to any type of CI module, not just CA modules.
